//! System tray icon on Windows and Linux so Flowie can stay running in the
//! background when its window is closed. macOS keeps its Dock icon and menu bar
//! and does not use a tray icon.
//!
//! - Windows: native `Shell_NotifyIconW` on a dedicated message-loop thread.
//! - Linux: StatusNotifierItem + DBusMenu over the session D-Bus (`zbus`),
//!   with no GTK or libappindicator dependency. Availability reflects whether
//!   an `org.kde.StatusNotifierWatcher` with a registered host is present.

use crate::app::i18n::t;

/// User action coming from the system tray icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(any(target_os = "windows", target_os = "linux")), allow(dead_code))]
pub enum TrayEvent {
    Show,
    Quit,
}

/// Localized labels shown in the tray tooltip and context menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayLabels {
    pub tooltip: String,
    pub show: String,
    pub quit: String,
}

impl TrayLabels {
    pub fn current() -> Self {
        Self { tooltip: "Flowie".to_string(), show: t!("tray.show").to_string(), quit: t!("tray.quit").to_string() }
    }
}

/// Running tray icon handle. Dropping it removes the icon and stops its thread.
pub struct Tray {
    inner: imp::TrayImpl,
}

impl Tray {
    /// Start the system tray icon on Windows and Linux. Returns `None` on macOS
    /// or when the platform tray cannot be initialized.
    pub fn start(labels: TrayLabels, tx: futures::channel::mpsc::UnboundedSender<TrayEvent>) -> Option<Self> {
        imp::TrayImpl::start(labels, tx).map(|inner| Self { inner })
    }

    /// Whether a tray host is currently available to show the icon.
    /// When `false` (for example on Linux desktops without a StatusNotifier host),
    /// closing the window falls back to quitting so Flowie never hides unreachable.
    pub fn is_available(&self) -> bool {
        self.inner.is_available()
    }

    /// Update tooltip and menu labels after a UI language change.
    pub fn set_labels(&self, labels: TrayLabels) {
        self.inner.set_labels(labels);
    }
}

/// Downsample an RGBA image (`width` × `height`) to `size` × `size` and pack as
/// network-byte-order ARGB32 (the `IconPixmap` wire format for StatusNotifierItem).
#[cfg(any(target_os = "linux", test))]
fn rgba_to_argb_icon(rgba: &[u8], width: u32, height: u32, size: u32) -> Vec<u8> {
    if width == 0 || height == 0 || size == 0 || rgba.len() < (width as usize) * (height as usize) * 4 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity((size as usize) * (size as usize) * 4);
    for dy in 0..size {
        let y0 = (dy as usize) * (height as usize) / (size as usize);
        let y1 = (((dy + 1) as usize) * (height as usize) / (size as usize)).max(y0 + 1).min(height as usize);
        for dx in 0..size {
            let x0 = (dx as usize) * (width as usize) / (size as usize);
            let x1 = (((dx + 1) as usize) * (width as usize) / (size as usize)).max(x0 + 1).min(width as usize);
            let (mut r, mut g, mut b, mut a, mut count) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * (width as usize) + x) * 4;
                    r += u64::from(rgba[i]);
                    g += u64::from(rgba[i + 1]);
                    b += u64::from(rgba[i + 2]);
                    a += u64::from(rgba[i + 3]);
                    count += 1;
                }
            }
            out.push((a / count) as u8);
            out.push((r / count) as u8);
            out.push((g / count) as u8);
            out.push((b / count) as u8);
        }
    }
    out
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
mod imp {
    use super::{TrayEvent, TrayLabels};

    pub struct TrayImpl;

    impl TrayImpl {
        pub fn start(_labels: TrayLabels, _tx: futures::channel::mpsc::UnboundedSender<TrayEvent>) -> Option<Self> {
            None
        }

        pub fn is_available(&self) -> bool {
            false
        }

        pub fn set_labels(&self, _labels: TrayLabels) {}
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::{null, null_mut};
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread::JoinHandle;

