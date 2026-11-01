//! Embeds the icon (Explorer, the Start menu, shortcuts and the tray show
//! it) and the version information (the Task Manager shows the
//! description as the app's name) in the Windows executable.
//!
//! Only when building on Windows, where the resource compiler is; a cross
//! build from Linux goes without them, and the tray then draws its own icon.

fn main() {
    println!("cargo:rerun-if-changed=assets/flexflyout.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        // Resource ID 1, which the tray loads (`tray::icon`).
        resource.set_icon_with_id("assets/flexflyout.ico", "1");
        resource.set("ProductName", "FlexFlyout");
        resource.set("FileDescription", "FlexFlyout");
        resource.set("LegalCopyright", "Copyright (c) 2026 Akiva Miura");
        resource
            .compile()
            .expect("failed to embed the Windows icon");
    }
}
