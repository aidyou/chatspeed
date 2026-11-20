//! Embeds the window-class icon used by GPUI on Windows.

fn main() {
    println!("cargo:rerun-if-changed=windows-icon.rc");
    println!("cargo:rerun-if-changed=../../icons/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var_os("CARGO_FEATURE_GUI").is_some()
    {
        if let Err(error) =
            embed_resource::compile_for("windows-icon.rc", ["chatspeed-gpui"], embed_resource::NONE)
                .manifest_required()
        {
            panic!("Could not embed ChatSpeed icon: {error}");
        }
    }
}
