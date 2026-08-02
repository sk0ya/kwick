use eframe::egui;

/// egui's bundled fonts have no CJK glyphs, so pull in a Japanese system font.
///
/// It goes *first* in the proportional family rather than last: as a fallback,
/// mixed 和欧 text is drawn half in egui's Latin font and half in the Japanese
/// one, whose differing size and baseline make every mixed line look ragged.
/// Yu Gothic/Meiryo cover Latin too, so using one font for everything keeps a
/// single baseline. Monospace keeps it as a trailing fallback, where a
/// proportional font must not become the default.
/// Fails silently (Latin-only) if no Japanese font can be read.
pub fn install_japanese_fallback(ctx: &egui::Context) {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let candidates = ["meiryo.ttc", "YuGothM.ttc", "YuGothR.ttc", "msgothic.ttc"];
    for name in candidates {
        let path = std::path::Path::new(&windir).join("Fonts").join(name);
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let mut fonts = egui::FontDefinitions::default();
        fonts
            .font_data
            .insert("jp".into(), egui::FontData::from_owned(bytes).into());
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "jp".into());
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .push("jp".into());
        ctx.set_fonts(fonts);
        return;
    }
    eprintln!("kwick: no Japanese system font found; CJK text will not render");
}
