//! The app's colours, read from `assets/theme.toml` (the icon uses the same file).

const TOML: &str = include_str!("../../../assets/theme.toml");

/// The quoted value of `key` in theme.toml.
fn value(key: &str) -> Option<&'static str> {
    TOML.lines().find_map(|line| {
        let (k, v) = line.split_once('=')?;
        if k.trim() != key {
            return None;
        }
        v.trim().strip_prefix('"')?.split('"').next()
    })
}

/// `#rrggbb` → colour.
fn parse_hex(hex: &str) -> Option<egui::Color32> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    Some(egui::Color32::from_rgb(byte(0)?, byte(2)?, byte(4)?))
}

/// A theme colour; grey if the key is missing (a test checks that none is).
pub fn color(key: &str) -> egui::Color32 {
    value(key)
        .and_then(parse_hex)
        .unwrap_or(egui::Color32::GRAY)
}

/// Colour of the "edited by hand" marks.
pub fn edited() -> egui::Color32 {
    color("edited")
}

/// Colour of the "incomplete data" marks.
pub fn incomplete() -> egui::Color32 {
    color("incomplete")
}

fn visuals(dark: bool) -> egui::Visuals {
    let (mut v, side) = if dark {
        (egui::Visuals::dark(), "_dark")
    } else {
        (egui::Visuals::light(), "_light")
    };
    let c = |name: &str| color(&format!("{name}{side}"));
    let accent = color("accent");
    v.panel_fill = c("bg");
    v.window_fill = c("bg");
    v.faint_bg_color = c("panel");
    v.extreme_bg_color = c("panel");
    v.override_text_color = Some(c("fg"));
    v.hyperlink_color = accent;
    v.selection.bg_fill = if dark { color("accent_press") } else { accent };
    v.selection.stroke = egui::Stroke::new(1.0, if dark { c("fg") } else { c("bg") });
    v.widgets.inactive.bg_fill = c("faint");
    v.widgets.inactive.weak_bg_fill = c("faint");
    v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, c("faint"));
    v.widgets.active.bg_fill = accent;
    v.widgets.active.weak_bg_fill = accent;
    v
}

/// Installs the light and dark visuals and picks one.
pub fn apply(ctx: &egui::Context, dark: bool) {
    ctx.set_visuals_of(egui::Theme::Light, visuals(false));
    ctx.set_visuals_of(egui::Theme::Dark, visuals(true));
    ctx.set_theme(if dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_colour_the_app_uses_is_in_the_theme() {
        for key in [
            "accent",
            "accent_press",
            "bg_light",
            "panel_light",
            "fg_light",
            "faint_light",
            "bg_dark",
            "panel_dark",
            "fg_dark",
            "faint_dark",
            "edited",
            "incomplete",
        ] {
            assert!(value(key).and_then(parse_hex).is_some(), "{key}");
        }
        assert_eq!(value("name"), Some("Raft"));
        assert_eq!(
            parse_hex("#1f6f6a"),
            Some(egui::Color32::from_rgb(31, 111, 106))
        );
        assert_eq!(parse_hex("1f6f6a"), None);
        assert_eq!(parse_hex("#1f6f6"), None);
    }
}
