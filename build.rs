//! Embeds the icon and version information into the Windows executable.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/spotilite.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("assets/spotilite.ico")
        .set("ProductName", "SpotiLite")
        .set("FileDescription", "SpotiLite - client Spotify Premium léger")
        .set("LegalCopyright", "Licence MIT");
    // A missing resource compiler must not break the build: the app still runs,
    // it just shows the default icon in Explorer.
    if let Err(e) = resource.compile() {
        println!("cargo:warning=icône Windows non intégrée : {e}");
    }
}
