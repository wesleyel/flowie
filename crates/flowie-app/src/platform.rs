//! What differs between macOS, Windows and Linux, in one place: where data
//! lives, how keys are written, fonts, the menu bar and the OS's own names.
//! The rest of the app asks this module instead of testing `target_os`.

use std::path::PathBuf;

use crate::i18n::t;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Mac,
    Windows,
    /// Linux and the other Unix desktops GPUI supports.
    Linux,
}

pub const OS: Os = if cfg!(target_os = "macos") {
    Os::Mac
} else if cfg!(target_os = "windows") {
    Os::Windows
} else {
    Os::Linux
};

/// Windows and Linux have no global menu bar, so the window draws its own.
pub const MENU_BAR_IN_WINDOW: bool = !matches!(OS, Os::Mac);

/// The code font. `monospace` is fontconfig's alias for the desktop's choice.
pub const MONO_FONT: &str = match OS {
    Os::Mac => "Menlo",
    Os::Windows => "Consolas",
    Os::Linux => "monospace",
};

/// The signed-in user's full name as the OS knows it (the default author
/// name), else the login name, else empty.
pub fn full_user_name() -> String {
    #[cfg(unix)]
    {
        // SAFETY: getpwuid returns a pointer to static storage or null; its fields are C strings.
        let gecos = unsafe {
            let pw = libc::getpwuid(libc::getuid());
            if pw.is_null() || (*pw).pw_gecos.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr((*pw).pw_gecos).to_string_lossy().into_owned()
            }
        };
        // GECOS is "Full Name,room,phone,…" on Linux; just the name on macOS.
        let name = gecos.split(',').next().unwrap_or("").trim().to_string();
        if !name.is_empty() {
            return name;
        }
    }
    std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default()
}

/// The OS's name, as in "Follow macOS".
pub fn name() -> &'static str {
    match OS {
        Os::Mac => "macOS",
        Os::Windows => "Windows",
        Os::Linux => "Linux",
    }
}

/// The file manager's name, as in "Show data in Finder".
pub fn file_manager() -> &'static str {
    match OS {
        Os::Mac => t!("settings.about.file_manager_macos"),
        Os::Windows => t!("settings.about.file_manager_windows"),
        Os::Linux => t!("settings.about.file_manager_other"),
    }
}

/// The OS and its version for feedback mail, e.g. "macOS 15.1" or "Windows 11 (10.0.22631)".
pub fn os_version() -> String {
    #[cfg(target_os = "macos")]
    let version = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|v| format!("macOS {}", v.trim()));
    #[cfg(target_os = "windows")]
    let version = {
        let v = windows_version::OsVersion::current();
        // Windows 11 still reports 10.0; build 22000 is its first release.
        let release = match (v.major, v.build) {
            (10, b) if b >= 22000 => "Windows 11",
            (10, _) => "Windows 10",
            _ => "Windows",
        };
        Some(format!("{release} ({}.{}.{})", v.major, v.minor, v.build))
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let version = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")).map(|v| format!("Linux ({})", v.trim_matches('"'))));
    version.unwrap_or_else(|| name().to_string())
}

// ------------------------------------------------------------------ data

/// Inside the App Sandbox (Mac App Store build): the container's `Data`
/// directory. Built from the real home directory and the container id, so
/// `flowie mcp` started by an agent from a terminal (whose `HOME` is the
/// user's real home) finds the same database and socket as the app.
#[cfg(target_os = "macos")]
pub fn sandbox_container() -> Option<PathBuf> {
    let id = std::env::var("APP_SANDBOX_CONTAINER_ID").ok().filter(|s| !s.is_empty())?;
    // SAFETY: getpwuid returns a pointer to static storage or null; pw_dir is a C string.
    let real_home = unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_dir.is_null() {
            return None;
        }
        std::ffi::CStr::from_ptr((*pw).pw_dir).to_string_lossy().into_owned()
    };
    Some(PathBuf::from(real_home).join("Library/Containers").join(id).join("Data"))
}

#[cfg(not(target_os = "macos"))]
pub fn sandbox_container() -> Option<PathBuf> {
    None
}

/// The folder for the outline and settings:
/// `~/Library/Application Support/Flowie` (inside the container on the
/// App Store build), `%LOCALAPPDATA%\Flowie` (a database doesn't belong in
/// the roaming profile), or `$XDG_DATA_HOME/flowie`.
pub fn data_dir() -> PathBuf {
    if let Some(container) = sandbox_container() {
        return container.join("Library/Application Support/Flowie");
    }
    let dir = match OS {
        Os::Mac => dirs::data_dir().map(|d| d.join("Flowie")),
        Os::Windows => dirs::data_local_dir().map(|d| d.join("Flowie")),
        Os::Linux => dirs::data_dir().map(|d| d.join("flowie")),
    };
    dir.unwrap_or_else(|| PathBuf::from("."))
}

/// `s` quoted as one argument for the shell a user pastes commands into:
/// POSIX shells on macOS and Linux; PowerShell or cmd on Windows, where
/// double quotes work in both.
pub fn shell_quote(s: &str) -> String {
    let plain = !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "/\\:._-+=@%,".contains(c));
    if plain {
        return s.to_string();
    }
    match OS {
        Os::Windows => format!("\"{}\"", s.replace('"', "\"\"")),
        Os::Mac | Os::Linux => format!("'{}'", s.replace('\'', r"'\''")),
    }
}

