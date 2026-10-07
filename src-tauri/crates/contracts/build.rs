fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    println!(
        "cargo:rustc-env=CHATSpeed_WORK_ROOT={}",
        root.join("../../..").display()
    );
}
