fn main() {
    if std::env::var("CARGO_CFG_WINDOWS").is_ok() {
        winres::WindowsResource::new()
            .set_icon("assets/icon.ico")
            .compile()
            .expect("failed to embed Windows icon resource");
    }
}
