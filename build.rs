fn main() {
    println!("cargo:rerun-if-changed=assets/gitr.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/gitr.ico")
            .set("ProductName", "Gitr")
            .set("FileDescription", "Gitr — A fast, native Git client")
            .compile()
            .expect("failed to compile the Windows icon resource");
    }
}
