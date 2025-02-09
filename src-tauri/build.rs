#[cfg(windows)]
fn main() {
    use std::env;

    // Detect target architecture
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "x86_64".to_string());
    let is_arm64 = target_arch == "aarch64";

    println!("cargo:warning=Target architecture: {}", target_arch);

    // Set environment variables for Windows build
    if cfg!(target_env = "msvc") {
        // Set compiler environment variables
        println!("cargo:rustc-env=CC=clang");
        println!("cargo:rustc-env=CXX=clang++");

        // Get LLVM path from environment variable or use default
        let llvm_path =
            env::var("LLVM_PATH").unwrap_or_else(|_| "C:\\Program Files\\LLVM".to_string());
        println!("cargo:rustc-env=LIBCLANG_PATH={}/bin", llvm_path);

        // Link against required Windows libraries
        println!("cargo:rustc-link-lib=shell32");
        println!("cargo:rustc-link-lib=user32");
        println!("cargo:rustc-link-lib=advapi32");
        println!("cargo:rustc-link-lib=userenv");
        println!("cargo:rustc-link-lib=ws2_32");
        println!("cargo:rustc-link-lib=msvcrt");
        println!("cargo:rustc-link-lib=ucrt");
        println!("cargo:rustc-link-lib=vcruntime");

        // Set Visual Studio environment variables if not already set
        if env::var("VCINSTALLDIR").is_err() {
            println!("cargo:rustc-env=PreferredToolArchitecture=x64");

            // These variables will be set by GitHub Actions or Visual Studio environment
            // We only set them if they're not already set
            for var in &[
                "VCINSTALLDIR",
                "WindowsSdkDir",
                "WindowsSDKVersion",
                "VCToolsInstallDir",
                "VCToolsVersion",
            ] {
                if let Ok(value) = env::var(var) {
                    println!("cargo:rustc-env={}={}", var, value);
                }
            }
        }

        // Get Windows SDK and MSVC paths
        let (windows_sdk_path, msvc_path) = get_windows_sdk_and_msvc_paths();
        
        // Add basic Windows link libraries
        println!("cargo:rustc-link-lib=user32");
        println!("cargo:rustc-link-lib=gdi32");
        println!("cargo:rustc-link-lib=shell32");

        // Add MSVC runtime library configurations
        println!("cargo:rustc-link-lib=msvcrt");
        println!("cargo:rustc-link-lib=ucrt");
        println!("cargo:rustc-link-lib=vcruntime");
        
        // Dynamically add search paths
        if let Some(sdk_path) = windows_sdk_path {
            println!("cargo:rustc-link-search=native={}/Lib/10.0/ucrt/x64", sdk_path);
        }
        
        if let Some(msvc_path) = msvc_path {
            if let Ok(entries) = std::fs::read_dir(&msvc_path) {
                if let Some(latest_version) = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().is_dir())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .max() {
                    println!("cargo:rustc-link-search=native={}/{}/lib/x64", msvc_path, latest_version);
                }
            }
        }
    }

    // must be installed vcpkg and dependencies:
    // git clone https://github.com/microsoft/vcpkg
    // cd vcpkg
    // .\bootstrap-vcpkg.bat
    // .\vcpkg install sqlite3:x64-windows-static-md
    // .\vcpkg install bzip2:x64-windows-static-md
    // .\vcpkg install sqlite3:arm64-windows-static-md # for arm64
    println!("cargo:warning=Build script is running on Windows");

    // Use vcpkg to find the sqlite3 library
    let mut config = vcpkg::Config::new();

    // Set correct triplet based on architecture
    let triplet = if is_arm64 {
        "arm64-windows-static-md"
    } else {
        "x64-windows-static-md"
    };

    println!("cargo:warning=Using vcpkg triplet: {}", triplet);
    config.target_triplet(triplet);

    // Set vcpkg search path
    if let Ok(vcpkg_root) = env::var("VCPKG_ROOT") {
        use std::path::PathBuf;
        config.vcpkg_root(PathBuf::from(vcpkg_root));
    }

    config.find_package("sqlite3").unwrap_or_else(|_| {
        panic!(
            "Failed to find sqlite3 via vcpkg. Please ensure sqlite3:{} is installed.",
            triplet
        )
    });
    println!("cargo:warning=Successfully found sqlite3 via vcpkg");

    // Add link directive for sqlite3
    if let Ok(lib) = config.find_package("sqlite3") {
        println!(
            "cargo:rustc-link-search=native={}",
            lib.link_paths[0].display()
        );
        println!("cargo:rustc-link-lib=sqlite3");
    }

    // Use vcpkg to manage bzip2 dependency
    config.find_package("bzip2").unwrap_or_else(|_| {
        panic!(
            "Failed to find bzip2 via vcpkg. Please ensure bzip2:{} is installed.",
            triplet
        )
    });
    if let Ok(lib) = config.find_package("bzip2") {
        println!(
            "cargo:rustc-link-search=native={}",
            lib.link_paths[0].display()
        );
        println!("cargo:rustc-link-lib=bz2");
    }
    println!("cargo:warning=Successfully found bzip2 via vcpkg");

    if cfg!(target_env = "msvc") {
        // Set Platform and PreferredToolArchitecture
        if is_arm64 {
            println!("cargo:rustc-env=Platform=ARM64");
        } else {
            println!("cargo:rustc-env=Platform=x64");
        }
        println!("cargo:rustc-env=PreferredToolArchitecture=x64");

        // Get Visual Studio and Windows SDK paths from environment variables
        if let Ok(tools_version) = env::var("VCToolsVersion") {
            if let Ok(vs_path) = env::var("VSINSTALLDIR") {
                let msvc_lib = format!(
                    "{}\\VC\\Tools\\MSVC\\{}\\lib\\x64",
                    vs_path.trim_end_matches('\\'),
                    tools_version
                );
                println!("cargo:rustc-link-search=native={}", msvc_lib);
            }
        }

        // Get Windows SDK paths from environment variables
        if let Ok(windows_sdk_dir) = env::var("WindowsSdkDir") {
            if let Ok(windows_sdk_version) = env::var("WindowsSDKVersion") {
                let sdk_version = windows_sdk_version.trim_end_matches('\\');
                let windows_sdk_dir = windows_sdk_dir.trim_end_matches('\\');

                // Windows SDK UM library path
                let sdk_um_path = format!("{}\\Lib\\{}\\um\\x64", windows_sdk_dir, sdk_version);
                println!("cargo:rustc-link-search=native={}", sdk_um_path);

                // Windows SDK UCRT library path
                let sdk_ucrt_path = format!("{}\\Lib\\{}\\ucrt\\x64", windows_sdk_dir, sdk_version);
                println!("cargo:rustc-link-search=native={}", sdk_ucrt_path);
            }
        }
    }

    tauri_build::build();
}

