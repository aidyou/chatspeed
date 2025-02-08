# ChatSpeed

🚧 **Project Status: Under Active Development** 🚧

ChatSpeed is an innovative desktop AI chat application built with Tauri and Vue 3, focusing on performance, privacy, and user experience. The project is currently under active development and not yet ready for production use.

## Current Status

- 🏗️ Core Features Development
  - Multi-model AI chat support
  - Basic conversation interface
  - Note-taking functionality
  - Multi-language support

- ⚠️ Please Note
  - This repository is in active development
  - Features and APIs may change significantly
  - Not recommended for production use yet
  - Documentation is being worked on

## Coming Soon

- 📚 Comprehensive documentation
- 🔄 Auto-update system
- 🎨 Theme customization
- 🌐 Official website

## Development Requirements

### System Dependencies
- sqlite3: Required for database operations
- bzip2: Required for compression functionality

### Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Volar](https://marketplace.visualstudio.com/items?itemName=Vue.volar) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## Development

```sh
yarn install
yarn tauri dev
```

## Build

### Windows

#### Prerequisites

1. Install Visual Studio 2022 with:
   - "Desktop development with C++" workload
   - Windows SDK (10.0.22621.0 or later)
   - MSVC v143 - VS 2022 C++ x64/x86 build tools
   - For ARM64 builds: "MSVC v143 - VS 2022 C++ ARM64 build tools"

2. Install Node.js and Yarn

   ```sh
   # Install yarn if not already installed
   npm install -g yarn
   ```

3. Install Rust

   ```sh
   # Install from https://rustup.rs/
   rustup target add x86_64-pc-windows-msvc  # For x64 builds
   rustup target add aarch64-pc-windows-msvc # For ARM64 builds
   ```

4. Install Dependencies

   ```sh
   # Install project dependencies
   yarn install
   ```

5. Install and Configure vcpkg

   ```sh
   # Clone and bootstrap vcpkg
   git clone https://github.com/microsoft/vcpkg
   cd vcpkg
   .\bootstrap-vcpkg.bat

   # Install required libraries
   # For x64 builds:
   .\vcpkg install sqlite3:x64-windows-static-md
   .\vcpkg install bzip2:x64-windows-static-md

   # For ARM64 builds:
   .\vcpkg install sqlite3:arm64-windows-static-md
   .\vcpkg install bzip2:arm64-windows-static-md
   ```

#### Building

Option 1: Using automated build script (Recommended)

```sh
# This will automatically set up the environment and build
.\build.bat
```

Option 2: Manual build

```sh
# First, set up environment variables
.\setup-env.ps1  # PowerShell script (Recommended)
# or
.\setup-env.bat  # Batch script (For compatibility)

# Then build
yarn tauri build
```

The build output will be located in `src-tauri/target/release/`.

Note: The environment setup needs to be done each time you open a new command prompt window, as the environment variables are only valid for the current session.

### Linux

#### Prerequisites

```sh
# Install system dependencies
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
sudo apt install libappindicator3-1

# Install Node.js and Yarn
curl -fsSL https://deb.nodesource.com/setup_lts.x | sudo -E bash -
sudo apt-get install -y nodejs
npm install -g yarn

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

#### Building

```sh
# Install dependencies
yarn install

# Build
yarn tauri build
```

### macOS

#### Prerequisites

```sh
# Install Homebrew if not already installed
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"

# Install system dependencies
brew install node
brew install yarn
brew install sqlite3

# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

#### Building

```sh
# Install dependencies
yarn install

# Build without bundle
yarn tauri build --no-bundle

# Bundle for distribution outside the macOS App Store
yarn tauri bundle --bundles app,dmg
```

For more details about macOS distribution, please refer to the [Tauri documentation](https://v2.tauri.app/zh-cn/distribute/).
