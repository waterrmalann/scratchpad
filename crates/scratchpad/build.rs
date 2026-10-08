//! Embeds the app icon and version information into the Windows executable.
//!
//! GPUI takes the window and taskbar icon from the executable's icon resource with ID 1
//! (`LoadImageW(module, 1, ...)` in its Windows platform), which is what `set_icon` writes.
//! The version fields feed the file's Properties dialog and Windows' app lists.

fn main() {
    println!("cargo::rerun-if-changed=../../assets/icons/scratchpad.ico");

    // Cargo runs build scripts on the host, so the target has to come from the environment:
    // cross-compiling to Windows from elsewhere must not skip this, and the reverse must not run it.
    if std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows") {
        embed_windows_resources();
    }
}

// `winresource` is a Windows-host build dependency only, so the body needs the host check as well.
#[cfg(windows)]
fn embed_windows_resources() {
    winresource::WindowsResource::new()
        .set_icon("../../assets/icons/scratchpad.ico")
        .set("ProductName", "Scratchpad")
        .set("FileDescription", "Scratchpad")
        .set("OriginalFilename", "scratchpad.exe")
        .compile()
        .expect("failed to embed the Windows icon and version information");
}

#[cfg(not(windows))]
fn embed_windows_resources() {
    println!(
        "cargo::warning=building for Windows from another OS: the icon and version information are not embedded"
    );
}
