#[cfg(windows)]
fn main() {
    use std::env;

    // Detect target architecture
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "x86_64".to_string());
    let is_arm64 = target_arch == "aarch64";

    println!("cargo:warning=Target architecture: {}", target_arch);

    // Set environment variables for LLVM/Clang
    if cfg!(target_env = "msvc") {
        println!("cargo:rustc-env=CC=clang");
        println!("cargo:rustc-env=CXX=clang++");
        println!("cargo:rustc-env=LIBCLANG_PATH=C:\\Program Files\\LLVM\\bin");
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

    // 设置 vcpkg 搜索路径
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

    // Use vcpkg to manage bzip2 dependency
    config.find_package("bzip2").unwrap_or_else(|_| {
        panic!(
            "Failed to find bzip2 via vcpkg. Please ensure bzip2:{} is installed.",
            triplet
        )
    });
    println!("cargo:warning=Successfully found bzip2 via vcpkg");

    if cfg!(target_env = "msvc") {
        // 设置 Platform 和 PreferredToolArchitecture
        if is_arm64 {
            println!("cargo:rustc-env=Platform=ARM64");
        } else {
            println!("cargo:rustc-env=Platform=x64");
        }
        println!("cargo:rustc-env=PreferredToolArchitecture=x64");

        // 从环境变量获取 Visual Studio 和 Windows SDK 路径
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

        // 从环境变量获取 Windows SDK 路径
        if let Ok(windows_sdk_dir) = env::var("WindowsSdkDir") {
            if let Ok(windows_sdk_version) = env::var("WindowsSDKVersion") {
                let sdk_version = windows_sdk_version.trim_end_matches('\\');
                let windows_sdk_dir = windows_sdk_dir.trim_end_matches('\\');

                // Windows SDK UM 库路径
                let sdk_um_path = format!("{}\\Lib\\{}\\um\\x64", windows_sdk_dir, sdk_version);
                println!("cargo:rustc-link-search=native={}", sdk_um_path);

                // Windows SDK UCRT 库路径
                let sdk_ucrt_path = format!("{}\\Lib\\{}\\ucrt\\x64", windows_sdk_dir, sdk_version);
                println!("cargo:rustc-link-search=native={}", sdk_ucrt_path);
            }
        }

        // 添加必要的系统库
        let system_libs = ["shell32", "user32", "advapi32", "userenv", "ws2_32"];
        for lib in system_libs {
            println!("cargo:rustc-link-lib={}", lib);
        }

        let libs = ["sqlite3", "bzip2"];
        let mut errors = Vec::new();

        for lib in libs {
            vcpkg::find_package(lib)
                .map(|lib| {
                    println!(
                        "cargo:rustc-link-search=native={}",
                        lib.link_paths[0].display()
                    );
                })
                .map_err(|e| errors.push((lib, e)))
                .ok();
        }

        if !errors.is_empty() {
            for (lib, e) in errors {
                eprintln!("Error finding {}: {}", lib, e);
            }
            std::process::exit(1);
        }
    }

    // Add shell32 library for Windows
    if cfg!(target_os = "windows") {
        println!("cargo:rustc-link-lib=shell32");
    }

    tauri_build::build();
}

#[cfg(target_os = "linux")]
fn main() {
    // Static link C++ standard library
    println!("cargo:rustc-link-arg=-static-libstdc++");
    println!("cargo:rustc-link-arg=-static-libgcc");

    // Static link bzip2
    println!("cargo:rustc-link-lib=static=bz2");

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
