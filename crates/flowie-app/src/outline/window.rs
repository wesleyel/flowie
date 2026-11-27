//! What the outline keeps about its window, which can close to the system tray (Windows and
//! Linux) and open again while the outline lives on: where the window was, its subscriptions,
//! the tray icon and the dialog asking what closing does.

use std::rc::Rc;

use gpui::{App, Subscription, Window};

use crate::app::tray::{Tray, TrayLabels};
use crate::app::window_place::Place;

/// Modal asking whether closing the window hides to the system tray or quits
/// (Windows and Linux, when `config.close_action` is `Ask`).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CloseDialog {
    pub remember: bool,
}

#[derive(Default)]
pub(crate) struct WindowState {
    /// The window's size and place as observed this session, saved on quit.
    place: Option<Place>,
    /// The place loaded from `meta`, used until a window has reported bounds.
    saved_place: Option<Place>,
    /// Subscriptions tied to the open window (replaced when it opens again from the tray).
    subs: Vec<Subscription>,
    /// The system tray icon on Windows and Linux (`None` on macOS or if it couldn't be set up).
    tray: Option<Rc<Tray>>,
    /// The close-window dialog, while it is open.
    close_dialog: Option<CloseDialog>,
}

impl WindowState {
    pub(crate) fn new(saved_place: Option<Place>) -> Self {
        WindowState { saved_place, ..Default::default() }
    }

    /// Where to open the window: where it was last seen, else where it was left last launch.
    pub(crate) fn place(&self) -> Option<&Place> {
        self.place.as_ref().or(self.saved_place.as_ref())
    }

    /// The place to save: only one observed this session.
    pub(crate) fn observed_place(&self) -> Option<&Place> {
        self.place.as_ref()
    }

    /// Bind to a newly opened `window`: note its place (starting from the saved one, for the
    /// frame to keep under full screen) and keep `subs` alive while it is open.
    pub(crate) fn attach(&mut self, window: &Window, cx: &App, subs: Vec<Subscription>) {
        self.close_dialog = None;
        let observed = Place::observe(self.place(), window, cx);
        self.place = observed.or(self.place.take());
        self.subs = subs;
    }

    /// The window moved or was resized.
    pub(crate) fn bounds_changed(&mut self, window: &Window, cx: &App) {
        let observed = Place::observe(self.place.as_ref(), window, cx);
        self.place = observed.or(self.place.take());
    }

    pub(crate) fn set_tray(&mut self, tray: Option<Rc<Tray>>) {
        self.tray = tray;
    }

    /// Drop the tray icon at once (on shutdown, so it doesn't linger).
    pub(crate) fn remove_tray(&mut self) {
        self.tray = None;
    }

    /// Whether a system tray host is currently available to show the icon.
    pub(crate) fn tray_available(&self) -> bool {
        self.tray.as_ref().is_some_and(|t| t.is_available())
    }

    /// Update the tray's tooltip and menu after a language change.
    pub(crate) fn refresh_tray_labels(&self) {
        if let Some(tray) = &self.tray {
            tray.set_labels(TrayLabels::current());
        }
    }

    pub(crate) fn close_dialog(&self) -> Option<CloseDialog> {
        self.close_dialog
    }

    pub(crate) fn close_dialog_open(&self) -> bool {
        self.close_dialog.is_some()
    }

    /// Open the dialog asking what closing does.
    pub(crate) fn ask_how_to_close(&mut self) {
        self.close_dialog = Some(CloseDialog { remember: false });
    }

    /// Tick or untick the dialog's "Remember my choice".
    pub(crate) fn toggle_remember(&mut self) {
        if let Some(d) = self.close_dialog.as_mut() {
            d.remember = !d.remember;
        }
    }

    /// Close the dialog, returning it if it was open.
    pub(crate) fn take_close_dialog(&mut self) -> Option<CloseDialog> {
        self.close_dialog.take()
    }
}
