#[cfg(windows)]
fn main() {
    use std::env;

    // Detect target architecture
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "x86_64".to_string());
    let is_arm64 = target_arch == "aarch64";

    println!("cargo:warning=Target architecture: {}", target_arch);

    // Link against required Windows libraries
    println!("cargo:rustc-link-lib=shell32"); // Required for shell integration
    println!("cargo:rustc-link-lib=user32"); // Required for GUI and user interaction
    println!("cargo:rustc-link-lib=gdi32"); // Required for GUI rendering
    println!("cargo:rustc-link-lib=advapi32"); // Required for Windows API
    println!("cargo:rustc-link-lib=userenv"); // Required for user environment
    println!("cargo:rustc-link-lib=ws2_32"); // Required for networking

    // must be installed vcpkg and dependencies:
    // git clone https://github.com/microsoft/vcpkg
    // cd vcpkg
    // .\bootstrap-vcpkg.bat
    // .\vcpkg install sqlite3:x64-windows-static
    // .\vcpkg install bzip2:x64-windows-static
    println!("cargo:warning=Build script is running on Windows");

    // Use vcpkg to find dependencies
    let mut config = vcpkg::Config::new();

    // Set correct triplet based on architecture
    let triplet = if is_arm64 {
        "arm64-windows-static"
    } else {
        "x64-windows-static"
    };

    println!("cargo:warning=Using vcpkg triplet: {}", triplet);
    config.target_triplet(triplet);

    // Set vcpkg search path
    if let Ok(vcpkg_root) = env::var("VCPKG_ROOT") {
        use std::path::PathBuf;
        config.vcpkg_root(PathBuf::from(vcpkg_root));
    }

    // Find and link dependencies
    config.find_package("sqlite3").unwrap_or_else(|_| {
        panic!(
            "Failed to find sqlite3 via vcpkg. Please ensure sqlite3:{} is installed.",
            triplet
        )
    });

    let bzip2 = config.find_package("bzip2").unwrap_or_else(|_| {
        panic!(
            "Failed to find bzip2 via vcpkg. Please ensure bzip2:{} is installed.",
            triplet
        )
    });
    println!(
        "cargo:rustc-link-search=native={}",
        bzip2.link_paths[0].display()
    );

    // Set platform architecture for MSVC
    if cfg!(target_env = "msvc") {
        println!(
            "cargo:rustc-env=Platform={}",
            if is_arm64 { "ARM64" } else { "x64" }
        );
        println!("cargo:rustc-env=PreferredToolArchitecture=x64");
    }

    tauri_build::build();
}

#[cfg(target_os = "linux")]
fn main() {
    // Static link C++ standard library and runtime
    println!("cargo:rustc-link-arg=-static-libstdc++");
    println!("cargo:rustc-link-arg=-static-libgcc");

    // Add library search paths
    println!("cargo:rustc-link-search=native=/usr/lib");
    println!("cargo:rustc-link-search=native=/usr/lib/x86_64-linux-gnu");
    println!("cargo:rustc-link-search=native=/lib/x86_64-linux-gnu");

    // Static link dependencies
    println!("cargo:rustc-link-lib=static=sqlite3");
    println!("cargo:rustc-link-lib=static=bz2");

    // Add symbolic linking for better compatibility
    println!("cargo:rustc-link-arg=-Wl,-Bsymbolic");

    tauri_build::build();
}

#[cfg(target_os = "macos")]
fn main() {
    // Static link SQLite3 and bzip2
    if cfg!(target_arch = "x86_64") {
        println!("cargo:rustc-link-search=/usr/local/opt/bzip2/lib");
        println!("cargo:rustc-link-search=/usr/local/opt/sqlite/lib");
    } else {
        println!("cargo:rustc-link-search=/opt/homebrew/opt/bzip2/lib");
        println!("cargo:rustc-link-search=/opt/homebrew/opt/sqlite/lib");
    }
    println!("cargo:rustc-link-lib=static=bz2");
    println!("cargo:rustc-link-lib=static=sqlite3");

    // Set up libc++ linking on macOS
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/local/lib");
    // 添加应用程序框架路径
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");

    tauri_build::build();
}
