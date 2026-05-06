# NodeNet

NodeNet is a macOS desktop controller for SSH-managed VPN servers and 3x-ui panels. It uses Tauri 2, React, TypeScript, Zustand, Recharts, Framer Motion, and Rust.

## Requirements

- macOS with Xcode Command Line Tools
- Rust stable
- Node.js 20+
- pnpm

Install the common dependencies:

```bash
xcode-select --install
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
corepack enable
corepack prepare pnpm@latest --activate
pnpm install
```

## Development

```bash
pnpm tauri dev
```

The app config is stored at:

```text
~/Library/Application Support/NodeNet/config.json
```

SSH and 3x-ui passwords are stored in macOS Keychain via `/usr/bin/security`.
The events log is encrypted with AES-256-GCM and stored at:

```text
~/Library/Application Support/NodeNet/events.json
```

## macOS DMG Build

Install both Apple targets:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
```

Build a universal app and DMG:

```bash
pnpm tauri build --target universal-apple-darwin
```

The DMG artifact will be written to:

```text
src-tauri/target/release/bundle/dmg/
```

The bundle identifier is `com.vpnctrl.app`, and the macOS bundle category is Utilities.
