# Contributing to Halyard

Thanks for helping! Halyard is an unofficial desktop client for Signal, written in Rust with
GPUI. Bug reports, fixes, features, tests and docs are all welcome.

Please read the [Code of Conduct](CODE_OF_CONDUCT.md) first. To report a security problem, follow
[SECURITY.md](SECURITY.md); don't open a public issue.

## Before you start

* **Small fixes** (typos, obvious bugs, tests): open a pull request directly.
* **Features or larger changes**: open an issue first so we can agree on the approach. The goal
  is to match [Signal Desktop](https://github.com/signalapp/Signal-Desktop)'s behaviour, and
  [PARITY.md](PARITY.md) tracks where we stand.
* **Calls** are out of scope for now.

## Setting up

You need the Rust toolchain from `rust-toolchain.toml` (rustup installs it automatically) and:

* **macOS**: Xcode command line tools and `brew install protobuf`.
* **Linux** (Debian/Ubuntu): `script/install-linux-deps.sh`.

```sh
cargo run -p signal-app -- --demo   # the UI with sample data, no Signal account needed
```

The demo mode is the easiest way to work on the UI. Only link a real account when you need to
test networking, and use a test account or `--staging` (Signal's staging servers) for anything
experimental.

## Project layout

| Crate | What lives there |
|---|---|
| `crates/signal-core` | Domain model, SQLCipher database and migrations, UI state reducer, formatting. No network, no UI. |
| `crates/signal-backend` | The presage/libsignal worker thread, protocol interpretation (`interpret.rs`, pure and unit-tested), the demo backend. |
| `crates/signal-app` | The GPUI Kit UI and the `halyard` binary. |

The UI and backend only talk through `BackendCommand` and `BackendEvent`
(`signal-core/src/protocol.rs`). See the Architecture section of the README for more.

## Making changes

* **Tests**: add or update tests with your change.
  * Protocol handling goes in `signal-backend/src/interpret.rs` tests.
  * Database changes go in `signal-core/src/db/tests.rs`. Schema changes need a new migration, never an edit to an old one.
  * UI behaviour goes in `signal-app/tests/` (headless windows).
  * Visual changes: run the rendering test with `SCREENSHOT_DIR` set and look at the screenshots.
* **Style**: `cargo fmt`, and code must pass `cargo clippy --workspace --all-targets -- -D warnings`.
* **Dependencies**: keep new dependencies to a minimum, and they must be AGPL-3.0 compatible.
* **Signal's servers**: don't add anything that sends bulk or automated messages, creates
  accounts, or collects data about other users. These break Signal's terms of service.
* **Branding**: don't use Signal's name, logo or trademarks for this app. Saying "a client for
  Signal" is fine.

Before opening a pull request, run:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
script/test-linux.sh   # optional on macOS: the same suite in a Linux container (Docker)
```

CI runs the same checks on macOS and Linux.

## Pull requests

* Keep each pull request focused on one change, and explain what changed and why.
* Link the issue it fixes.
* Include screenshots for UI changes, ideally light and dark.
* Add a line to the `Unreleased` section of [CHANGELOG.md](CHANGELOG.md) for anything users will notice.

## Licence

Halyard is licensed under the [GNU AGPL v3.0 only](LICENSE). By contributing, you agree that your
contribution is licensed under the same terms.
