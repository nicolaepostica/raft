//! Raft desktop app: wires the SQLite backend and file access into the egui UI.

mod native;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use raft_engine::Session;
use raft_ui::RaftUi;

/// Command-line options.
#[derive(Debug)]
pub struct Options {
    /// Folder with the books, `catalog.db` and `covers/`.
    pub library: PathBuf,
    /// The app's own database (reading progress).
    pub data: PathBuf,
    /// Port of the local control channel, if enabled.
    pub control: Option<u16>,
}

fn parse_args() -> Result<Options> {
    let mut library = std::env::var_os("RAFT_LIBRARY").map_or_else(|| PathBuf::from("/backup/books"), PathBuf::from);
    let mut data = default_data_path();
    let mut control = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--library" => library = args.next().context("--library needs a folder")?.into(),
            "--data" => data = args.next().context("--data needs a file")?.into(),
            "--control" => {
                let port = args.next().context("--control needs a port")?;
                control = Some(port.parse().context("--control port must be a number")?);
            }
            "-h" | "--help" => {
                println!(
                    "raft [--library DIR] [--data FILE] [--control PORT]\n\n\
                     --library DIR    folder with catalog.db and covers/ (default /backup/books, or $RAFT_LIBRARY)\n\
                     --data FILE      the app's database (default ~/.local/share/raft/raft.db)\n\
                     --control PORT   enable the local control channel on 127.0.0.1:PORT"
                );
                std::process::exit(0);
            }
            other => bail!("unknown argument `{other}` (see --help)"),
        }
    }
    Ok(Options { library, data, control })
}

fn default_data_path() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("raft").join("raft.db")
}

struct App {
    ui: RaftUi,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Safety net: a bug in drawing must not take the user's state down with it.
        let result = catch_unwind(AssertUnwindSafe(|| self.ui.show(ui)));
        if let Err(payload) = result {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".to_owned());
            self.ui.session.status = Some(format!("internal error while drawing: {msg}"));
        }
    }
}

fn main() -> Result<()> {
    let opts = parse_args()?;
    let backend = native::NativeBackend::open(&opts.library, &opts.data)?;
    let catalog = backend.load_catalog()?;
    let progress = backend.load_progress()?;
    let files = native::NativeFiles::new(&opts.library);
    let session = Session::new(catalog, progress, Box::new(backend));
    let ui = RaftUi::new(session, Box::new(files));

    let viewport = egui::ViewportBuilder::default()
        .with_title("Raft")
        .with_app_id("raft")
        .with_inner_size([1280.0, 860.0])
        .with_min_inner_size([640.0, 420.0]);
    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Raft",
        native_options,
        Box::new(move |_cc| Ok(Box::new(App { ui }))),
    )
    .map_err(|e| anyhow::anyhow!("window error: {e}"))
}
