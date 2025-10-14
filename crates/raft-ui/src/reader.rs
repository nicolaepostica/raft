//! Paged reader (PDF, DjVu): one rendered page at a time, fit to width or to the window.

use egui::TextureHandle;
use serde_json::json;

use crate::{RaftUi, decode_image};

/// Page textures kept for quick back-and-forth.
const KEEP: usize = 6;
/// Rendered widths are rounded to this step so resizing does not re-render each pixel.
const WIDTH_STEP: f32 = 200.0;

type Key = (String, u32, u32);

#[derive(Default)]
pub struct ReaderView {
    textures: Vec<(Key, Result<TextureHandle, String>)>,
    /// Fit the whole page in the window instead of fitting the width.
    pub fit_page: bool,
    /// Height / width of the last page shown.
    aspect: Option<f32>,
}

impl ReaderView {
    fn get(&self, key: &Key) -> Option<&Result<TextureHandle, String>> {
        self.textures.iter().find(|(k, _)| k == key).map(|(_, t)| t)
    }

    fn put(&mut self, key: Key, value: Result<TextureHandle, String>) {
        self.textures.retain(|(k, _)| *k != key);
        self.textures.push((key, value));
        if self.textures.len() > KEEP {
            drop(self.textures.remove(0));
        }
    }
}

pub fn reader(app: &mut RaftUi, ui: &mut egui::Ui, id: &str) {
    let Some(book) = app.session.book(id).cloned() else {
        ui.label("Cartea nu mai există în catalog.");
        return;
    };
    let page = app.session.progress(id).and_then(|p| p.page).unwrap_or(1);
    let pages = book.pages.unwrap_or(0);

    ui.horizontal(|ui| {
        crate::back_button(app, ui, "Înapoi la pagina cărții");
        ui.separator();
        if ui
            .add_enabled(page > 1, egui::Button::new("Anterioara"))
            .on_hover_text("Săgeată stânga")
            .clicked()
        {
            app.act("reader.prev", json!({}));
        }
        let label = ui.label("Pagina");
        let mut edit = page;
        let resp = ui
            .add(egui::DragValue::new(&mut edit).range(1..=pages.max(1)))
            .labelled_by(label.id);
        if resp.changed() && edit != page {
            app.act("reader.goto", json!({ "page": edit }));
        }
        ui.label(format!("din {pages}"));
        if ui
            .add_enabled(page < pages, egui::Button::new("Următoarea"))
            .on_hover_text("Săgeată dreapta")
            .clicked()
        {
            app.act("reader.next", json!({}));
        }
        ui.separator();
        ui.checkbox(&mut app.reader.fit_page, "Toată pagina");
        ui.separator();
        ui.add(egui::Label::new(egui::RichText::new(book.display_title()).strong()).truncate());
    });
    ui.separator();

    let avail = ui.available_size();
    let ppp = ui.ctx().pixels_per_point();
    let mut width_pts = avail.x - 16.0;
    if app.reader.fit_page {
        let aspect = app.reader.aspect.unwrap_or(1.414);
        width_pts = width_pts.min((avail.y - 8.0) / aspect);
    }
    let width_px = ((width_pts * ppp / WIDTH_STEP).ceil() * WIDTH_STEP).clamp(200.0, 4000.0) as u32;

    let key: Key = (book.id.clone(), page, width_px);
    if app.reader.get(&key).is_none() {
        match app.files.page(&book, page, width_px) {
            Some(Ok(bytes)) => {
                let tex = decode_image(&bytes).map(|img| {
                    ui.ctx().load_texture(
                        format!("page-{}-{page}-{width_px}", book.id),
                        img,
                        egui::TextureOptions::LINEAR,
                    )
                });
                app.reader.put(key.clone(), tex);
            }
            Some(Err(e)) => app.reader.put(key.clone(), Err(e)),
            None => ui
                .ctx()
                .request_repaint_after(std::time::Duration::from_millis(50)),
        }
    }
    // Prefetch the next page.
    if page < pages {
        let _ = app.files.page(&book, page + 1, width_px);
    }

    match app.reader.get(&key).cloned() {
        None => {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.spinner();
                ui.label(format!("Se randează pagina {page}…"));
            });
        }
        Some(Err(e)) => {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);
                ui.label(format!("Pagina nu a putut fi afișată: {e}"));
                if ui.button("Deschide în aplicația externă").clicked() {
                    app.act("book.open_external", json!({ "id": book.id }));
                }
            });
        }
        Some(Ok(tex)) => {
            let size = tex.size_vec2();
            if size.x > 0.0 {
                app.reader.aspect = Some(size.y / size.x);
            }
            let scale = width_pts / size.x.max(1.0);
            egui::ScrollArea::both()
                .id_salt(("page", page))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        let resp = ui.add(
                            egui::Image::new((tex.id(), size * scale)).sense(egui::Sense::click()),
                        );
                        if resp.clicked()
                            && let Some(pos) = resp.interact_pointer_pos()
                        {
                            let cmd = if pos.x > resp.rect.center().x {
                                "reader.next"
                            } else {
                                "reader.prev"
                            };
                            app.act(cmd, json!({}));
                        }
                    });
                });
        }
    }
}
