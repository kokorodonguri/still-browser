fn main() {
    println!("cargo:rerun-if-changed=still.rc");
    println!("cargo:rerun-if-changed=../../desktop/src-tauri/icons/icon.ico");
    // Checks the target, not the host: release builds cross-compile from Linux.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("still.rc", embed_resource::NONE).manifest_required().unwrap();
    }
}
