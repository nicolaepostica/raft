//! Catalog builds: `build-catalog DIR --no-ai` run in the background for a
//! library folder, its progress read from the script's output.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use raft_engine::{IndexProgress, IndexState};

/// Longest output line kept, in bytes (file names come from the folder).
const MAX_LINE: u64 = 4096;
/// Error lines kept for the message of a failed build.
const MAX_ERROR_LINES: usize = 20;

/// The running and just finished builds, by folder.
#[derive(Default)]
pub struct Jobs {
    script: Option<PathBuf>,
    jobs: Mutex<HashMap<String, Arc<Mutex<IndexState>>>>,
}

impl Jobs {
    /// Builds run `script` (none found: every build fails with a hint).
    pub fn new(script: Option<PathBuf>) -> Self {
        Self {
            script,
            jobs: Mutex::default(),
        }
    }

    /// Starts a build of `root` unless one is running.
    pub fn start(&self, root: &str) -> Result<(), String> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        if jobs.get(root).is_some_and(|j| {
            matches!(
                *j.lock().unwrap_or_else(|e| e.into_inner()),
                IndexState::Running(_)
            )
        }) {
            return Ok(());
        }
        let Some(script) = &self.script else {
            return Err("build-catalog not found: link scripts/build-catalog and \
                 scripts/rename-books into ~/.local/bin"
                .into());
        };
        let mut child = Command::new(script)
            .arg(root)
            .arg("--no-ai")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("{}: {e}", script.display()))?;
        let state = Arc::new(Mutex::new(IndexState::Running(IndexProgress::default())));
        jobs.insert(root.to_owned(), state.clone());
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        std::thread::spawn(move || {
            let errors = std::thread::spawn(move || stderr.map(last_lines).unwrap_or_default());
            if let Some(out) = stdout {
                for_lines(out, |line| {
                    if let Some(p) = parse_progress(line) {
                        *state.lock().unwrap_or_else(|e| e.into_inner()) = IndexState::Running(p);
                    }
                });
            }
            let errors = errors.join().unwrap_or_default();
            let end = match child.wait() {
                Ok(status) if status.success() => IndexState::Done,
                Ok(status) => IndexState::Failed(
                    errors
                        .last()
                        .cloned()
                        .unwrap_or_else(|| format!("build-catalog ended with {status}")),
                ),
                Err(e) => IndexState::Failed(e.to_string()),
            };
            *state.lock().unwrap_or_else(|e| e.into_inner()) = end;
        });
        Ok(())
    }

    /// The build's state; a finished one is reported once and forgotten.
    pub fn state(&self, root: &str) -> Option<IndexState> {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        let state = jobs
            .get(root)?
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if !matches!(state, IndexState::Running(_)) {
            jobs.remove(root);
        }
        Some(state)
    }
}

/// `build-catalog`: next to the raft binary (the release archive), else from
/// `PATH`, else `~/.local/bin`.
pub fn find_script() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf));
    let path: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    let home = std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/bin"));
    script_in(exe_dir.into_iter().chain(path).chain(home))
}

/// The first `build-catalog` file in `dirs`.
fn script_in(dirs: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    dirs.into_iter()
        .map(|d| d.join("build-catalog"))
        .find(|p| p.is_file())
}

/// Progress from a line of build-catalog: `N books, M to process` (start) or
/// `[done/total] file …` (one book).
pub fn parse_progress(line: &str) -> Option<IndexProgress> {
    if let Some(rest) = line.strip_prefix('[') {
        let (counts, _) = rest.split_once(']')?;
        let (done, total) = counts.split_once('/')?;
        return Some(IndexProgress {
            done: done.trim().parse().ok()?,
            total: total.trim().parse().ok()?,
        });
    }
    let (_, todo) = line.split_once(" books, ")?;
    let total = todo.strip_suffix(" to process")?.trim().parse().ok()?;
    Some(IndexProgress { done: 0, total })
}

/// Calls `f` with every line of `r`, each cut to `MAX_LINE` bytes.
fn for_lines(r: impl Read, mut f: impl FnMut(&str)) {
    let mut r = BufReader::new(r);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match r.by_ref().take(MAX_LINE).read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if buf.last() != Some(&b'\n') {
            // Too long: skip to the end of the line.
            let mut rest = Vec::new();
            loop {
                rest.clear();
                match r.by_ref().take(MAX_LINE).read_until(b'\n', &mut rest) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if rest.last() == Some(&b'\n') => break,
                    Ok(_) => {}
                }
            }
        }
        f(String::from_utf8_lossy(&buf).trim());
    }
}

/// The last non-empty lines of `r`.
fn last_lines(r: impl Read) -> Vec<String> {
    let mut lines = std::collections::VecDeque::new();
    for_lines(r, |l| {
        if !l.is_empty() {
            if lines.len() == MAX_ERROR_LINES {
                lines.pop_front();
            }
            lines.push_back(l.to_owned());
        }
    });
    lines.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn progress_lines_are_read() {
        assert_eq!(
            parse_progress("543 books, 12 to process"),
            Some(IndexProgress { done: 0, total: 12 })
        );
        assert_eq!(
            parse_progress("[3/12] Autor/Carte_2020.fb2  →  fiction  (no AI)"),
            Some(IndexProgress { done: 3, total: 12 })
        );
        assert_eq!(parse_progress("[x/12] a"), None);
        assert_eq!(parse_progress("Catalog: 5 books, 5 with a cover"), None);
        assert_eq!(parse_progress("ERROR in a.pdf: boom"), None);
    }

    #[test]
    fn long_lines_are_cut() {
        let long = format!("{}\nnext\n", "x".repeat(10_000));
        let mut seen = Vec::new();
        for_lines(long.as_bytes(), |l| seen.push(l.len()));
        assert_eq!(seen, [MAX_LINE as usize, 4]);
    }

    #[test]
    fn the_script_is_found_in_the_first_folder_that_has_it() {
        let tmp = std::env::temp_dir().join(format!("raft-script-{}", std::process::id()));
        let (a, b) = (tmp.join("a"), tmp.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        let found = script(&b, "true");
        assert_eq!(script_in([a.clone(), b.clone()]), Some(found));
        assert_eq!(script_in([a]), None);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn script(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join("build-catalog");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn wait(jobs: &Jobs, root: &str) -> IndexState {
        for _ in 0..500 {
            match jobs.state(root) {
                Some(IndexState::Running(_)) => {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                Some(s) => return s,
                None => break,
            }
        }
        IndexState::Failed("timed out".into())
    }

    #[test]
    fn builds_run_and_report() {
        let tmp = std::env::temp_dir().join(format!("raft-index-{}", std::process::id()));
        let ok = Jobs::new(Some(script(
            &tmp.join("ok"),
            r#"[ "$2" = "--no-ai" ] || exit 3; echo "2 books, 2 to process"; echo "[1/2] a"; echo "[2/2] b""#,
        )));
        ok.start("/lib").unwrap();
        assert_eq!(wait(&ok, "/lib"), IndexState::Done);
        assert_eq!(ok.state("/lib"), None, "a finished build is reported once");

        let bad = Jobs::new(Some(script(
            &tmp.join("bad"),
            "echo 'ModuleNotFoundError: No module named PIL' >&2; exit 1",
        )));
        bad.start("/lib").unwrap();
        assert_eq!(
            wait(&bad, "/lib"),
            IndexState::Failed("ModuleNotFoundError: No module named PIL".into())
        );
        assert!(Jobs::new(None).start("/lib").is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    proptest! {
        #[test]
        fn any_line_parses_without_panic(line in ".{0,200}") {
            let _ = parse_progress(&line);
        }
    }
}
