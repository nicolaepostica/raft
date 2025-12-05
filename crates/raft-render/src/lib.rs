//! Page rendering (layer 3, native only).
//!
//! PDF pages are rendered by `pdftoppm` (poppler-utils) as PNG, DjVu pages by
//! `ddjvu` (djvulibre) as PNM. Both tools are already required by the library scripts.
//! The tools run with a timeout and their output is capped.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use raft_doc::Format;

/// Narrowest and widest page image, in pixels.
pub const MIN_WIDTH: u32 = 100;
pub const MAX_WIDTH: u32 = 4000;
/// Largest accepted tool output, in bytes.
pub const MAX_OUTPUT: usize = 128 * 1024 * 1024;
/// Longest a render may take.
pub const TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RenderError {
    #[error("{0} pages cannot be rendered")]
    Unsupported(&'static str),
    #[error("page must be 1 or greater")]
    BadPage,
    #[error("could not start {tool}: {reason} (is it installed?)")]
    Spawn { tool: &'static str, reason: String },
    #[error("{0} took too long")]
    Timeout(&'static str),
    #[error("{tool} failed: {reason}")]
    Failed { tool: &'static str, reason: String },
    #[error("page image is too large")]
    TooLarge,
}

/// Image encoding of a rendered page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Png,
    Pnm,
}

/// The command line that renders `page` of `file` at `width` pixels.
pub fn command_for(
    file: &Path,
    format: Format,
    page: u32,
    width: u32,
) -> Result<(&'static str, Vec<String>, Encoding), RenderError> {
    if page == 0 {
        return Err(RenderError::BadPage);
    }
    let width = width.clamp(MIN_WIDTH, MAX_WIDTH);
    let file = file.to_string_lossy().into_owned();
    match format {
        Format::Pdf => Ok((
            "pdftoppm",
            vec![
                "-f".into(),
                page.to_string(),
                "-l".into(),
                page.to_string(),
                "-scale-to-x".into(),
                width.to_string(),
                "-scale-to-y".into(),
                "-1".into(),
                "-png".into(),
                "--".into(),
                file,
            ],
            Encoding::Png,
        )),
        Format::Djvu => Ok((
            "ddjvu",
            vec![
                "-format=ppm".into(),
                format!("-page={page}"),
                format!("-size={width}x{}", width * 4),
                file,
                "-".into(),
            ],
            Encoding::Pnm,
        )),
        other => Err(RenderError::Unsupported(other.slug())),
    }
}

/// Renders one page; returns the encoded image.
pub fn render_page(
    file: &Path,
    format: Format,
    page: u32,
    width: u32,
) -> Result<(Vec<u8>, Encoding), RenderError> {
    let (tool, args, encoding) = command_for(file, format, page, width)?;
    run_tool(tool, &args).map(|out| (out, encoding))
}

/// Runs `tool` with a time limit; returns its output (an error when it fails
/// or prints nothing).
fn run_tool(tool: &'static str, args: &[String]) -> Result<Vec<u8>, RenderError> {
    let mut child = Command::new(tool)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| RenderError::Spawn {
            tool,
            reason: e.to_string(),
        })?;
    let mut out = Vec::new();
    let reader = child.stdout.take().map(|s| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let r = s.take(MAX_OUTPUT as u64 + 1).read_to_end(&mut buf);
            r.map(|_| buf)
        })
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RenderError::Timeout(tool));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => {
                return Err(RenderError::Failed {
                    tool,
                    reason: e.to_string(),
                });
            }
        }
    };
    if let Some(handle) = reader {
        out = handle.join().ok().and_then(Result::ok).unwrap_or_default();
    }
    if out.len() > MAX_OUTPUT {
        return Err(RenderError::TooLarge);
    }
    if !status.success() || out.is_empty() {
        let mut err = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = e.by_ref().take(4096).read_to_string(&mut err);
        }
        let reason = err.lines().next().unwrap_or("no output").to_owned();
        return Err(RenderError::Failed { tool, reason });
    }
    Ok(out)
}

/// The number of pages of a PDF (`pdfinfo`) or DjVu (`djvused -e n`) file.
pub fn page_count(file: &Path, format: Format) -> Result<u32, RenderError> {
    let file = file.to_string_lossy().into_owned();
    let (tool, args) = match format {
        Format::Pdf => ("pdfinfo", vec!["--".to_owned(), file]),
        Format::Djvu => ("djvused", vec!["-e".to_owned(), "n".to_owned(), file]),
        other => return Err(RenderError::Unsupported(other.slug())),
    };
    let out = run_tool(tool, &args)?;
    parse_page_count(&String::from_utf8_lossy(&out)).ok_or(RenderError::Failed {
        tool,
        reason: "no page count in its output".into(),
    })
}

/// The page count from `pdfinfo` ("Pages:  123") or `djvused -e n` ("123") output.
pub fn parse_page_count(out: &str) -> Option<u32> {
    let value = out
        .lines()
        .find_map(|l| l.strip_prefix("Pages:"))
        .unwrap_or(out);
    value.trim().parse().ok().filter(|&n| n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_commands() {
        let (tool, args, enc) = command_for(Path::new("/b/x.pdf"), Format::Pdf, 3, 50_000).unwrap();
        assert_eq!(tool, "pdftoppm");
        assert!(args.contains(&"4000".to_owned()));
        assert_eq!(args.last().map(String::as_str), Some("/b/x.pdf"));
        assert_eq!(enc, Encoding::Png);
        let (tool, args, _) = command_for(Path::new("/b/y.djvu"), Format::Djvu, 2, 800).unwrap();
        assert_eq!(tool, "ddjvu");
        assert!(args.contains(&"-page=2".to_owned()));
        assert_eq!(
            command_for(Path::new("x"), Format::Pdf, 0, 800),
            Err(RenderError::BadPage)
        );
        assert!(matches!(
            command_for(Path::new("x"), Format::Epub, 1, 800),
            Err(RenderError::Unsupported("epub"))
        ));
    }

    #[test]
    fn page_counts_are_read() {
        assert_eq!(
            parse_page_count("Title: x\nPages:          412\nEncrypted: no\n"),
            Some(412)
        );
        assert_eq!(parse_page_count("37\n"), Some(37));
        assert_eq!(parse_page_count("Pages: 0"), None);
        assert_eq!(parse_page_count("Syntax Error: bad file"), None);
        let p = Path::new("/backup/books/100_Go_Mistakes_and_How_to_Avoid_Them_2022.pdf");
        if p.exists() {
            assert!(page_count(p, Format::Pdf).unwrap() > 100);
        }
        assert!(page_count(Path::new("/nonexistent/x.pdf"), Format::Pdf).is_err());
    }

    proptest::proptest! {
        #[test]
        fn any_output_parses_without_panic(out in ".{0,300}") {
            let _ = parse_page_count(&out);
        }
    }

    #[test]
    fn missing_file_is_an_error_not_a_panic() {
        let r = render_page(Path::new("/nonexistent/raft-test.pdf"), Format::Pdf, 1, 400);
        assert!(r.is_err());
    }

    #[test]
    fn renders_a_real_page_if_present() {
        let p = Path::new("/backup/books/100_Go_Mistakes_and_How_to_Avoid_Them_2022.pdf");
        if !p.exists() {
            return;
        }
        let (png, enc) = render_page(p, Format::Pdf, 2, 300).unwrap();
        assert_eq!(enc, Encoding::Png);
        assert_eq!(png.get(1..4), Some(&b"PNG"[..]));
        // Page past the end fails cleanly.
        assert!(render_page(p, Format::Pdf, 100_000, 300).is_err());
    }
}
