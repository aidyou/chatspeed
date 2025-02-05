#[cfg(windows)]
fn main() {
    // must be installed vcpkg and dependencies:
    // git clone https://github.com/microsoft/vcpkg
    // cd vcpkg
    // .\bootstrap-vcpkg.bat
    // .\vcpkg install bzip2:x64-windows-static-md
    // .\vcpkg install sqlite3:x64-windows-static-md
    // .\vcpkg install sqlite3:arm64-windows-static-md # for arm64
    println!("cargo:warning=Build script is running on Windows");

    // Use vcpkg to find the sqlite3 library
    let mut config = vcpkg::Config::new();
    config.target_triplet("x64-windows-static-md");
    let lib = config.find_package("sqlite3")
        .expect("Failed to find sqlite3 via vcpkg. Please ensure sqlite3:x64-windows-static-md is installed.");
    println!("cargo:warning=Successfully found sqlite3 via vcpkg");

    // Use vcpkg to manage bzip2 dependency
    config.find_package("bzip2")
        .expect("Failed to find bzip2 via vcpkg. Please ensure bzip2:x64-windows-static-md is installed.");
    println!("cargo:warning=Successfully found bzip2 via vcpkg");

    if cfg!(target_env = "msvc") {
        let libs = ["sqlite3", "bzip2"];
        let mut errors = Vec::new();

        for lib in libs {
            vcpkg::find_package(lib)
                .map(|lib| {
                    println!("cargo:rustc-link-search=native={}", lib.link_paths[0].display());
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

        println!("cargo:rustc-link-lib=static=shell32");
    }

    tauri_build::build().expect("Failed to run tauri-build");
}

#[cfg(target_os = "linux")]
fn main() {
    // Static link C++ standard library
    println!("cargo:rustc-link-arg=-static-libstdc++");
    println!("cargo:rustc-link-arg=-static-libgcc");

    // Static link bzip2
    println!("cargo:rustc-link-lib=static=bz2");

    tauri_build::build().expect("Failed to run tauri-build");
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

    tauri_build::build().expect("Failed to run tauri-build");
}
