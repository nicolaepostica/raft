//! `raft mcp [--port PORT] [--no-start] [--data FILE]`: an MCP server on stdin/stdout (one JSON-RPC
//! message per line) that drives a running Raft through its control channel on
//! 127.0.0.1:PORT. When nothing answers there it starts `raft --control PORT` itself.
//! stdout carries only protocol messages; diagnostics go to stderr.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use raft_engine::control::MAX_LINE;
use raft_engine::mcp::{self, Bridge};
use serde_json::{Value, json};

/// The control port `raft mcp` uses unless told otherwise.
pub const DEFAULT_PORT: u16 = 7878;
/// Longest control reply read, in bytes (a `book` or `commands` reply is far smaller).
const MAX_REPLY: u64 = 16 * 1024 * 1024;
/// Largest screenshot passed on, in bytes.
const MAX_SCREENSHOT: u64 = 32 * 1024 * 1024;
/// How long a started app gets to open its control port.
const START_WAIT: Duration = Duration::from_secs(20);

struct Options {
    port: u16,
    start: bool,
    /// `--data` for the app it starts.
    data: Option<String>,
}

fn parse(args: impl Iterator<Item = String>) -> Result<Options> {
    let mut opts = Options {
        port: DEFAULT_PORT,
        start: true,
        data: None,
    };
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => {
                let p = args.next().context("--port needs a number")?;
                opts.port = p.parse().context("--port must be a number")?;
            }
            "--no-start" => opts.start = false,
            "--data" => opts.data = Some(args.next().context("--data needs a file")?),
            "-h" | "--help" => {
                eprintln!(
                    "raft mcp [--port PORT] [--no-start] [--data FILE]\n\n\
                     An MCP server on stdin/stdout for Raft's commands. It talks to Raft's control\n\
                     channel on 127.0.0.1:PORT (default {DEFAULT_PORT}) and starts\n\
                     `raft --control PORT [--data FILE]` when nothing answers there, unless --no-start.\n\n\
                     Claude Code: claude mcp add raft -- raft mcp"
                );
                std::process::exit(0);
            }
            other => bail!("unknown argument `{other}` (see raft mcp --help)"),
        }
    }
    Ok(opts)
}

/// Runs the server until stdin closes.
pub fn main(args: impl Iterator<Item = String>) -> Result<()> {
    let opts = parse(args)?;
    let mut bridge = Control {
        addr: SocketAddr::from(([127, 0, 0, 1], opts.port)),
        start: opts.start,
        data: opts.data,
        conn: None,
        shots: 0,
    };
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut out = std::io::stdout().lock();
    loop {
        let mut line = Vec::new();
        let n = input
            .by_ref()
            .take((MAX_LINE + 1) as u64)
            .read_until(b'\n', &mut line)?;
        if n == 0 {
            return Ok(());
        }
        if n > MAX_LINE && !line.ends_with(b"\n") {
            // Drop the rest of an over-long line before answering it.
            let mut rest = Vec::new();
            input.read_until(b'\n', &mut rest)?;
        }
        let line = line.trim_ascii();
        if line.is_empty() {
            continue;
        }
        if let Some(reply) = mcp::handle(line, &mut bridge, env!("CARGO_PKG_VERSION")) {
            let mut text = reply.to_string();
            text.push('\n');
            out.write_all(text.as_bytes())?;
            out.flush()?;
        }
    }
}

/// The connection to the app's control channel.
struct Control {
    addr: SocketAddr,
    start: bool,
    data: Option<String>,
    conn: Option<(TcpStream, BufReader<TcpStream>)>,
    shots: u32,
}

impl Control {
    fn connect(&self) -> std::io::Result<(TcpStream, BufReader<TcpStream>)> {
        let stream = TcpStream::connect_timeout(&self.addr, Duration::from_secs(2))?;
        // The app answers within 20 s or replies with an error itself.
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        let reader = BufReader::new(stream.try_clone()?);
        Ok((stream, reader))
    }

