// Embeds the icon into the .exe on Windows, so the file shows it in Explorer
// and on shortcuts. Does nothing when building for Linux or macOS.
fn main() {
    println!("cargo:rerun-if-changed=../assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../assets/icon.ico");
        res.set("ProductName", "metascrub");
        res.set("FileDescription", "metascrub (command line) - remove hidden metadata from files");
        res.compile().expect("failed to embed the Windows icon");
    }
}
