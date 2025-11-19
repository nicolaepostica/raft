//! The settings window: theme, interface language and the library folders.

use raft_engine::{SourceEntry, SourceStatus};
use serde_json::json;

use crate::i18n::{self, t, tf};
use crate::{Pick, RaftUi};

/// Width of the name and path column.
const NAME_WIDTH: f32 = 300.0;

/// Draws the settings window when it is open.
pub(crate) fn window(app: &mut RaftUi, ctx: &egui::Context) {
    if !app.session.settings_open() {
        return;
    }
    let mut open = true;
    egui::Window::new(t("Setări"))
        .id(egui::Id::unique("settings"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(760.0)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            appearance(app, ui);
            ui.add_space(12.0);
            sources(app, ui);
        });
    if !open {
        app.act("view.settings", json!({ "open": false }));
    }
}

fn appearance(app: &mut RaftUi, ui: &mut egui::Ui) {
    ui.heading(t("Aspect"));
    egui::Grid::new("settings-appearance")
        .num_columns(2)
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            ui.label(t("Temă"));
            ui.horizontal(|ui| {
                let dark = app.session.dark();
                if ui.radio(!dark, t("Albă")).clicked() && dark {
                    app.act("view.theme", json!({ "theme": "light" }));
                }
                if ui.radio(dark, t("Întunecată")).clicked() && !dark {
                    app.act("view.theme", json!({ "theme": "dark" }));
                }
            });
            ui.end_row();
            let label = ui.label(t("Limba interfeței"));
            let current = i18n::Lang::from_code(app.session.language()).unwrap_or(i18n::Lang::Ro);
            let combo = egui::ComboBox::new("settings-language", "")
                .selected_text(current.name())
                .show_ui(ui, |ui| {
                    for lang in i18n::Lang::ALL {
                        if ui.selectable_label(lang == current, lang.name()).clicked() {
                            app.act("view.language", json!({ "language": lang.code() }));
                        }
                    }
                });
            combo.response.labelled_by(label.id);
            ui.end_row();
        });
}

fn sources(app: &mut RaftUi, ui: &mut egui::Ui) {
    ui.heading(t("Surse de cărți"));
    ui.label(
        egui::RichText::new(t(
            "Folderele cu cărți (fiecare cu catalog.db făcut de build-catalog). Un folder dezactivat rămâne în listă, dar cărțile lui nu se arată.",
        ))
        .weak(),
    );
    ui.add_space(6.0);
    let entries: Vec<SourceEntry> = app.session.sources().to_vec();
    if entries.is_empty() {
        ui.label(t("Niciun folder încă."));
    }
    egui::Grid::new("settings-sources")
        .num_columns(4)
        .spacing([12.0, 10.0])
        .striped(true)
        .show(ui, |ui| {
            for entry in &entries {
                source_row(app, ui, entry);
                ui.end_row();
            }
        });
    ui.add_space(8.0);
    let picking = app.picking.is_some();
    if ui
        .add_enabled(!picking, egui::Button::new(t("Adaugă un folder…")))
        .clicked()
    {
        crate::add_folder(app);
    }
}

fn source_row(app: &mut RaftUi, ui: &mut egui::Ui, entry: &SourceEntry) {
    let path = entry.source.path.clone();
    let mut on = entry.source.enabled;
    let resp = ui.checkbox(&mut on, "");
    let label = tf("Activ: {0}", &[&entry.source.display_name()]);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, &label));
    if resp
        .on_hover_text(t("Activează sau dezactivează folderul"))
        .changed()
    {
        app.act("source.enable", json!({ "path": path, "enabled": on }));
    }

    ui.vertical(|ui| {
        ui.set_width(NAME_WIDTH);
        let name = app
            .source_names
            .entry(path.clone())
            .or_insert_with(|| entry.source.name.clone());
        let placeholder = raft_doc::Source::new(path.clone())
            .display_name()
            .to_owned();
        let resp = ui.add(
            egui::TextEdit::singleline(name)
                .hint_text(placeholder)
                .desired_width(NAME_WIDTH),
        );
        let typed = name.trim().to_owned();
        if resp.lost_focus() {
            if typed != entry.source.name {
                app.act("source.edit", json!({ "path": path, "name": typed }));
            }
            app.source_names.remove(&path);
        } else if !resp.has_focus() {
            // Not being edited: follow the saved name (it may change elsewhere).
            app.source_names.remove(&path);
        }
        let shown = ui
            .add(egui::Label::new(egui::RichText::new(path.as_str()).small().weak()).truncate())
            .on_hover_text(path.as_str());
        // The name field is announced by the folder's path.
        resp.labelled_by(shown.id);
    });

    match &entry.status {
        SourceStatus::Loaded => {
            ui.label(tf("{0} cărți", &[&app.session.source_books(&path)]));
        }
        SourceStatus::Disabled => {
            ui.label(egui::RichText::new(t("dezactivat")).weak());
        }
        SourceStatus::Unavailable(reason) => {
            ui.horizontal(|ui| {
                ui.colored_label(ui.visuals().warn_fg_color, t("indisponibil"))
                    .on_hover_text(reason.as_str());
                if ui.small_button(t("Reîncearcă")).clicked() {
                    app.act("source.enable", json!({ "path": path, "enabled": true }));
                }
            });
        }
    }

    ui.horizontal(|ui| {
        let picking = app.picking.is_some();
        if ui
            .add_enabled(!picking, egui::Button::new(t("Schimbă folderul…")))
            .on_hover_text(t(
                "Pentru un folder mutat: cărțile își păstrează progresul și corecturile",
            ))
            .clicked()
        {
            crate::pick_folder(app, Pick::Move(path.clone()));
        }
        if ui
            .button(t("Scoate"))
            .on_hover_text(t(
                "Scoate folderul din bibliotecă (fișierele rămân pe disc)",
            ))
            .clicked()
        {
            app.act("source.remove", json!({ "path": path }));
            app.source_names.remove(&path);
        }
    });
}
