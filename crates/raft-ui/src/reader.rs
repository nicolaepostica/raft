//! Paged reader (PDF, DjVu): every page in one scrolling column, fit to the width
//! or to the window, times the reader zoom.
//!
//! Only the pages in view (and the next one) are rendered; the others are empty
//! slots of the same size, so the column has the whole book's height from the
//! start. The page at the reading point (a third down the view) is the current
//! page: it is saved with `reader.goto` as the view scrolls, and a page changed
//! elsewhere (buttons, ←/→, the page field, the control channel) scrolls the view
//! to it.

use egui::TextureHandle;
use serde_json::json;

use crate::i18n::{t, tf};
use crate::{RaftUi, decode_image};

/// Page textures kept, enough for a screen of small pages and a few around it.
const KEEP: usize = 16;
/// Widest page rendered, in pixels (larger zooms scale it up), to bound memory.
const MAX_WIDTH_PX: f32 = 3000.0;
/// Rendered widths are rounded to this step so resizing does not re-render each pixel.
const WIDTH_STEP: f32 = 200.0;
/// Space between pages, in points.
const GAP: f32 = 12.0;
/// Height / width assumed until a page of the book has been rendered (A4).
const DEFAULT_ASPECT: f32 = 1.414;
/// The reading point, as a fraction of the view's height from its top.
const READING_POINT: f32 = 0.3;
/// Points scrolled by ↑/↓.
const LINE: f32 = 80.0;

type Key = (String, u32, u32);

/// Where to scroll after the page size changed.
#[derive(Clone, Copy)]
enum Relayout {
    /// The top of this page (the view was sitting on it).
    Page(u32),
    /// This place, in pages from the top of the book, stays at the reading point.
    Place(f32),
}

#[derive(Default)]
pub struct ReaderView {
    textures: Vec<(Key, Result<TextureHandle, String>)>,
    /// Fit the whole page in the window instead of fitting the width.
    pub fit_page: bool,
    /// Height / width of the book's pages (of the first one rendered).
    aspect: Option<(String, f32)>,
    /// The page the view is at, for this book; differs from the saved page when
    /// the page was changed elsewhere and the view must scroll to it.
    shown: Option<(String, u32)>,
    /// The zoom of the last frame (a new zoom centres the page horizontally).
    zoom: f32,
    /// Work left for the next frame, once the scroll area knows the new size
    /// (an offset past the old size would spring back).
    relayout: Option<Relayout>,
    center: bool,
    /// Scroll offset and view size of the last frame.
    offset: f32,
    view_height: f32,
    view_width: f32,
    /// Distance between page tops in the last frame (changes with the width).
    stride: f32,
    /// Offset set by the last jump: the page is not read back from the scroll
    /// until the user scrolls away from it (the end of the book cannot be
    /// scrolled to the top of the view).
    pinned: Option<f32>,
}

impl ReaderView {
    fn get(&self, key: &Key) -> Option<&Result<TextureHandle, String>> {
        self.textures.iter().find(|(k, _)| k == key).map(|(_, t)| t)
    }

    /// Any texture of this page, at another width (while the right one renders).
    fn any_width(&self, id: &str, page: u32) -> Option<&TextureHandle> {
        self.textures
            .iter()
            .rev()
            .find(|((b, p, _), r)| b == id && *p == page && r.is_ok())
            .and_then(|(_, r)| r.as_ref().ok())
    }

    /// Drops the textures of pages far from the view.
    fn prune(&mut self, id: &str, first: u32, last: u32) {
        self.textures
            .retain(|((b, p, _), _)| b == id && *p + 2 >= first && *p <= last + 2);
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
        ui.label(t("Cartea nu mai există în catalog."));
        return;
    };
    let page = app
        .session
        .progress(id)
        .and_then(|p| p.page)
        .unwrap_or(1)
        .max(1);
    let pages = book.pages.unwrap_or(0);
    // Unknown page count: one page past the current one, so scrolling goes on.
    let count = if pages > 0 { pages } else { page + 1 };

    toolbar(app, ui, &book, page, pages);
    ui.separator();
    crate::wheel_zoom(app, ui);
    let zoom = app.session.zoom();

    let avail = ui.available_size();
    let ppp = ui.ctx().pixels_per_point();
    let aspect = match &app.reader.aspect {
        Some((b, a)) if *b == book.id => *a,
        _ => DEFAULT_ASPECT,
    };
    // 100% = the page fits the width (or the whole page fits the window).
    let mut fit = avail.x - 24.0;
    if app.reader.fit_page {
        fit = fit.min((avail.y - GAP) / aspect);
    }
    let width_pts = fit.max(50.0) * zoom;
    let width_px =
        ((width_pts * ppp / WIDTH_STEP).ceil() * WIDTH_STEP).clamp(200.0, MAX_WIDTH_PX) as u32;
    let slot = egui::vec2(width_pts, width_pts * aspect);
    let stride = slot.y + GAP;
    // Wider than the window when zoomed in: it scrolls sideways too.
    let content_w = (slot.x + 24.0).max(avail.x);