// ------------------------------------------------------------------ keys

/// Modifiers in the order each platform writes them: ⌃⌥⇧⌘ on macOS,
/// Ctrl+Alt+Shift+Win elsewhere. `cmd` stands for the platform key, which
/// GPUI also spells `super`, `win` or `platform`.
const MODIFIERS: [&str; 5] = ["ctrl", "alt", "shift", "cmd", "fn"];

/// A keystroke such as "cmd-shift-up" or "ctrl--" split into its modifiers
/// (in display order, platform key as `cmd`) and its key.
pub fn split_stroke(stroke: &str) -> (Vec<&'static str>, &str) {
    let (mods, key) = match stroke.strip_suffix('-').filter(|m| m.is_empty() || m.ends_with('-')) {
        Some(mods) => (mods, "-"),
        None => stroke.rsplit_once('-').unwrap_or(("", stroke)),
    };
    let given: Vec<&str> = mods
        .split('-')
        .map(|m| match m {
            "super" | "win" | "platform" => "cmd",
            m => m,
        })
        .collect();
    (MODIFIERS.into_iter().filter(|m| given.contains(m)).collect(), key)
}

/// A modifier as prose: "⌥" on macOS, "Alt" elsewhere.
pub fn modifier_name(modifier: &str) -> &'static str {
    modifier_name_on(OS, modifier)
}

fn modifier_name_on(os: Os, modifier: &str) -> &'static str {
    match (os, modifier) {
        (Os::Mac, "ctrl") => "⌃",
        (Os::Mac, "alt") => "⌥",
        (Os::Mac, "shift") => "⇧",
        (Os::Mac, "cmd") => "⌘",
        (Os::Mac, _) => "fn",
        (_, "ctrl") => "Ctrl",
        (_, "alt") => "Alt",
        (_, "shift") => "Shift",
        (Os::Windows, "cmd") => "Win",
        (_, "cmd") => "Super",
        _ => "Fn",
    }
}

/// A key as prose: arrows as arrows, named keys in words, the rest uppercased.
pub fn key_name(key: &str) -> String {
    key_name_on(OS, key)
}

fn key_name_on(os: Os, key: &str) -> String {
    match key {
        "up" => "↑",
        "down" => "↓",
        "left" => "←",
        "right" => "→",
        "enter" => "Enter",
        "backspace" => "Backspace",
        // macOS calls Backspace "delete"; its forward delete is ⌦.
        "delete" if os == Os::Mac => "⌦",
        "delete" => "Delete",
        "tab" => "Tab",
        "space" => "Space",
        "escape" => "Esc",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PgUp",
        "pagedown" => "PgDn",
        "mouse-back" => t!("settings.shortcuts.mouse_back"),
        "mouse-forward" => t!("settings.shortcuts.mouse_forward"),
        k => return k.to_uppercase(),
    }
    .to_string()
}

/// Keys ("cmd-shift-p", or a sequence "ctrl-k ctrl-c") as prose for hints,
/// toasts and the tutorial: "⇧⌘P" on macOS, "Ctrl+Shift+P" elsewhere.
pub fn key_text(keys: &str) -> String {
    key_text_on(OS, keys)
}

fn key_text_on(os: Os, keys: &str) -> String {
    let strokes = keys.split_whitespace().map(|stroke| {
        if stroke.starts_with("mouse-") {
            return key_name_on(os, stroke);
        }
        let (mods, key) = split_stroke(stroke);
        let mut parts: Vec<String> = mods.iter().map(|m| modifier_name_on(os, m).to_string()).collect();
        parts.push(key_name_on(os, key));
        parts.join(if os == Os::Mac { "" } else { "+" })
    });
    strokes.collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strokes_split_into_ordered_modifiers_and_key() {
        assert_eq!(split_stroke("shift-cmd-up"), (vec!["shift", "cmd"], "up"));
        assert_eq!(split_stroke("ctrl--"), (vec!["ctrl"], "-"));
        assert_eq!(split_stroke("-"), (vec![], "-"));
        assert_eq!(split_stroke("win-alt-x"), (vec!["alt", "cmd"], "x"));
        assert_eq!(split_stroke("f3"), (vec![], "f3"));
    }

    #[test]
    fn keys_read_as_each_platform_writes_them() {
        assert_eq!(key_text_on(Os::Mac, "cmd-shift-p"), "⇧⌘P");
        assert_eq!(key_text_on(Os::Windows, "ctrl-shift-p"), "Ctrl+Shift+P");
        assert_eq!(key_text_on(Os::Linux, "super-enter"), "Super+Enter");
        assert_eq!(key_text_on(Os::Windows, "alt-left"), "Alt+←");
        assert_eq!(key_text_on(Os::Mac, "cmd-shift-delete"), "⇧⌘⌦");
        assert_eq!(key_text_on(Os::Windows, "ctrl-k ctrl-c"), "Ctrl+K Ctrl+C");
    }

    #[test]
    fn shell_quoting_keeps_plain_paths_and_quotes_the_rest() {
        assert_eq!(shell_quote("/usr/local/bin/flowie"), "/usr/local/bin/flowie");
        let spaced = shell_quote("/Users/a b/flowie");
        assert!(spaced.starts_with(['\'', '"']) && spaced.contains("a b"), "{spaced}");
    }
}
