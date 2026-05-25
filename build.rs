fn main() {
    println!("cargo:rerun-if-changed=art/app-icon.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    winresource::WindowsResource::new()
        .set_icon("art/app-icon.ico")
        .compile()
        .expect("failed to embed Windows application icon");
}
