use eframe::egui;

/// egui's bundled fonts have no CJK glyphs, so pull in a Japanese system font
/// as a fallback. Fails silently (Latin-only) if none can be read.
pub fn install_japanese_fallback(ctx: &egui::Context) {
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let candidates = ["YuGothM.ttc", "YuGothR.ttc", "meiryo.ttc", "msgothic.ttc"];
    for name in candidates {
        let path = std::path::Path::new(&windir).join("Fonts").join(name);
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let mut fonts = egui::FontDefinitions::default();
        fonts
            .font_data
            .insert("jp-fallback".into(), egui::FontData::from_owned(bytes).into());
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .push("jp-fallback".into());
        }
        ctx.set_fonts(fonts);
        return;
    }
    eprintln!("kwick: no Japanese system font found; CJK text will not render");
}