fn get_windows_sdk_and_msvc_paths() -> (Option<String>, Option<String>) {
    use std::path::PathBuf;

    let vswhom_path = match vswhom::VsFindResult::search() {
        Ok(result) => Some(PathBuf::from(result.vs_path)),
        Err(_) => None,
    };

    let msvc_path = vswhom_path.map(|vs_path| {
        vs_path.join("VC\\Tools\\MSVC")
            .to_string_lossy()
            .into_owned()
    });

    let windows_sdk_path = std::env::var("WindowsSdkDir")
        .ok()
        .or_else(|| {
            if let Ok(output) = std::process::Command::new("reg")
                .args(&["query", "HKLM\\SOFTWARE\\Microsoft\\Windows Kits\\Installed Roots", "/v", "KitsRoot10"])
                .output() {
                String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .find(|line| line.contains("KitsRoot10"))
                    .and_then(|line| line.split_whitespace().last())
                    .map(String::from)
            } else {
                None
            }
        });

    (windows_sdk_path, msvc_path)
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
    // Static link bzip2
    if cfg!(target_arch = "x86_64") {
        println!("cargo:rustc-link-search=/usr/local/opt/bzip2/lib");
    } else {
        println!("cargo:rustc-link-search=/opt/homebrew/opt/bzip2/lib");
    }
    println!("cargo:rustc-link-lib=static=bz2");

    // Set up libc++ linking on macOS
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/local/lib");

    tauri_build::build();
}
