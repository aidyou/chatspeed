#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // must be installed vcpkg and sqlite3
    // git clone https://github.com/microsoft/vcpkg
    // cd vcpkg
    // .\bootstrap-vcpkg.bat
    // .\vcpkg install sqlite3:x64-windows-static-md
    // .\vcpkg install sqlite3:arm64-windows-static-md # for arm64
    println!("cargo:warning=Build script is running on Windows");

    // Use vcpkg to find the sqlite3 library
    let mut config = vcpkg::Config::new();
    config.target_triplet("x64-windows-static-md");
    match config.find_package("sqlite3") {
        Ok(_) => println!("cargo:warning=Successfully found sqlite3 via vcpkg"),
        Err(e) => {
            println!("cargo:warning=Failed to find sqlite3 via vcpkg: {}", e);
            return Err(e.into());
        }
    }

    // Use vcpkg to manage bzip2 dependency
    match config.find_package("bzip2") {
        Ok(_) => println!("cargo:warning=Successfully found bzip2 via vcpkg"),
        Err(e) => {
            println!("cargo:warning=Failed to find bzip2 via vcpkg: {}", e);
            println!("cargo:warning=Attempting to use system bzip2");
            println!("cargo:rustc-link-lib=bz2");
        }
    }

    // Static link MSVC runtime
    println!("cargo:rustc-link-arg=/MT");

    tauri_build::build()?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Static link C++ standard library
    println!("cargo:rustc-link-arg=-static-libstdc++");
    println!("cargo:rustc-link-arg=-static-libgcc");

    // Static link bzip2
    println!("cargo:rustc-link-lib=static=bz2");

    tauri_build::build()?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
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

    tauri_build::build()?;
    Ok(())
}
