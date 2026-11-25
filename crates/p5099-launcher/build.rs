fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=../../assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return Ok(());
    }
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("../../assets/icon.ico")
        .set("ProductName", "P5099")
        .set("FileDescription", "P5099");
    resource.compile()
}
