# NodeNet

NodeNet is a macOS desktop controller for SSH-managed VPN servers and 3x-ui panels. It uses Tauri 2, React, TypeScript, Zustand, Recharts, Framer Motion, and Rust.

## Requirements

- macOS with Xcode Command Line Tools
- Rust stable
- Node.js 22+
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
src-tauri/target/universal-apple-darwin/release/bundle/dmg/
```

The bundle identifier is `com.nodenet.app`, and the macOS bundle category is Utilities.

## Updating NodeNet

NodeNet publishes macOS release builds from Git tags such as `v0.2.0`. The app checks:

```text
https://github.com/DarkDo11/NodeNet/releases/latest/download/latest.json
```

The updater manifest uses the standard Tauri format and points to the universal DMG release asset. Builds are not Apple Developer ID signed or notarized, so users may need to right-click NodeNet and choose Open on first launch.

Generate updater signing keys locally before the first release:

```bash
pnpm tauri signer generate -w ~/.tauri/nodenet.key
```

Add the private key as the GitHub secret `TAURI_SIGNING_PRIVATE_KEY`. If you set a key password, add it as `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Paste the generated public key into `src-tauri/tauri.conf.json` as `plugins.updater.pubkey`.

User data lives outside `NodeNet.app`, under:

```text
~/Library/Application Support/NodeNet/
```

These files and folders survive app updates automatically:

- `config.json` - server list and settings
- `events.json` - encrypted alert log
- `known_hosts.json` - SSH fingerprints
- `metrics-cache.json` - metrics history
- `backups/` - downloaded 3x-ui configs

None of these are inside the app bundle, so replacing `NodeNet.app` during an update does not touch them.
