# Tauri + Vue 3

This template should help get you started developing with Tauri + Vue 3 in Vite. The template uses Vue 3 `<script setup>` SFCs, check out the [script setup docs](https://v3.vuejs.org/api/sfc-script-setup.html#sfc-script-setup) to learn more.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Volar](https://marketplace.visualstudio.com/items?itemName=Vue.volar) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## dev

```sh
yarn install
yarn tauri dev
```

## build

```sh
# https://v2.tauri.app/zh-cn/distribute/
yarn tauri build --no-bundle
# bundle for distribution outside the macOS App Store
yarn tauri bundle --bundles app,dmg
```

## requirements

sqlite3

### windows

```sh
git clone https://github.com/microsoft/vcpkg
cd vcpkg
# for x64
vcpkg install sqlite3:x64-windows-static-md
```

### linux

#### build dependencies

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

#### run dependencies

```sh
sudo apt install libappindicator3-1
```
