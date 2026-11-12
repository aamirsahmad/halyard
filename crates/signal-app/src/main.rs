//! Halyard desktop client entry point.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use gpui_kit::{AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size};
use signal_app::assets::Assets;
use signal_app::theme::{Appearance, apply_appearance};
use signal_app::{DesktopNotifier, SignalApp, actions};
use signal_backend::{BackendConfig, DataPaths, EventSink, SignalServers};
use signal_core::{BackendEvent, Database};

const USAGE: &str = "\
Halyard — an unofficial desktop client for Signal (not affiliated with Signal)

USAGE:
    halyard [OPTIONS]

OPTIONS:
    --demo              Run with sample data, without a Signal account
    --data-dir <PATH>   Store data in PATH instead of the platform default
    --staging           Use Signal's staging servers (development only)
    -h, --help          Print this help
    -V, --version       Print the version

ENVIRONMENT:
    SIGNAL_LOG          Log filter, e.g. `info` or `signal_backend=debug`
";

#[derive(Debug, Default, PartialEq)]
struct Args {
    demo: bool,
    staging: bool,
    data_dir: Option<PathBuf>,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Option<Args>, String> {
    let mut out = Args::default();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--demo" => out.demo = true,
            "--staging" => out.staging = true,
            "--data-dir" => {
                out.data_dir = Some(args.next().ok_or("--data-dir needs a path")?.into());
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("halyard {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            other => return Err(format!("unknown argument: {other}\n\n{USAGE}")),
        }
    }
    Ok(Some(out))
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("SIGNAL_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,libsignal=warn,presage=info"));
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(filter)
        .init();
}

fn default_data_dir() -> anyhow::Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("org", "halyard", "Halyard")
        .context("could not determine a data directory")?;
    let dir = dirs.data_dir().to_path_buf();
    // the app used to be called "Signal GPUI": keep existing data
    if let Some(old) = directories::ProjectDirs::from("org", "signal-gpui", "Signal GPUI") {
        adopt_old_data_dir(old.data_dir(), &dir)?;
    }
    Ok(dir)
}

/// Moves `old` to `new` when only the old data directory exists.
fn adopt_old_data_dir(old: &std::path::Path, new: &std::path::Path) -> anyhow::Result<()> {
    if new.exists() || !old.exists() {
        return Ok(());
    }
    if let Some(parent) = new.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(old, new).with_context(|| {
        format!(
            "could not move data from {} to {}",
            old.display(),
            new.display()
        )
    })
}

fn main() -> anyhow::Result<()> {
    let Some(args) = parse_args(std::env::args().skip(1)).map_err(anyhow::Error::msg)? else {
        return Ok(());
    };
    init_logging();

    let root = match args.data_dir {
        Some(dir) => dir,
        None => default_data_dir()?,
    };
    let root = if args.demo { root.join("demo") } else { root };
    let paths = DataPaths::new(&root);
    let settings_file = root.join("ui.conf");
    let key = paths
        .load_or_create_db_key()
        .context("failed to load the database key")?;
    // Open (and migrate) the message database before the backend starts.
    let attachment_access = signal_app::attachments::AttachmentAccess::new(
        paths.attachment_store(&key),
        paths.open_cache_dir(),
    );
    let db = Database::open(paths.app_db(), Some(&key))
        .context("failed to open the message database")?;
    tracing::info!(data_dir = %root.display(), demo = args.demo, "starting");

    let (tx, rx) = futures::channel::mpsc::unbounded::<BackendEvent>();
    let sink: EventSink = Arc::new(move |event| {
        let _ = tx.unbounded_send(event);
    });
    let backend = if args.demo {
        signal_backend::demo::spawn_demo(
            paths.app_db(),
            Some(key),
            sink,
            Duration::from_millis(900),
        )?
    } else {
        let servers = if args.staging {
            SignalServers::Staging
        } else {
            SignalServers::Production
        };
        signal_backend::spawn(
            BackendConfig {
                paths,
                db_key: key,
                servers,
            },
            sink,
        )?
    };

    gpui_kit::application().with_assets(Assets).run(move |cx| {
        gpui_kit::init(cx);
        actions::bind_keys(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("Halyard".into()),
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1120.), px(780.)),
                cx,
            ))),
            window_min_size: Some(size(px(480.), px(420.))),
            app_id: Some("org.halyard.Halyard".into()),
            ..Default::default()
        };
        let opened = gpui_kit::open_window(options, cx, move |window, cx| {
            apply_appearance(Appearance::System, Some(window), cx);
            cx.new(|cx| {
                let mut app =
                    SignalApp::new(db, Rc::new(backend), Rc::new(DesktopNotifier), window, cx);
                app.use_settings_file(settings_file, window, cx);
                app.set_attachment_access(attachment_access);
                app.listen(rx, window, cx);
                app
            })
        });
        if let Err(e) = opened {
            tracing::error!(%e, "failed to open the main window");
            cx.quit();
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adopts_data_from_the_former_app_name() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("old");
        let new = tmp.path().join("nested").join("new");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("messages.db3"), b"x").unwrap();
        adopt_old_data_dir(&old, &new).unwrap();
        assert!(!old.exists());
        assert_eq!(std::fs::read(new.join("messages.db3")).unwrap(), b"x");

        // an existing new directory is never replaced
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("stale"), b"y").unwrap();
        adopt_old_data_dir(&old, &new).unwrap();
        assert!(old.join("stale").exists() && !new.join("stale").exists());

        // nothing to adopt
        adopt_old_data_dir(&tmp.path().join("missing"), &tmp.path().join("other")).unwrap();
        assert!(!tmp.path().join("other").exists());
    }

    fn parse(v: &[&str]) -> Result<Option<Args>, String> {
        parse_args(v.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses_flags() {
        assert_eq!(parse(&[]).unwrap(), Some(Args::default()));
        let a = parse(&["--demo", "--data-dir", "/tmp/x", "--staging"])
            .unwrap()
            .unwrap();
        assert!(a.demo && a.staging);
        assert_eq!(a.data_dir, Some(PathBuf::from("/tmp/x")));
        assert_eq!(parse(&["--help"]).unwrap(), None);
        assert!(parse(&["--data-dir"]).is_err());
        assert!(parse(&["--bogus"]).is_err());
    }
}
