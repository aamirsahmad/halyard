//! Device linking screen, modelled after Signal Desktop's install flow.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{Icon, Sizable as _};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    Pixels, Styled as _, TestSupportExt as _, Window, div, px, rgb,
};
use signal_core::AccountState;

use crate::app::SignalApp;
use crate::assets::Icons;
use crate::theme::Palette;

/// QR modules as rows of `(start, len)` dark runs, plus the matrix width.
pub fn qr_runs(data: &str) -> Option<(usize, Vec<Vec<(usize, usize)>>)> {
    let code = qrcode::QrCode::with_error_correction_level(data, qrcode::EcLevel::M).ok()?;
    let width = code.width();
    let colors = code.to_colors();
    let rows = colors
        .chunks(width)
        .map(|row| {
            let mut runs = Vec::new();
            let mut x = 0;
            while x < width {
                if row[x] == qrcode::Color::Dark {
                    let start = x;
                    while x < width && row[x] == qrcode::Color::Dark {
                        x += 1;
                    }
                    runs.push((start, x - start));
                } else {
                    x += 1;
                }
            }
            runs
        })
        .collect();
    Some((width, rows))
}

fn render_qr(data: &str, size: Pixels) -> AnyElement {
    let Some((width, rows)) = qr_runs(data) else {
        return div().child("Unable to render QR code").into_any_element();
    };
    let quiet = 4.;
    let module = size / (width as f32 + 2. * quiet);
    div()
        .id("link-qr")
        .size(size)
        .bg(rgb(0xFFFFFF))
        .rounded(px(12.))
        .p(module * quiet)
        .flex()
        .flex_col()
        .children(rows.into_iter().map(move |runs| {
            div()
                .relative()
                .h(module)
                .w_full()
                .children(runs.into_iter().map(move |(start, len)| {
                    div()
                        .absolute()
                        .top_0()
                        .left(module * start as f32)
                        .w(module * len as f32)
                        .h(module)
                        .bg(rgb(0x000000))
                }))
        }))
        .test_support()
        .into_any_element()
}

impl SignalApp {
    pub(super) fn render_link_screen(
        &mut self,
        p: &Palette,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (qr, status): (AnyElement, Option<AnyElement>) = match &self.state.account {
            AccountState::Linking { url: Some(url) } => (render_qr(url, px(260.)), None),
            AccountState::Linking { url: None } => (
                div()
                    .size(px(260.))
                    .rounded(px(12.))
                    .bg(p.hover)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Spinner::new().large())
                    .into_any_element(),
                None,
            ),
            AccountState::LinkFailed(err) => (
                placeholder_qr(p),
                Some(
                    div()
                        .id("link-error")
                        .max_w(px(360.))
                        .text_color(p.danger)
                        .text_size(px(13.))
                        .child(err.clone())
                        .test_support()
                        .into_any_element(),
                ),
            ),
            _ => (placeholder_qr(p), None),
        };
        let linking = matches!(self.state.account, AccountState::Linking { .. });

        let steps = div()
            .flex()
            .flex_col()
            .gap_3()
            .max_w(px(380.))
            .child(
                div()
                    .text_size(px(24.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Link this device to your phone"),
            )
            .child(step(1, "Open Signal on your phone", p))
            .child(step(2, "Tap Settings, then tap Linked devices", p))
            .child(step(3, "Tap Link new device and scan this code", p))
            .when(!linking, |d| {
                d.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .mt_4()
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(p.text_secondary)
                                .child("Device name"),
                        )
                        .child(Input::new(&self.device_name).id("device-name")),
                )
                .child(
                    Button::new("link-device")
                        .primary()
                        .large()
                        .icon(Icon::new(Icons::Smartphone))
                        .label(
                            if matches!(self.state.account, AccountState::LinkFailed(_)) {
                                "Try again"
                            } else {
                                "Show linking code"
                            },
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.start_linking(cx))),
                )
            })
            .when(linking, |d| {
                d.child(
                    div()
                        .mt_4()
                        .text_size(px(13.))
                        .text_color(p.text_secondary)
                        .child("Waiting for your phone… Keep this window open."),
                )
            })
            .children(status);

        div()
            .id("link-screen")
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .gap_16()
            .bg(p.background)
            .child(qr)
            .child(steps)
            .test_support()
            .into_any_element()
    }
}

fn placeholder_qr(p: &Palette) -> AnyElement {
    div()
        .size(px(260.))
        .rounded(px(12.))
        .bg(p.hover)
        .flex()
        .items_center()
        .justify_center()
        .child(Icon::new(Icons::Lock).large().text_color(p.text_secondary))
        .into_any_element()
}

fn step(n: u8, text: &'static str, p: &Palette) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap_3()
        .child(
            div()
                .size(px(24.))
                .flex_none()
                .rounded_full()
                .bg(p.accent)
                .text_color(p.accent_text)
                .text_size(px(12.))
                .font_weight(FontWeight::BOLD)
                .flex()
                .items_center()
                .justify_center()
                .child(n.to_string()),
        )
        .child(div().text_color(p.text).child(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_runs_cover_finder_patterns() {
        let (width, rows) = qr_runs("sgnl://linkdevice?uuid=abc&pub_key=xyz").unwrap();
        assert!(width >= 21);
        assert_eq!(rows.len(), width);
        // top-left finder pattern: first row starts with a 7-module dark run
        assert_eq!(rows[0][0], (0, 7));
        // runs never exceed the width
        for row in &rows {
            for (start, len) in row {
                assert!(start + len <= width);
            }
        }
    }
}