    use futures::channel::mpsc::UnboundedSender;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Shell::{
        Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW,
        GetCursorPos, GetMessageW, GetSystemMetrics, GetWindowLongPtrW, LoadIconW, LoadImageW, PostMessageW, PostQuitMessage,
        RegisterClassW, RegisterWindowMessageW, SetForegroundWindow, SetWindowLongPtrW, TrackPopupMenuEx, TranslateMessage, GWLP_USERDATA,
        HICON, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTCOLOR, MF_STRING, MSG, SM_CXSMICON, SM_CYSMICON, TPM_NONOTIFY, TPM_RETURNCMD,
        TPM_RIGHTBUTTON, WM_APP, WM_CLOSE, WM_DESTROY, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
    };

    use super::{TrayEvent, TrayLabels};

    const TRAY_UID: u32 = 1;
    const WM_TRAY: u32 = WM_APP + 1;
    const WM_UPDATE_LABELS: u32 = WM_APP + 2;
    const CMD_SHOW: usize = 1001;
    const CMD_QUIT: usize = 1002;

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }

    fn copy_tip(dst: &mut [u16; 128], tip: &str) {
        dst.fill(0);
        for (slot, unit) in dst.iter_mut().take(127).zip(OsStr::new(tip).encode_wide()) {
            *slot = unit;
        }
    }

    struct WindowState {
        tx: UnboundedSender<TrayEvent>,
        labels: Arc<Mutex<TrayLabels>>,
        icon: HICON,
        owns_icon: bool,
        taskbar_created: u32,
    }

    impl WindowState {
        fn make_nid(&self, hwnd: HWND) -> NOTIFYICONDATAW {
            // SAFETY: NOTIFYICONDATAW is a plain Win32 C struct where all-zero is valid.
            let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = TRAY_UID;
            nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.uCallbackMessage = WM_TRAY;
            nid.hIcon = self.icon;
            if let Ok(labels) = self.labels.lock() {
                copy_tip(&mut nid.szTip, &labels.tooltip);
            }
            nid
        }

        fn add_icon(&self, hwnd: HWND) -> bool {
            let nid = self.make_nid(hwnd);
            // SAFETY: `nid` is properly sized and valid for the duration of the call.
            unsafe { Shell_NotifyIconW(NIM_ADD, &nid) != 0 }
        }

        fn update_tip(&self, hwnd: HWND) {
            let nid = self.make_nid(hwnd);
            // SAFETY: `nid` is properly sized and valid for the duration of the call.
            unsafe {
                Shell_NotifyIconW(NIM_MODIFY, &nid);
            }
        }

