# Halyard

An unofficial, native desktop client for the Signal messenger on **macOS and Linux**, written in Rust.
The UI is built with [GPUI](https://www.gpui.rs/) via [GPUI Kit](https://gpui-kit.com/) and
modelled on [Signal Desktop](https://github.com/signalapp/Signal-Desktop). The Signal protocol
comes from [libsignal](https://github.com/signalapp/libsignal), through
[presage](https://github.com/whisperfish/presage) and
[libsignal-service-rs](https://github.com/whisperfish/libsignal-service-rs).

This version supports messaging only. Voice and video calls are planned.

> Halyard is an independent project. It is not affiliated with or endorsed by Signal Messenger LLC
> or the Signal Technology Foundation. "Signal" is a trademark of the Signal Technology Foundation.
> Signal may restrict access for third-party clients at any time; use it at your own risk.

## Features

The UI follows Signal Desktop closely; see [PARITY.md](PARITY.md) for a feature-by-feature
comparison. Highlights:

| Area | Features |
|---|---|
| Account | Link as a secondary device (QR code), edit your profile name and about, device info, unlink |
| Messaging | 1:1 and group (v2) chats, leave group, membership notices, Note to Self, replies, reactions (quick bar + full emoji picker), edit (24 h) with history, delete for me / for everyone, forward, message info |
| Rich content | Contact and group photos, bold / italic / strikethrough / monospace / spoiler formatting, @mentions with autocomplete, view-once media, clickable links, link-preview cards, stickers, shared contacts, image viewer with Save As, file attachments (picker or drag and drop, up to 100 MB) |
| Privacy | Disappearing messages (timer, countdown, notices), message requests from your phone's accepted/blocked state (accept / block / delete), block / unblock, safety numbers, read receipts and typing indicators toggles, notification content setting |
| Chat list | Collapses to avatars in narrow windows, pin, timed mute, mark read/unread, archive, delete, drafts, typing previews, search across chats and messages, search within a chat, unread divider |
| Stories & media | Contacts' stories (text and photo, 24 h) with a viewer and replies; post your own text or photo stories; voice notes play in the app and are recorded as MP3 like Signal Desktop |
| App | Zoom (⌘/Ctrl + / − / 0), light / dark / system themes with Signal's accent, desktop notifications, unread count in the title, keyboard shortcuts (⌘/Ctrl+N, ⌘/Ctrl+F, Alt+↑/↓, ⌘/Ctrl+, , ⌘/Ctrl+/, Esc) |
| Storage | Encrypted (SQLCipher) protocol and message stores, AES-256-GCM encrypted attachment store |

Not supported yet: registering as a primary device and phone-number contact discovery (this is a
linked device, like Signal Desktop). Voice and video calls are planned.

## Architecture

```
crates/
  signal-core      domain model, SQLCipher message DB, attachment crypto, UI state reducer,
                   timeline grouping, time formatting (no network, no UI)
  signal-backend   presage/libsignal worker on its own thread; interprets protocol content
                   into DB mutations; demo backend with sample data
  signal-app       GPUI Kit UI (`halyard` binary)
```

* presage's `Manager` is `!Send`, so the backend runs on a dedicated thread with a
  single-threaded tokio runtime and a `LocalSet`. The UI and the backend communicate only
  through `BackendCommand` and `BackendEvent` messages (`signal-core::protocol`).
* The backend turns decrypted `Content` into `Mutation`s
  (`signal-backend::interpret`). This is a pure function with its own unit tests. The
  mutations are applied atomically to the local database (`signal-core::db`), and the
  changed rows go to the UI as events.
* The UI keeps a `ChatState` (`signal-core::state`). Applying an event returns *effects*
  (mark as read, notify, re-render the timeline), so the UI logic is testable without a
  window.

### Data and security

All data lives in the platform data directory (`~/Library/Application Support/org.halyard.Halyard`
on macOS, `~/.local/share/halyard` on Linux; data from the app's former name, Signal GPUI, is
moved there on first launch), or in `--data-dir`:

| File | Contents | Protection |
|---|---|---|
| `protocol.db3` | identity keys, sessions, pre-keys, contacts, groups (presage) | SQLCipher |
| `messages.db3` | conversations and messages | SQLCipher |
| `attachments/*.sga` | downloaded attachments | AES-256-GCM, key derived with HKDF from the DB key |
| `config.json` | random 256-bit database key | file mode `0600`, directory `0700` |
| `open-cache/` | decrypted copies of files you open in other apps | cleared on every start and on unlink |

Like Signal Desktop, the database key is stored next to the databases. The encryption
protects backups and copies of the data folder. It does not protect against malware
running as your user. When you send an attachment, the original file is referenced in
place and is not copied.

## Building

Requires Rust **1.99.1** (pinned in `rust-toolchain.toml`; rustup installs it) and `protoc`.

**macOS** (11+): Xcode command line tools and `brew install protobuf`.

**Linux** (Ubuntu 24.04 / Debian; other distributions need the equivalent packages):

```sh
script/install-linux-deps.sh
```

Then:

```sh
cargo run --release -p signal-app            # link to your phone and start chatting
cargo run --release -p signal-app -- --demo  # try the UI with sample data, no account needed
```

Options: `--data-dir <PATH>`, `--staging` (Signal's staging servers), `--help`. Set
`SIGNAL_LOG=debug` to get verbose logs.

### Linking

1. Start the app and click **Show linking code**.
2. On your phone, open Signal → Settings → Linked devices → Link new device.
3. Scan the QR code. Contacts and groups sync automatically.

### Packaging

* macOS: `script/bundle-macos.sh` builds `target/release/bundle/Halyard.app`. Sign and
  notarize it with your Developer ID before you distribute it.
* Linux: install `target/release/halyard` into your `PATH` and
  `packaging/linux/halyard.desktop` into `~/.local/share/applications/`.

## Testing

```sh
cargo test --workspace                                         # unit, integration and UI tests
cargo test -p signal-app --test rendering                      # offscreen rendering (Metal / Vulkan)
SCREENSHOT_DIR=/tmp/shots cargo test -p signal-app --test rendering   # also save screenshots
cargo test -p signal-backend --test worker -- --ignored        # live: request a real linking URL from Signal
script/test-linux.sh                                           # full suite in an Ubuntu container
```

* **signal-core**: database (migrations, encryption, pagination, receipts, edits,
  reactions, read tracking, search escaping), state reducer, timeline grouping,
  formatting and attachment crypto.
* **signal-backend**: protocol interpretation for every supported message type, data
  paths and key handling, the demo backend round trip, and the real presage worker
  (starts unlinked, creates encrypted stores).
* **signal-app**: headless UI integration tests (`#[gpui_kit::test]`) drive the real
  views with clicks and keystrokes: open a chat, send, reply, search, new-chat picker,
  archive, linking flow, drafts, notifications, the members panel, editing, Note to Self
  (`tests/ui.rs`), and every Signal Desktop parity feature (`tests/parity.rs`). A rendering test
  paints every screen with the real GPU renderer in light and dark themes.

CI (`.github/workflows/ci.yml`) runs fmt, clippy (`-D warnings`), tests, rendering tests
and a release build on Ubuntu and macOS.

## License

AGPL-3.0-only, the same as libsignal, presage and Signal Desktop. See [LICENSE](LICENSE).
