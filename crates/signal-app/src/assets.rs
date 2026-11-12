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
        CircleDashed,
        Clock,
        Download,
        FaceSlightlySmiling,
        FaceSlightlySmilingPlus,
        Forward,
        Lock,
        LogOut,
        MessageCircle,
        Mic,
        NotebookPen,
        Paperclip,
        Pin,
        Pencil,
        PinOff,
        Reply,
        SendHorizontal,
        Settings,
        Smartphone,
        SquarePen,
        Trash,
        Users,
        X,
    ]
);

/// Signal Desktop's own icons (send button, message status), by asset path.
pub mod signal_icons {
    pub const SEND: &str = "signal/send-fill.svg";
    pub const SENDING: &str = "signal/messagestatus-sending.svg";
    pub const SENT: &str = "signal/messagestatus-sent.svg";
    pub const DELIVERED: &str = "signal/messagestatus-delivered.svg";
    pub const READ: &str = "signal/messagestatus-read.svg";
    pub const PHONE: &str = "signal/phone.svg";
    pub const PHONE_FILL: &str = "signal/phone-fill.svg";
    pub const PHONE_DOWN: &str = "signal/phone-down-fill.svg";
    pub const MIC: &str = "signal/mic-fill.svg";
    pub const MIC_OFF: &str = "signal/mic-slash-fill.svg";
}

const SIGNAL_ICONS: [(&str, &[u8]); 10] = [
    (
        signal_icons::SEND,
        include_bytes!("../assets/signal/send-fill.svg"),
    ),
    (
        signal_icons::SENDING,
        include_bytes!("../assets/signal/messagestatus-sending.svg"),
    ),
    (
        signal_icons::SENT,
        include_bytes!("../assets/signal/messagestatus-sent.svg"),
    ),
    (
        signal_icons::DELIVERED,
        include_bytes!("../assets/signal/messagestatus-delivered.svg"),
    ),
    (
        signal_icons::READ,
        include_bytes!("../assets/signal/messagestatus-read.svg"),
    ),
    (
        signal_icons::PHONE,
        include_bytes!("../assets/signal/phone.svg"),
    ),
    (
        signal_icons::PHONE_FILL,
        include_bytes!("../assets/signal/phone-fill.svg"),
    ),
    (
        signal_icons::PHONE_DOWN,
        include_bytes!("../assets/signal/phone-down-fill.svg"),
    ),
    (
        signal_icons::MIC,
        include_bytes!("../assets/signal/mic-fill.svg"),
    ),
    (
        signal_icons::MIC_OFF,
        include_bytes!("../assets/signal/mic-slash-fill.svg"),
    ),
];

/// Application asset source.
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = SIGNAL_ICONS.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.extend(
            SIGNAL_ICONS
                .iter()
                .filter(|(p, _)| p.starts_with(path))
                .map(|(p, _)| SharedString::from(*p)),
        );
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
    fn signal_icons_resolve() {
        for path in [
            signal_icons::SEND,
            signal_icons::SENDING,
            signal_icons::SENT,
            signal_icons::DELIVERED,
            signal_icons::READ,
            signal_icons::PHONE,
            signal_icons::PHONE_FILL,
            signal_icons::PHONE_DOWN,
            signal_icons::MIC,
            signal_icons::MIC_OFF,
        ] {
            let bytes = Assets.load(path).unwrap().expect(path);
            assert!(bytes.starts_with(b"<svg"), "{path}");
        }
    }

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