        fn delete_icon(&self, hwnd: HWND) {
            let nid = self.make_nid(hwnd);
            // SAFETY: `nid` identifies this window's tray icon.
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &nid);
            }
        }

        fn show_menu(&self, hwnd: HWND) {
            let (show_label, quit_label) = match self.labels.lock() {
                Ok(l) => (wide(&l.show), wide(&l.quit)),
                Err(_) => return,
            };
            // SAFETY: standard Win32 popup menu lifecycle on the tray thread.
            unsafe {
                let menu = CreatePopupMenu();
                if menu.is_null() {
                    return;
                }
                AppendMenuW(menu, MF_STRING, CMD_SHOW, show_label.as_ptr());
                AppendMenuW(menu, MF_STRING, CMD_QUIT, quit_label.as_ptr());
                let mut pt = POINT { x: 0, y: 0 };
                GetCursorPos(&mut pt);
                // Required by Win32 so clicking outside dismisses the tray menu.
                SetForegroundWindow(hwnd);
                let cmd = TrackPopupMenuEx(menu, TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY, pt.x, pt.y, hwnd, null());
                DestroyMenu(menu);
                PostMessageW(hwnd, WM_NULL, 0, 0);
                if cmd as usize == CMD_SHOW {
                    let _ = self.tx.unbounded_send(TrayEvent::Show);
                } else if cmd as usize == CMD_QUIT {
                    let _ = self.tx.unbounded_send(TrayEvent::Quit);
                }
            }
        }
    }

    // SAFETY: `hwnd`'s `GWLP_USERDATA` holds a `Box<WindowState>` pointer while alive.
    unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut WindowState;
        if !ptr.is_null() {
            let state = &mut *ptr;
            if msg == WM_TRAY {
                let event = (lparam as u32) & 0xFFFF;
                match event {
                    WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                        let _ = state.tx.unbounded_send(TrayEvent::Show);
                    }
                    WM_RBUTTONUP => state.show_menu(hwnd),
                    _ => {}
                }
                return 0;
            }
            if msg == WM_UPDATE_LABELS {
                state.update_tip(hwnd);
                return 0;
            }
            if state.taskbar_created != 0 && msg == state.taskbar_created {
                state.add_icon(hwnd);
                return 0;
            }
            if msg == WM_CLOSE {
                DestroyWindow(hwnd);
                return 0;
            }
            if msg == WM_DESTROY {
                state.delete_icon(hwnd);
                if state.owns_icon && !state.icon.is_null() {
                    DestroyIcon(state.icon);
                }
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(ptr));
                PostQuitMessage(0);
                return 0;
            }
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    pub struct TrayImpl {
        hwnd: usize,
        labels: Arc<Mutex<TrayLabels>>,
        thread: Option<JoinHandle<()>>,
    }

    impl TrayImpl {
        pub fn start(labels: TrayLabels, tx: UnboundedSender<TrayEvent>) -> Option<Self> {
            let shared_labels = Arc::new(Mutex::new(labels));
            let thread_labels = Arc::clone(&shared_labels);
            let (ready_tx, ready_rx) = mpsc::sync_channel::<Option<usize>>(1);
            let thread = std::thread::Builder::new()
                .name("flowie-tray".into())
                .spawn(move || {
                    let class_name = wide("FlowieTrayWindow");
                    let taskbar_msg = wide("TaskbarCreated");
                    // SAFETY: Win32 window class and hidden top-level window creation on this thread.
                    unsafe {
                        let hinst = GetModuleHandleW(null());
                        let mut wc: WNDCLASSW = std::mem::zeroed();
                        wc.lpfnWndProc = Some(wnd_proc);
                        wc.hInstance = hinst;
                        wc.lpszClassName = class_name.as_ptr();
                        RegisterClassW(&wc);

                        // Hidden top-level window (not HWND_MESSAGE) so it receives TaskbarCreated.
                        let hwnd = CreateWindowExW(
                            0,
                            class_name.as_ptr(),
                            class_name.as_ptr(),
                            0,
                            0,
                            0,
                            0,
                            0,
                            null_mut(),
                            null_mut(),
                            hinst,
                            null(),
                        );
                        if hwnd.is_null() {
                            let _ = ready_tx.send(None);
                            return;
                        }
                        let cx = GetSystemMetrics(SM_CXSMICON).max(16);
                        let cy = GetSystemMetrics(SM_CYSMICON).max(16);
                        // Resource ID 1 is embedded by build.rs via winresource (MAKEINTRESOURCEW(1)).
                        let loaded =
                            LoadImageW(hinst, std::ptr::without_provenance::<u16>(1), IMAGE_ICON, cx, cy, LR_DEFAULTCOLOR) as HICON;
                        let (icon, owns_icon) =
                            if !loaded.is_null() { (loaded, true) } else { (LoadIconW(null_mut(), IDI_APPLICATION), false) };
                        let taskbar_created = RegisterWindowMessageW(taskbar_msg.as_ptr());
                        let state = Box::new(WindowState { tx, labels: thread_labels, icon, owns_icon, taskbar_created });
                        let ok = state.add_icon(hwnd);
                        let ptr = Box::into_raw(state);
                        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);
                        if !ok {
                            DestroyWindow(hwnd);
                            let _ = ready_tx.send(None);
                            return;
                        }
                        let _ = ready_tx.send(Some(hwnd as usize));

                        let mut msg: MSG = std::mem::zeroed();
                        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                            TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    }
                })
                .ok()?;

            match ready_rx.recv().ok().flatten() {
                Some(hwnd) => Some(Self { hwnd, labels: shared_labels, thread: Some(thread) }),
                None => {
                    let _ = thread.join();
                    None
                }
            }
        }

        pub fn is_available(&self) -> bool {
            true
        }

        pub fn set_labels(&self, labels: TrayLabels) {
            if let Ok(mut slot) = self.labels.lock() {
                *slot = labels;
            }
            // SAFETY: posting a custom message to the live tray window handle is thread-safe.
            unsafe {
                PostMessageW(self.hwnd as HWND, WM_UPDATE_LABELS, 0, 0);
            }
        }
    }

    impl Drop for TrayImpl {
        fn drop(&mut self) {
            // SAFETY: ask the tray thread to destroy its window and exit its message loop.
            unsafe {
                PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0);
            }
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::Duration;

    use futures::channel::mpsc::UnboundedSender;
    use zbus::blocking::connection::Builder;
    use zbus::blocking::Connection;
    use zbus::zvariant::{OwnedObjectPath, OwnedValue, Structure, Value};

    use super::{rgba_to_argb_icon, TrayEvent, TrayLabels};

    const APP_ID: &str = "flowie.spokey.cn";
    const ITEM_PATH: &str = "/StatusNotifierItem";
    const MENU_PATH: &str = "/Menu";
    const WATCHER_NAME: &str = "org.kde.StatusNotifierWatcher";
    const WATCHER_PATH: &str = "/StatusNotifierWatcher";
    const WATCHER_IFACE: &str = "org.kde.StatusNotifierWatcher";

    const ITEM_SHOW: i32 = 1;
    const ITEM_QUIT: i32 = 2;

    const ICON_PNG: &[u8] = include_bytes!("../../../../assets/icon.png");

    type IconPixmap = (i32, i32, Vec<u8>);

    fn decode_icon_pixmap() -> Vec<IconPixmap> {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(ICON_PNG));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let Ok(mut reader) = decoder.read_info() else {
            return Vec::new();
        };
        let Some(buf_len) = reader.output_buffer_size() else {
            return Vec::new();
        };
        let mut buf = vec![0u8; buf_len];
        let Ok(info) = reader.next_frame(&mut buf) else {
            return Vec::new();
        };
        let bytes = &buf[..info.buffer_size()];
        let rgba: Vec<u8> = match info.color_type {
            png::ColorType::Rgba => bytes.to_vec(),
            png::ColorType::Rgb => bytes.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
            png::ColorType::GrayscaleAlpha => bytes.chunks_exact(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
            png::ColorType::Grayscale => bytes.iter().flat_map(|&g| [g, g, g, 255]).collect(),
            png::ColorType::Indexed => return Vec::new(),
        };
        let size = 32u32;
        let argb = rgba_to_argb_icon(&rgba, info.width, info.height, size);
        if argb.is_empty() {
            Vec::new()
        } else {
            vec![(size as i32, size as i32, argb)]
        }
    }

    struct StatusNotifierItem {
        tx: UnboundedSender<TrayEvent>,
        labels: Arc<Mutex<TrayLabels>>,
        pixmap: Vec<IconPixmap>,
    }

    #[zbus::interface(name = "org.kde.StatusNotifierItem")]
    impl StatusNotifierItem {
        #[zbus(property)]
        fn category(&self) -> &'static str {
            "ApplicationStatus"
        }

        #[zbus(property)]
        fn id(&self) -> &'static str {
            APP_ID
        }

        #[zbus(property)]
        fn title(&self) -> String {
            self.labels.lock().map(|l| l.tooltip.clone()).unwrap_or_else(|_| "Flowie".to_string())
        }

        #[zbus(property)]
        fn status(&self) -> &'static str {
            "Active"
        }

        #[zbus(property)]
        fn window_id(&self) -> i32 {
            0
        }

        #[zbus(property)]
        fn icon_name(&self) -> &'static str {
            APP_ID
        }

        #[zbus(property)]
        fn icon_pixmap(&self) -> Vec<IconPixmap> {
            self.pixmap.clone()
        }

        #[zbus(property)]
        fn overlay_icon_name(&self) -> &'static str {
            ""
        }

        #[zbus(property)]
        fn attention_icon_name(&self) -> &'static str {
            ""
        }

        #[zbus(property)]
        fn item_is_menu(&self) -> bool {
            false
        }

        #[zbus(property)]
        fn menu(&self) -> OwnedObjectPath {
            OwnedObjectPath::try_from(MENU_PATH).expect("valid object path")
        }

        #[zbus(property)]
        fn tool_tip(&self) -> (String, Vec<IconPixmap>, String, String) {
            let title = self.labels.lock().map(|l| l.tooltip.clone()).unwrap_or_else(|_| "Flowie".to_string());
            (APP_ID.to_string(), Vec::new(), title, String::new())
        }

        fn context_menu(&self, _x: i32, _y: i32) {}

        fn activate(&self, _x: i32, _y: i32) {
            let _ = self.tx.unbounded_send(TrayEvent::Show);
        }

        fn secondary_activate(&self, _x: i32, _y: i32) {
            let _ = self.tx.unbounded_send(TrayEvent::Show);
        }

        fn scroll(&self, _delta: i32, _orientation: &str) {}
    }

    type MenuNode = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

    fn string_prop(s: &str) -> OwnedValue {
        OwnedValue::try_from(Value::from(s)).expect("string value")
    }

    fn bool_prop(b: bool) -> OwnedValue {
        OwnedValue::try_from(Value::from(b)).expect("bool value")
    }

    fn menu_item_value(id: i32, label: &str) -> OwnedValue {
        let mut props = HashMap::new();
        props.insert("label".to_string(), string_prop(label));
        props.insert("enabled".to_string(), bool_prop(true));
        props.insert("visible".to_string(), bool_prop(true));
        let children: Vec<OwnedValue> = Vec::new();
        OwnedValue::try_from(Value::from(Structure::from((id, props, children)))).expect("menu item structure")
    }

    struct DBusMenu {
        tx: UnboundedSender<TrayEvent>,
        labels: Arc<Mutex<TrayLabels>>,
        revision: Arc<AtomicU32>,
    }

    #[zbus::interface(name = "com.canonical.dbusmenu")]
    impl DBusMenu {
        #[zbus(property)]
        fn version(&self) -> u32 {
            3
        }

        #[zbus(property)]
        fn text_direction(&self) -> &'static str {
            "ltr"
        }

        #[zbus(property)]
        fn status(&self) -> &'static str {
            "normal"
        }

        #[zbus(property)]
        fn icon_theme_path(&self) -> Vec<String> {
            Vec::new()
        }

        #[zbus(out_args("revision", "layout"))]
        fn get_layout(&self, parent_id: i32, _recursion_depth: i32, _property_names: Vec<String>) -> (u32, MenuNode) {
            let rev = self.revision.load(Ordering::Relaxed);
            let labels = self.labels.lock().map(|l| l.clone()).unwrap_or_else(|_| TrayLabels::current());
            if parent_id != 0 {
                return (rev, (parent_id, HashMap::new(), Vec::new()));
            }
            let mut root_props = HashMap::new();
            root_props.insert("children-display".to_string(), string_prop("submenu"));
            let children = vec![menu_item_value(ITEM_SHOW, &labels.show), menu_item_value(ITEM_QUIT, &labels.quit)];
            (rev, (0, root_props, children))
        }

        fn get_group_properties(&self, ids: Vec<i32>, _property_names: Vec<String>) -> Vec<(i32, HashMap<String, OwnedValue>)> {
            let labels = self.labels.lock().map(|l| l.clone()).unwrap_or_else(|_| TrayLabels::current());
            ids.into_iter()
                .filter_map(|id| {
                    let label = match id {
                        ITEM_SHOW => labels.show.as_str(),
                        ITEM_QUIT => labels.quit.as_str(),
                        _ => return None,
                    };
                    let mut props = HashMap::new();
                    props.insert("label".to_string(), string_prop(label));
                    props.insert("enabled".to_string(), bool_prop(true));
                    props.insert("visible".to_string(), bool_prop(true));
                    Some((id, props))
                })
                .collect()
        }

        fn get_property(&self, id: i32, name: String) -> OwnedValue {
            let labels = self.labels.lock().map(|l| l.clone()).unwrap_or_else(|_| TrayLabels::current());
            match (id, name.as_str()) {
                (ITEM_SHOW, "label") => string_prop(&labels.show),
                (ITEM_QUIT, "label") => string_prop(&labels.quit),
                (_, "enabled" | "visible") => bool_prop(true),
                _ => string_prop(""),
            }
        }

        fn event(&self, id: i32, event_id: String, _data: OwnedValue, _timestamp: u32) {
            if event_id == "clicked" {
                match id {
                    ITEM_SHOW => {
                        let _ = self.tx.unbounded_send(TrayEvent::Show);
                    }
                    ITEM_QUIT => {
                        let _ = self.tx.unbounded_send(TrayEvent::Quit);
                    }
                    _ => {}
                }
            }
        }

        fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
            for (id, event_id, data, timestamp) in events {
                self.event(id, event_id, data, timestamp);
            }
            Vec::new()
        }

        fn about_to_show(&self, _id: i32) -> bool {
            false
        }

        #[zbus(out_args("updatesNeeded", "idErrors"))]
        fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
            (Vec::new(), Vec::new())
        }
    }

    fn watcher_has_host(conn: &Connection) -> bool {
        let has_owner = conn
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "NameHasOwner",
                &(WATCHER_NAME,),
            )
            .ok()
            .and_then(|reply| reply.body().deserialize::<bool>().ok())
            .unwrap_or(false);
        if !has_owner {
            return false;
        }
        conn.call_method(
            Some(WATCHER_NAME),
            WATCHER_PATH,
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(WATCHER_IFACE, "IsStatusNotifierHostRegistered"),
        )
        .ok()
        .and_then(|reply| reply.body().deserialize::<OwnedValue>().ok())
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false)
    }

    fn register_item(conn: &Connection, service_name: &str) -> bool {
        if !watcher_has_host(conn) {
            return false;
        }
        conn.call_method(Some(WATCHER_NAME), WATCHER_PATH, Some(WATCHER_IFACE), "RegisterStatusNotifierItem", &(service_name,)).is_ok()
    }

    enum Cmd {
        UpdateLabels,
        Stop,
    }

    pub struct TrayImpl {
        available: Arc<AtomicBool>,
        labels: Arc<Mutex<TrayLabels>>,
        cmd_tx: mpsc::Sender<Cmd>,
        thread: Option<JoinHandle<()>>,
    }

    impl TrayImpl {
        pub fn start(labels: TrayLabels, tx: UnboundedSender<TrayEvent>) -> Option<Self> {
            let shared_labels = Arc::new(Mutex::new(labels));
            let available = Arc::new(AtomicBool::new(false));
            let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
            let (ready_tx, ready_rx) = mpsc::sync_channel::<bool>(1);

            let thread_labels = Arc::clone(&shared_labels);
            let thread_available = Arc::clone(&available);
            let thread = std::thread::Builder::new()
                .name("flowie-tray".into())
                .spawn(move || {
                    let revision = Arc::new(AtomicU32::new(1));
                    let item = StatusNotifierItem { tx: tx.clone(), labels: Arc::clone(&thread_labels), pixmap: decode_icon_pixmap() };
                    let menu = DBusMenu { tx, labels: thread_labels, revision: Arc::clone(&revision) };
                    let service_name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
                    let conn = match Builder::session()
                        .and_then(|b| b.name(service_name.as_str()))
                        .and_then(|b| b.serve_at(ITEM_PATH, item))
                        .and_then(|b| b.serve_at(MENU_PATH, menu))
                        .and_then(|b| b.build())
                    {
                        Ok(c) => c,
                        Err(_) => {
                            let _ = ready_tx.send(false);
                            return;
                        }
                    };
                    let mut registered = register_item(&conn, &service_name);
                    thread_available.store(registered, Ordering::Relaxed);
                    let _ = ready_tx.send(true);

                    // Periodically re-check watcher/host presence so late-starting panels
                    // or disabled AppIndicator extensions are reflected accurately.
                    loop {
                        match cmd_rx.recv_timeout(Duration::from_secs(5)) {
                            Ok(Cmd::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                            Ok(Cmd::UpdateLabels) => {
                                let rev = revision.fetch_add(1, Ordering::Relaxed) + 1;
                                let _ = conn.emit_signal(None::<&str>, ITEM_PATH, "org.kde.StatusNotifierItem", "NewTitle", &());
                                let _ = conn.emit_signal(None::<&str>, ITEM_PATH, "org.kde.StatusNotifierItem", "NewToolTip", &());
                                let _ = conn.emit_signal(None::<&str>, MENU_PATH, "com.canonical.dbusmenu", "LayoutUpdated", &(rev, 0i32));
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                let has_host = watcher_has_host(&conn);
                                if has_host && !registered {
                                    registered = register_item(&conn, &service_name);
                                } else if !has_host {
                                    registered = false;
                                }
                                thread_available.store(registered, Ordering::Relaxed);
                            }
                        }
                    }
                })
                .ok()?;

            match ready_rx.recv() {
                Ok(true) => Some(Self { available, labels: shared_labels, cmd_tx, thread: Some(thread) }),
                _ => {
                    let _ = thread.join();
                    None
                }
            }
        }

        pub fn is_available(&self) -> bool {
            self.available.load(Ordering::Relaxed)
        }

        pub fn set_labels(&self, labels: TrayLabels) {
            if let Ok(mut slot) = self.labels.lock() {
                *slot = labels;
            }
            let _ = self.cmd_tx.send(Cmd::UpdateLabels);
        }
    }

    impl Drop for TrayImpl {
        fn drop(&mut self) {
            let _ = self.cmd_tx.send(Cmd::Stop);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::rgba_to_argb_icon;

    #[test]
    fn rgba_to_argb_icon_downsamples_and_packs_argb() {
        // 2x2 RGBA image with uniform color (R=10, G=20, B=30, A=255) -> 1x1 ARGB.
        let rgba = [10, 20, 30, 255, 10, 20, 30, 255, 10, 20, 30, 255, 10, 20, 30, 255];
        assert_eq!(rgba_to_argb_icon(&rgba, 2, 2, 1), vec![255, 10, 20, 30]);

        // 2x2 distinct quadrants preserved at 2x2.
        let quad = [
            1, 2, 3, 4, //
            5, 6, 7, 8, //
            9, 10, 11, 12, //
            13, 14, 15, 16,
        ];
        assert_eq!(rgba_to_argb_icon(&quad, 2, 2, 2), vec![4, 1, 2, 3, 8, 5, 6, 7, 12, 9, 10, 11, 16, 13, 14, 15]);

        assert!(rgba_to_argb_icon(&[], 2, 2, 1).is_empty());
        assert!(rgba_to_argb_icon(&rgba, 0, 2, 1).is_empty());
    }
}
