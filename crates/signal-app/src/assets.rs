//! Asset source: GPUI Kit's default icons plus the extra Lucide icons we use.

use std::borrow::Cow;

use gpui_kit::{AssetSource, Result, SharedString};

gpui_kit::assets::icon_assets!(
    pub ExtraIcons,
    [
        Archive,
        ArchiveRestore,
        BellOff,
        CheckCheck,
        Clock,
        Download,
        FaceSlightlySmilingPlus,
        Lock,
        LogOut,
        MessageCircle,
        NotebookPen,
        Paperclip,
        Pin,
        PinOff,
        Reply,
        SendHorizontal,
        Smartphone,
        SquarePen,
        Trash,
        Users,
        X,
    ]
);

/// Application asset source.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

/// Icon names (Lucide) used by the app.
pub use gpui_kit::assets::IconName as Icons;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extra_and_default_icons_resolve() {
        for icon in [
            Icons::Paperclip,
            Icons::SendHorizontal,
            Icons::SquarePen,
            Icons::CheckCheck,
        ] {
            let path = icon.path();
            assert!(Assets.load(&path).unwrap().is_some(), "missing {path}");
        }
        // a default component icon is still served
        let path = Icons::Search.path();
        assert!(Assets.load(&path).unwrap().is_some());
    }
}