    /// Connects, starting the app first when nothing listens and that is allowed.
    fn ensure(&mut self) -> Result<(), String> {
        if self.conn.is_some() {
            return Ok(());
        }
        match self.connect() {
            Ok(c) => {
                self.conn = Some(c);
                return Ok(());
            }
            Err(e) if !self.start => {
                return Err(format!(
                    "Raft is not running with --control {} ({e}); start it with `raft --control {}`",
                    self.addr.port(),
                    self.addr.port()
                ));
            }
            Err(_) => {}
        }
        let exe = std::env::current_exe().map_err(|e| format!("cannot find raft: {e}"))?;
        eprintln!(
            "raft mcp: starting {} --control {}",
            exe.display(),
            self.addr.port()
        );
        let mut cmd = Command::new(&exe);
        cmd.args(["--control", &self.addr.port().to_string()]);
        if let Some(data) = &self.data {
            cmd.args(["--data", data]);
        }
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("cannot start raft: {e}"))?;
        let deadline = Instant::now() + START_WAIT;
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(250));
            if let Ok(c) = self.connect() {
                self.conn = Some(c);
                return Ok(());
            }
        }
        Err(format!(
            "started raft, but its control channel on port {} did not open",
            self.addr.port()
        ))
    }

    /// One request and its reply line; the connection is dropped on any I/O error.
    fn exchange(&mut self, request: &Value) -> Result<Value, String> {
        self.ensure()?;
        let Some((stream, reader)) = self.conn.as_mut() else {
            return Err("not connected".into());
        };
        let mut text = request.to_string();
        text.push('\n');
        let mut reply = Vec::new();
        let io = stream.write_all(text.as_bytes()).and_then(|()| {
            reader
                .by_ref()
                .take(MAX_REPLY)
                .read_until(b'\n', &mut reply)
        });
        match io {
            Ok(n) if n > 0 => {}
            Ok(_) => {
                self.conn = None;
                return Err("Raft closed the control channel (was the window closed?)".into());
            }
            Err(e) => {
                self.conn = None;
                return Err(format!("control channel: {e}"));
            }
        }
        let reply: Value =
            serde_json::from_slice(&reply).map_err(|e| format!("bad reply from Raft: {e}"))?;
        if reply.get("ok") == Some(&Value::Bool(true)) {
            Ok(reply.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(reply
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Raft answered with an error")
                .to_owned())
        }
    }
}

impl Bridge for Control {
    fn call(&mut self, request: &Value) -> Result<Value, String> {
        // A connection the app dropped (restarted) is retried once with a fresh one.
        match self.exchange(request) {
            Err(_) if self.conn.is_none() => self.exchange(request),
            r => r,
        }
    }

    fn screenshot(&mut self) -> Result<Vec<u8>, String> {
        self.shots += 1;
        let path: PathBuf = std::env::temp_dir().join(format!(
            "raft-mcp-{}-{}.png",
            std::process::id(),
            self.shots
        ));
        let path_text = path.to_string_lossy().into_owned();
        self.call(&json!({ "method": "ui.screenshot", "path": path_text }))?;
        let read = std::fs::File::open(&path).and_then(|f| {
            let mut png = Vec::new();
            f.take(MAX_SCREENSHOT + 1).read_to_end(&mut png)?;
            Ok(png)
        });
        let _ = std::fs::remove_file(&path);
        let png = read.map_err(|e| format!("reading the screenshot: {e}"))?;
        if png.len() as u64 > MAX_SCREENSHOT {
            return Err("the screenshot is too large".into());
        }
        Ok(png)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn args(a: &[&str]) -> impl Iterator<Item = String> {
        a.iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn options() {
        let o = parse(args(&[])).unwrap();
        assert_eq!((o.port, o.start), (DEFAULT_PORT, true));
        let o = parse(args(&["--port", "7999", "--no-start", "--data", "/x.db"])).unwrap();
        assert_eq!(
            (o.port, o.start, o.data.as_deref()),
            (7999, false, Some("/x.db"))
        );
        assert!(parse(args(&["--port", "x"])).is_err());
        assert!(parse(args(&["--what"])).is_err());
    }

    /// A fake app: answers each request line with `ok` and the request echoed.
    fn fake_app() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut out = stream.try_clone().unwrap();
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    let req: Value = serde_json::from_str(&line).unwrap();
                    let reply = if req["command"] == "bad" {
                        json!({"ok": false, "error": "unknown command `bad`"})
                    } else {
                        json!({"ok": true, "result": req})
                    };
                    writeln!(out, "{reply}").unwrap();
                }
            }
        });
        port
    }

    #[test]
    fn requests_go_to_the_control_channel() {
        let port = fake_app();
        let mut c = Control {
            addr: SocketAddr::from(([127, 0, 0, 1], port)),
            start: false,
            data: None,
            conn: None,
            shots: 0,
        };
        let req = json!({"method": "run", "command": "search.set", "params": {"text": "go"}});
        assert_eq!(c.call(&req), Ok(req.clone()));
        // Same connection for the next one.
        assert_eq!(
            c.call(&json!({"method": "state"})),
            Ok(json!({"method": "state"}))
        );
        assert_eq!(
            c.call(&json!({"method": "run", "command": "bad"})),
            Err("unknown command `bad`".into())
        );
    }

    #[test]
    fn no_app_and_no_start_is_a_clear_error() {
        // A port nothing listens on: bind, read it, close.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut c = Control {
            addr: SocketAddr::from(([127, 0, 0, 1], port)),
            start: false,
            data: None,
            conn: None,
            shots: 0,
        };
        let err = c.call(&json!({"method": "state"})).unwrap_err();
        assert!(err.contains("not running"), "{err}");
    }
}
