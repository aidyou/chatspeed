# Tauri + Vue 3

This template should help get you started developing with Tauri + Vue 3 in Vite. The template uses Vue 3 `<script setup>` SFCs, check out the [script setup docs](https://v3.vuejs.org/api/sfc-script-setup.html#sfc-script-setup) to learn more.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Volar](https://marketplace.visualstudio.com/items?itemName=Vue.volar) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## Development

```sh
yarn install
yarn tauri dev
```

## Build

```sh
# https://v2.tauri.app/zh-cn/distribute/
yarn tauri build --no-bundle
# bundle for distribution outside the macOS App Store
yarn tauri bundle --bundles app,dmg
```

## Requirements

sqlite3

### Windows

#### Prerequisites
1. Install Visual Studio 2022 with:
   - "Desktop development with C++" workload
   - "MSVC v143 - VS 2022 C++ ARM64 build tools" (for ARM64 builds)
   - Windows 11 SDK

2. Install vcpkg dependencies:
```sh
git clone https://github.com/microsoft/vcpkg
cd vcpkg
.\bootstrap-vcpkg.bat

# For x64 builds
.\vcpkg install sqlite3:x64-windows-static-md
.\vcpkg install bzip2:x64-windows-static-md

# For ARM64 builds
.\vcpkg install sqlite3:arm64-windows-static-md
.\vcpkg install bzip2:arm64-windows-static-md
```

#### Building on Windows ARM64
When building on Windows ARM64 (or cross-compiling for ARM64), you need to set up the Visual Studio environment first:

1. Open Command Prompt (cmd.exe)
2. Navigate to the project directory
3. Run the environment setup script:
```sh
setup-env.bat
```
4. In the same command prompt window, run the build command:
```sh
yarn tauri build
```

Note: The environment setup needs to be done each time you open a new command prompt window, as the environment variables are only valid for the current session.

### Linux

#### Build Dependencies

```sh
sudo apt install pkg-config
sudo apt-get install libglib2.0-dev
sudo apt-get install libgtk-3-dev libgdk-pixbuf-2.0-dev
sudo apt-get install libssl-dev
sudo apt install libsoup2.4-1 libsoup2.4-dev
sudo apt install libjavascriptcoregtk-4.1-dev
sudo apt install libwebkit2gtk-4.1-dev
sudo apt install libappindicator3-dev
sudo apt install librsvg2-dev

sudo apt install sqlite3
```

#### Run Dependencies

```sh
sudo apt install libappindicator3-1
```
