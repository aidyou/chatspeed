#[cfg(windows)]
fn main() {
    use std::env;

    // Force the vcpkg crate to ignore the system-wide VCPKG_ROOT environment variable
    // and use the path we provide programmatically. This is crucial because the build
    // script might set a conflicting VCPKG_ROOT.
    env::remove_var("VCPKG_ROOT");

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

    // Set correct triplet based on architecture
    let triplet = if is_arm64 {
        "arm64-windows-static"
    } else {
        "x64-windows-static"
    };
    println!("cargo:warning=Using vcpkg triplet: {}", triplet);

    // --- Manual linking for vcpkg dependencies ---
    // This bypasses the `vcpkg` crate's find_package logic, which was failing,
    // and instead tells rustc directly where to find the libraries.
    use std::path::PathBuf;

    let vcpkg_locations = vec![
        PathBuf::from("../vcpkg_installed"),
        PathBuf::from("vcpkg_installed"),
    ];

    let mut vcpkg_found = false;
    for path in vcpkg_locations {
        if path.exists() {
            let absolute_path = path.canonicalize().unwrap_or_else(|_| path.clone());
            let lib_path = absolute_path.join(triplet).join("lib");

            if lib_path.exists() {
                println!("cargo:warning=Found vcpkg artifact directory at: {}", absolute_path.display());
                println!("cargo:warning=Adding library search path: {}", lib_path.display());
                println!("cargo:rustc-link-search=native={}", lib_path.display());
                vcpkg_found = true;
                break;
            }
        }
    }

    if !vcpkg_found {
        panic!("Could not find a local vcpkg artifact directory ('vcpkg_installed'). Please ensure it exists and contains the required libraries for the '{}' triplet.", triplet);
    }

    // Link dependencies directly
    println!("cargo:rustc-link-lib=static=sqlite3");
    println!("cargo:rustc-link-lib=static=bz2");

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