    // Where to scroll this frame, if anywhere.
    let view = &mut app.reader;
    let same_book = view.shown.as_ref().is_some_and(|(b, _)| *b == book.id);
    let mut jump = None;
    // A page changed elsewhere: scroll to its top and keep it until the user scrolls.
    let to_page = !same_book || view.shown.as_ref().map(|(_, p)| *p) != Some(page);
    let resized = (view.stride - stride).abs() > 0.5 && view.stride > 0.0;
    let rezoomed = (view.zoom - zoom).abs() > f32::EPSILON;
    let relayout = view.relayout.take();
    let center = std::mem::take(&mut view.center) || to_page;
    view.center = rezoomed;
    let mut pin = to_page;
    if to_page {
        jump = Some((page - 1) as f32 * stride);
    } else if resized {
        // New page size (zoom, window, the book's real shape once a page is
        // rendered): keep the place, next frame.
        view.relayout = Some(if view.pinned.is_some() {
            Relayout::Page(page)
        } else {
            Relayout::Place((view.offset + view.view_height * READING_POINT) / view.stride)
        });
    } else if let Some(r) = relayout {
        match r {
            Relayout::Page(p) => {
                jump = Some((p - 1) as f32 * stride);
                pin = true;
            }
            Relayout::Place(place) => {
                jump = Some(place * stride - view.view_height * READING_POINT);
            }
        }
    } else if let Some(delta) = scroll_keys(ui, view.view_height) {
        jump = Some(view.offset + delta);
    }
    let max_offset = (count as f32 * stride - view.view_height).max(0.0);
    let jump = jump.map(|y| y.clamp(0.0, max_offset));
    if pin {
        view.pinned = jump;
        view.shown = Some((book.id.clone(), page));
    } else if jump.is_some() {
        view.pinned = None;
    }
    // Nothing is read back from the scroll while a relayout waits.
    let settling = view.relayout.is_some();
    view.stride = stride;
    view.zoom = zoom;
    let view_width = view.view_width;

    let mut area = egui::ScrollArea::both()
        .id_salt(("pages", &book.id))
        .auto_shrink([false, false]);
    if let Some(y) = jump {
        area = area.vertical_scroll_offset(y);
    }
    if center {
        let width = if view_width > 0.0 {
            view_width
        } else {
            avail.x
        };
        area = area.horizontal_scroll_offset(((content_w - width) / 2.0).max(0.0));
    }
    let mut clicked = None;
    let mut waiting = false;
    let out = area.show_viewport(ui, |ui, viewport| {
        let top = ui.min_rect().top();
        let left = ui.min_rect().left() + ((content_w - slot.x) / 2.0).max(0.0);
        ui.set_height(count as f32 * stride);
        ui.set_width(content_w);
        let first = (viewport.min.y / stride).floor().max(0.0) as u32 + 1;
        let last = ((viewport.max.y / stride).ceil() as u32).clamp(1, count);
        for p in first..=last {
            let rect =
                egui::Rect::from_min_size(egui::pos2(left, top + (p - 1) as f32 * stride), slot);
            waiting |= page_slot(app, ui, &book, p, width_px, rect, &mut clicked);
        }
        // Prefetch the page after the view.
        if last < count {
            let _ = app.files.page(&book, last + 1, width_px);
        }
        app.reader.prune(&book.id, first, last);
    });
    if waiting {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }

    let view = &mut app.reader;
    view.offset = out.state.clamped_offset().y;
    view.view_height = out.inner_rect.height();
    view.view_width = out.inner_rect.width();
    if let Some(cmd) = clicked {
        app.act(cmd, json!({}));
        return;
    }
    // Scrolled by the user: the page at the reading point becomes the current one.
    let pinned = view
        .pinned
        .is_some_and(|y| (y.min(max_offset) - view.offset).abs() < 1.0);
    if jump.is_none() && !pinned && !settling {
        view.pinned = None;
        let at = view.offset + view.view_height * READING_POINT;
        let current = ((at / stride).floor() as u32 + 1).clamp(1, count);
        if current != page {
            view.shown = Some((book.id.clone(), current));
            app.act("reader.goto", json!({ "page": current }));
        }
    }
}

