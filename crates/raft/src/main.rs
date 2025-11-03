//! Raft desktop app: wires the SQLite backend and file access into the egui UI.

mod control;
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
    let mut library = std::env::var_os("RAFT_LIBRARY")
        .map_or_else(|| PathBuf::from("/backup/books"), PathBuf::from);
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
    Ok(Options {
        library,
        data,
        control,
    })
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
    control: Option<std::sync::mpsc::Receiver<control::Pending>>,
}

impl App {
    /// Answers control-channel requests on the UI thread.
    fn serve_control(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.control else { return };
        let pending: Vec<control::Pending> = rx.try_iter().collect();
        for p in pending {
            use raft_engine::control::{Request, commands_json, error, ok};
            let reply = match p.request {
                Request::Run { command, params } => match self.ui.run(&command, params) {
                    Ok(v) => ok(v),
                    Err(e) => error(e),
                },
                Request::State => ok(self.ui.session.snapshot()),
                Request::Commands => ok(commands_json()),
                Request::Screenshot { path } => {
                    let reply = p.reply.clone();
                    let cb = egui::ScreenshotCallback::new(move |img| {
                        let _ = reply.send(match save_png(&img, &path) {
                            Ok(()) => ok(serde_json::json!({ "path": path })),
                            Err(e) => error(e),
                        });
                    });
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(cb));
                    ctx.request_repaint();
                    continue;
                }
            };
            let _ = p.reply.send(reply);
        }
    }
}

fn save_png(img: &egui::ColorImage, path: &str) -> Result<()> {
    let [w, h] = img.size;
    let mut bytes = Vec::with_capacity(w * h * 4);
    for px in &img.pixels {
        bytes.extend_from_slice(&px.to_array());
    }
    let w = u32::try_from(w).context("image too wide")?;
    let h = u32::try_from(h).context("image too tall")?;
    let buf = image::RgbaImage::from_raw(w, h, bytes).context("bad screenshot buffer")?;
    buf.save_with_format(path, image::ImageFormat::Png)
        .with_context(|| format!("writing {path}"))
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.serve_control(&ctx);
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

/// Renders on the integrated GPU by default.
///
/// eframe asks wgpu for the high-performance adapter. On hybrid laptops that is the
/// discrete GPU, while the compositor shows the window from the integrated one; with
/// nouveau this produced blocky corrupted text and images. `WGPU_POWER_PREF=high`
/// still selects the discrete GPU.
fn wgpu_options() -> eframe::WgpuConfiguration {
    use eframe::egui_wgpu::WgpuSetup;
    use eframe::wgpu::PowerPreference;
    let mut config = eframe::WgpuConfiguration::default();
    if let WgpuSetup::CreateNew(create) = &mut config.wgpu_setup {
        create.power_preference = PowerPreference::from_env().unwrap_or(PowerPreference::LowPower);
    }
    config
}

/// The window icon (assets/icon.png, drawn by assets/icon.py).
fn app_icon() -> egui::IconData {
    image::load_from_memory_with_format(
        include_bytes!("../../../assets/icon.png"),
        image::ImageFormat::Png,
    )
    .map(|img| {
        let img = img.to_rgba8();
        egui::IconData {
            width: img.width(),
            height: img.height(),
            rgba: img.into_raw(),
        }
    })
    .unwrap_or_default()
}

fn main() -> Result<()> {
    let opts = parse_args()?;
    let backend = native::NativeBackend::open(&opts.library, &opts.data)?;
    let mut catalog = backend.load_catalog()?;
    let removed = backend.load_removed()?;
    catalog.books.retain(|b| !removed.contains(&b.id));
    let progress = backend.load_progress()?;
    let overrides = backend.load_overrides()?;
    // The saved language, else Romanian (the language the app was written in).
    let language = backend
        .load_language()?
        .unwrap_or_else(|| "ro".to_owned());
    // RAFT_THEME=light|dark wins over the saved choice for this run.
    let dark = match std::env::var("RAFT_THEME").as_deref() {
        Ok("dark") => true,
        Ok("light") => false,
        _ => backend.load_dark()?,
    };
    let files = native::NativeFiles::new(&opts.library, &opts.data);
    let session = Session::new(catalog, progress, Box::new(backend))
        .with_overrides(overrides)
        .with_dark(dark)
        .with_language(&language);
    let ui = RaftUi::new(session, Box::new(files));

    let viewport = egui::ViewportBuilder::default()
        .with_title("Raft")
        .with_app_id("raft")
        .with_icon(app_icon())
        .with_inner_size([1280.0, 860.0])
        .with_min_inner_size([640.0, 420.0]);
    let native_options = eframe::NativeOptions {
        viewport,
        wgpu_options: wgpu_options(),
        ..Default::default()
    };
    eframe::run_native(
        "Raft",
        native_options,
        Box::new(move |cc| {
            if let Some(rs) = &cc.wgpu_render_state {
                let info = rs.adapter.get_info();
                eprintln!(
                    "raft: rendering on {} ({:?}, {})",
                    info.name, info.backend, info.driver
                );
            }
            let control = match opts.control {
                Some(port) => Some(control::start(port, cc.egui_ctx.clone())?),
                None => None,
            };
            Ok(Box::new(App { ui, control }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("window error: {e}"))
}
