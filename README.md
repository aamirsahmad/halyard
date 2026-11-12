# Halyard

[![CI](https://github.com/aamirsahmad/halyard/actions/workflows/ci.yml/badge.svg)](https://github.com/aamirsahmad/halyard/actions/workflows/ci.yml)
[![License: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)

Halyard is an unofficial desktop client for the Signal messenger. It runs on macOS and Linux.
It is written in Rust with [GPUI](https://www.gpui.rs/) and [GPUI Kit](https://gpui-kit.com/).
The design follows [Signal Desktop](https://github.com/signalapp/Signal-Desktop).
The Signal protocol comes from [libsignal](https://github.com/signalapp/libsignal), through
[presage](https://github.com/whisperfish/presage).

> Halyard is an independent project. Signal Messenger LLC and the Signal Technology Foundation
> do not endorse it. "Signal" is a trademark of the Signal Technology Foundation.
> Signal can stop access for third-party clients at any time. Use Halyard at your own risk.

## Features

- 1:1 chats, group chats and Note to Self.
- 1:1 voice calls, with Signal's own call library (RingRTC). Call history in the chat.
  A choice of ringtones.
- Replies, reactions, edits, deletion, forwarding and message information.
- Formatting, @mentions, link previews, stickers, view-once media and attachments.
- Voice messages. Halyard plays them and records them as MP3.
- Disappearing messages, message requests, blocking and safety numbers.
- Stories. You can view, reply to and post stories.
- Pinned, muted and archived chats. Search, drafts and notifications.
- Light and dark themes, zoom and keyboard shortcuts.
- Encrypted local storage.

Halyard does not support video calls or group calls. You cannot register a new account with Halyard.
Halyard connects to your phone as a linked device, like Signal Desktop.
[PARITY.md](PARITY.md) compares each feature with Signal Desktop.

## Install

Download the package for your system from the
[releases page](https://github.com/aamirsahmad/halyard/releases).

**macOS 11 or later** (Apple silicon and Intel). Use [Homebrew](https://brew.sh/):

```sh
brew install --cask aamirsahmad/halyard/halyard
```

Or install the package by hand:

1. Open `Halyard-<version>-macos-universal.dmg`.
2. Drag Halyard to Applications.
3. Apple has not notarised Halyard yet, so macOS blocks the first start
   ("Apple could not verify Halyard"). Click **Done**. Open **System Settings → Privacy & Security**.
   Then click **Open Anyway**. Or run `xattr -dr com.apple.quarantine /Applications/Halyard.app`.

**Linux** (x86_64 or arm64, glibc 2.35 or later, for example Ubuntu 22.04, Debian 12 or Fedora 36):

- AppImage: run `chmod +x Halyard-*.AppImage`. Then run the file.
- Debian or Ubuntu: run `sudo apt install ./halyard_<version>_<arch>.deb`.
- Other systems: extract the `.tar.gz` file. Then run `./halyard`, or run `./install.sh`
  to add Halyard to the applications menu.

Linux needs a Vulkan driver. Most desktops have one. On Debian and Ubuntu, the package is
`mesa-vulkan-drivers`.

## Link your phone

1. Start Halyard. Click **Show linking code**.
2. On your phone, open Signal. Go to **Settings → Linked devices → Link new device**.
3. Scan the QR code. Halyard then gets your contacts and groups.

## Build from source

You need `protoc`, `cmake` and the Rust version in `rust-toolchain.toml`. Rustup installs that
version automatically.

- macOS: install the Xcode command line tools. Then run `brew install protobuf cmake`.
- Debian or Ubuntu: run `script/install-linux-deps.sh`.

Voice calls need Signal's prebuilt WebRTC library. Download it once before you build:

```sh
script/fetch-webrtc.sh                        # get WebRTC for calls (checks its SHA-256)
cargo run --release -p signal-app            # start Halyard
cargo run --release -p signal-app -- --demo  # start with sample data, no account
cargo test --workspace                        # run the tests
script/package-macos.sh                       # make the macOS .dmg and .zip
script/package-linux.sh                       # make the Linux packages (needs Docker)
```

Other options: `--data-dir <PATH>`, `--staging` and `--help`. For detailed logs, set
`SIGNAL_LOG=debug`. A `v<version>` tag starts the release workflow. This workflow builds all
packages and makes a draft GitHub release. When you publish the release, the Homebrew
workflow updates the cask in [homebrew-halyard](https://github.com/aamirsahmad/homebrew-halyard).

## Data and security

Halyard keeps its data in one folder:

- macOS: `~/Library/Application Support/org.halyard.Halyard`
- Linux: `~/.local/share/halyard`

SQLCipher encrypts the message and key databases. AES-256-GCM encrypts the attachments.
The database key is in the same folder, as in Signal Desktop. Thus the encryption protects
backups and copies of the folder. It does not protect against malware that runs as your user.

## Contributing

Contributions are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md) and the
[Code of Conduct](CODE_OF_CONDUCT.md). Report security problems privately, as
[SECURITY.md](SECURITY.md) tells you. [CHANGELOG.md](CHANGELOG.md) lists the changes.

**Status:** early. Automated tests and the demo mode cover all features. Some network features
have had little testing with real accounts: leaving groups, posting stories, and the sync of
message requests and blocks to your phone.
Please report all bugs.

## License

AGPL-3.0-only, the same licence as libsignal, presage and Signal Desktop. See [LICENSE](LICENSE).
The icons in `crates/signal-app/assets/signal/` come from Signal Desktop (AGPL-3.0).
The Halyard icon is original artwork with the same licence.
