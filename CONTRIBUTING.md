# Contributing to Halyard

Thank you for your help. We welcome bug reports, fixes, features, tests and documentation.

Read the [Code of Conduct](CODE_OF_CONDUCT.md) first. Do not report security problems in public
issues. Use the procedure in [SECURITY.md](SECURITY.md).

## Before you start

- For small fixes, open a pull request.
- For features and large changes, open an issue first. Then we can agree on the design.
- Halyard copies the behaviour of [Signal Desktop](https://github.com/signalapp/Signal-Desktop).
  [PARITY.md](PARITY.md) shows the status of each feature.
- Video calls and group calls are out of scope at this time.

## Set up

Install `protoc` and `cmake`. On macOS, run `brew install protobuf cmake`. On Debian or Ubuntu, run
`script/install-linux-deps.sh`. Rustup installs the correct Rust version automatically.

```sh
script/fetch-webrtc.sh              # once: Signal's prebuilt WebRTC, for voice calls
cargo run -p signal-app -- --demo   # start with sample data, no Signal account
```

Use the demo mode for UI work. In the demo, calls connect by themselves. To get an incoming call,
send "call me" to a contact. Use a test account or `--staging` for network work.

## Code structure

| Crate | Contents |
|---|---|
| `signal-core` | Data model, SQLCipher database and migrations, UI state. No network and no UI. |
| `signal-backend` | presage worker thread, protocol interpretation (`interpret.rs`), voice calls (`calls.rs`, RingRTC), demo backend. |
| `signal-app` | GPUI Kit UI and the `halyard` binary. |

- The backend runs on its own thread, because presage's `Manager` is `!Send`. The UI and the
  backend send only `BackendCommand` and `BackendEvent` messages (`signal-core/src/protocol.rs`).
- `interpret.rs` changes received content into database `Mutation`s. It is a pure function.
  The database applies each mutation atomically and sends the changed rows to the UI.
- The UI state (`ChatState`) returns effects for each event. Thus you can test UI logic
  without a window.

## Rules for changes

- Add or update tests:
  - protocol handling in `signal-backend/src/interpret.rs`;
  - database changes in `signal-core/src/db/tests.rs`;
  - UI behaviour in `signal-app/tests/`;
  - call signaling in `signal-backend/src/calls.rs`. For call changes, also run the real
    RingRTC call test: `cargo test -p signal-backend --test call_loopback -- --ignored`.
- Add a new migration for each schema change. Do not change an old migration.
- For visual changes, set `SCREENSHOT_DIR`, run the rendering test and examine the screenshots.
- Add dependencies only when necessary. Each dependency must be compatible with AGPL-3.0.
- Do not add code that sends bulk or automated messages, creates accounts or collects data
  about other users. Such code breaks Signal's terms of service.
- Do not use Signal's name, logo or trademarks for this app. "A client for Signal" is
  correct.

Before you open a pull request, run these commands:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
script/test-linux.sh                                                 # optional, needs Docker
SCREENSHOT_DIR=/tmp/shots cargo test -p signal-app --test rendering  # save screenshots
```

CI runs the same checks on macOS and Linux.

## Pull requests

- Make one change in each pull request. Tell what changed and why.
- Link the related issue.
- For UI changes, add screenshots in the light and dark themes.
- For changes that users can see, add a line to `Unreleased` in [CHANGELOG.md](CHANGELOG.md).

## Licence

Halyard uses the [GNU AGPL v3.0 only](LICENSE). Your contributions use the same licence.
