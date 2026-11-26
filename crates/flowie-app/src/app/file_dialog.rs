//! File dialogs (open, save) and what happens when one can't open: on Linux they go
//! through xdg-desktop-portal, which minimal desktops (i3, sway) may not have installed.

use gpui::{AsyncApp, WeakEntity};

use crate::app::i18n::t;
use crate::app::platform::{Os, OS};
use crate::outline::Outline;

/// What the user chose in a file dialog, or `None` if they cancelled. A dialog that couldn't
/// open (on Linux, without xdg-desktop-portal) is reported to the user instead of failing quietly.
pub async fn choice<T>(
    rx: futures::channel::oneshot::Receiver<gpui::Result<Option<T>>>,
    this: &WeakEntity<Outline>,
    cx: &mut AsyncApp,
) -> Option<T> {
    match rx.await {
        Ok(Ok(choice)) => choice,
        Ok(Err(error)) => {
            eprintln!("flowie: file dialog failed: {error:#}");
            this.update(cx, |this, cx| {
                this.toast(match OS {
                    Os::Linux => t!("toast.file_dialog_failed_linux").to_string(),
                    _ => t!("toast.file_dialog_failed", error = error),
                });
                cx.notify();
            })
            .ok();
            None
        }
        Err(_) => None,
    }
}
