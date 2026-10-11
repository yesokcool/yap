//! macOS: link ScreenCaptureKit weakly, so Yap still starts on macOS before
//! 12.3 (screen sharing then says it needs 12.3; nothing touches it before).
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-arg=-Wl,-weak_framework,ScreenCaptureKit");
    }
}