fn toolbar(app: &mut RaftUi, ui: &mut egui::Ui, book: &raft_doc::Book, page: u32, pages: u32) {
    ui.horizontal(|ui| {
        crate::back_button(app, ui, t("Înapoi la pagina cărții"));
        ui.separator();
        if ui
            .add_enabled(page > 1, egui::Button::new(t("Anterioara")))
            .on_hover_text(t("Săgeată stânga"))
            .clicked()
        {
            app.act("reader.prev", json!({}));
        }
        let label = ui.label(t("Pagina"));
        let mut edit = page;
        let resp = ui
            .add(egui::DragValue::new(&mut edit).range(1..=pages.max(1)))
            .labelled_by(label.id);
        if resp.changed() && edit != page {
            app.act("reader.goto", json!({ "page": edit }));
        }
        ui.label(tf("din {0}", &[&pages]));
        if ui
            .add_enabled(page < pages, egui::Button::new(t("Următoarea")))
            .on_hover_text(t("Săgeată dreapta"))
            .clicked()
        {
            app.act("reader.next", json!({}));
        }
        ui.separator();
        ui.checkbox(&mut app.reader.fit_page, t("Toată pagina"));
        ui.separator();
        crate::zoom_controls(app, ui);
        ui.separator();
        ui.add(egui::Label::new(egui::RichText::new(book.display_title()).strong()).truncate());
    });
}

/// ↑/↓ scroll a little, Page Up/Down and Space a screen, Home/End to the ends.
fn scroll_keys(ui: &egui::Ui, view_height: f32) -> Option<f32> {
    if ui.ctx().egui_wants_keyboard_input() {
        return None;
    }
    let screen = (view_height * 0.9).max(LINE);
    ui.input(|i| {
        let mut delta = 0.0;
        delta += LINE * f32::from(u8::from(i.key_pressed(egui::Key::ArrowDown)));
        delta -= LINE * f32::from(u8::from(i.key_pressed(egui::Key::ArrowUp)));
        if i.key_pressed(egui::Key::PageDown) || i.key_pressed(egui::Key::Space) {
            delta += screen;
        }
        if i.key_pressed(egui::Key::PageUp) {
            delta -= screen;
        }
        if i.key_pressed(egui::Key::Home) {
            delta = f32::NEG_INFINITY;
        }
        if i.key_pressed(egui::Key::End) {
            delta = f32::INFINITY;
        }
        (delta != 0.0).then_some(delta)
    })
}

/// Draws one page in its slot (rendering it if needed); true while it renders.
fn page_slot(
    app: &mut RaftUi,
    ui: &mut egui::Ui,
    book: &raft_doc::Book,
    page: u32,
    width_px: u32,
    rect: egui::Rect,
    clicked: &mut Option<&'static str>,
) -> bool {
    let key: Key = (book.id.clone(), page, width_px);
    let mut waiting = false;
    if app.reader.get(&key).is_none() {
        match app.files.page(book, page, width_px) {
            Some(Ok(bytes)) => {
                let tex = decode_image(&bytes).map(|img| {
                    ui.ctx().load_texture(
                        format!("page-{}-{page}-{width_px}", book.id),
                        img,
                        egui::TextureOptions::LINEAR,
                    )
                });
                if let Ok(tex) = &tex {
                    let size = tex.size_vec2();
                    let known = app
                        .reader
                        .aspect
                        .as_ref()
                        .is_some_and(|(b, _)| *b == book.id);
                    if size.x > 0.0 && !known {
                        app.reader.aspect = Some((book.id.clone(), size.y / size.x));
                    }
                }
                app.reader.put(key.clone(), tex);
            }
            Some(Err(e)) => app.reader.put(key.clone(), Err(e)),
            None => waiting = true,
        }
    }
    let texture = match app.reader.get(&key) {
        Some(Ok(tex)) => Some(tex.clone()),
        Some(Err(e)) => {
            let e = e.clone();
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(40.0);
                    ui.label(tf("Pagina nu a putut fi afișată: {0}", &[&e]));
                    if ui.button(t("Deschide în aplicația externă")).clicked() {
                        app.act("book.open_external", json!({ "id": book.id }));
                    }
                });
            });
            return false;
        }
        None => app.reader.any_width(&book.id, page).cloned(),
    };
    let Some(tex) = texture else {
        ui.painter()
            .rect_filled(rect, 2.0, ui.visuals().faint_bg_color);
        let label = egui::pos2(rect.center().x, rect.top() + 60.0);
        ui.put(
            egui::Rect::from_center_size(label, egui::vec2(240.0, 40.0)),
            egui::Label::new(tf("Se randează pagina {0}…", &[&page])),
        );
        return waiting;
    };
    // The page fits its slot (pages of another shape keep their own shape).
    let size = tex.size_vec2();
    let scale = (rect.width() / size.x.max(1.0)).min(rect.height() / size.y.max(1.0));
    let shown = egui::Rect::from_center_size(rect.center(), size * scale);
    let resp = ui.put(
        shown,
        egui::Image::new((tex.id(), shown.size())).sense(egui::Sense::click()),
    );
    if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        *clicked = Some(if pos.x > resp.rect.center().x {
            "reader.next"
        } else {
            "reader.prev"
        });
    }
    waiting
}
