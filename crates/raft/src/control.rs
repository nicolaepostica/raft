//! Local control channel: a TCP server on 127.0.0.1 that forwards requests to the UI thread.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use raft_engine::control::{self, MAX_LINE, Request};
use serde_json::Value;

/// A request waiting for the UI thread, with where to send the reply.
pub struct Pending {
    pub request: Request,
    pub reply: Sender<Value>,
}

/// Starts the server thread; returns the queue the UI drains every frame.
pub fn start(port: u16, ctx: egui::Context) -> std::io::Result<Receiver<Pending>> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let (tx, rx) = channel();
    std::thread::Builder::new()
        .name("raft-control".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let tx = tx.clone();
                let ctx = ctx.clone();
                let _ = std::thread::Builder::new()
                    .name("raft-control-conn".into())
                    .spawn(move || serve(stream, &tx, &ctx));
            }
        })?;
    Ok(rx)
}

fn serve(stream: TcpStream, tx: &Sender<Pending>, ctx: &egui::Context) {
    let Ok(write_half) = stream.try_clone() else {
        return;
    };
    let mut out = write_half;
    // Cap each line so a client cannot make us buffer unbounded data.
    let mut reader = BufReader::new(stream.take(u64::MAX));
    loop {
        let mut line = Vec::new();
        let n = match reader.by_ref().take((MAX_LINE + 1) as u64).read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        let reply = if n > MAX_LINE {
            control::error("request is too long")
        } else {
            match control::parse_request(line.trim_ascii()) {
                Err(e) => control::error(e),
                Ok(request) => {
                    let (rtx, rrx) = channel();
                    if tx.send(Pending { request, reply: rtx }).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                    rrx.recv_timeout(Duration::from_secs(20))
                        .unwrap_or_else(|_| control::error("the UI did not answer in time"))
                }
            }
        };
        let mut text = reply.to_string();
        text.push('\n');
        if out.write_all(text.as_bytes()).is_err() {
            return;
        }
    }
}
