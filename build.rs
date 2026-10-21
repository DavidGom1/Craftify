//! Windows resources: the Craftify icon and version details on both executables.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico")
            .set("ProductName", "Craftify")
            .set("FileDescription", "Craftify: SolidWorks y Fusion a SolidCraft")
            .set("CompanyName", "Hidroconta");
        if let Err(e) = res.compile() {
            // A missing resource compiler only costs the icon, never the build.
            println!("cargo:warning=no Windows resources: {e}");
        }
    }
}
