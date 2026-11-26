//! Localization.
//!
//! Every UI string lives in `locales/<code>.json` (found by build.rs, so a new
//! language is one new file). Files are nested JSON; a message's key is its
//! dotted path, e.g. `menu.file.title`. `en` is the source language and the
//! fallback for anything a translation lacks.
//!
//! A message may contain `{name}` placeholders (`{{` / `}}` for literal
//! braces), and may be a plural object `{"one": …, "other": …}` chosen by the
//! `count` argument.
//!
//! ```ignore
//! t!("menu.file.title")                              // &'static str
//! t!("toast.moved", count = 3, target = "Inbox")     // String
//! ```

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use serde_json::Value;

include!(concat!(env!("OUT_DIR"), "/locales.rs"));

/// Look up a message by key: without arguments it gives `&'static str`; with
/// `name = value` arguments it fills the placeholders and gives a `String`.
macro_rules! t {
    ($key:literal) => {
        $crate::app::i18n::text($key)
    };
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::app::i18n::format($key, &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+])
    };
}
pub(crate) use t;

#[derive(Debug)]
pub enum Message {
    Text(String),
    Plural { one: Option<String>, other: String },
}

impl Message {
    fn form(&self, count: Option<f64>) -> &str {
        match self {
            Message::Text(s) => s,
            Message::Plural { one: Some(one), .. } if count == Some(1.0) => one,
            Message::Plural { other, .. } => other,
        }
    }

    /// Every template of this message (one per plural form).
    #[cfg(test)]
    pub fn forms(&self) -> Vec<&str> {
        match self {
            Message::Text(s) => vec![s],
            Message::Plural { one, other } => one.iter().map(String::as_str).chain([other.as_str()]).collect(),
        }
    }
}

pub struct Catalog {
    pub code: &'static str,
    /// The language's own name (`meta.name`), for the language picker.
    pub name: String,
    pub messages: HashMap<String, Message>,
}

impl Catalog {
    pub fn parse(code: &'static str, json: &str) -> Result<Catalog, String> {
        let value: Value = serde_json::from_str(json).map_err(|e| format!("{code}.json: {e}"))?;
        let mut messages = HashMap::new();
        flatten("", &value, &mut messages).map_err(|e| format!("{code}.json: {e}"))?;
        let name = match messages.remove("meta.name") {
            Some(Message::Text(name)) => name,
            _ => return Err(format!("{code}.json: missing meta.name")),
        };
        Ok(Catalog { code, name, messages })
    }
}

fn flatten(prefix: &str, value: &Value, out: &mut HashMap<String, Message>) -> Result<(), String> {
    match value {
        Value::String(s) => {
            out.insert(prefix.to_string(), Message::Text(s.clone()));
        }
        Value::Object(map) if map.contains_key("other") && map.keys().all(|k| k == "one" || k == "other") => {
            let form = |k: &str| map.get(k).map(|v| v.as_str().map(String::from).ok_or(format!("{prefix}.{k}: not a string"))).transpose();
            out.insert(prefix.to_string(), Message::Plural { one: form("one")?, other: form("other")?.unwrap_or_default() });
        }
        Value::Object(map) => {
            for (k, v) in map {
                if k.contains('.') {
                    return Err(format!("key {k:?} must not contain '.'"));
                }
                let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                flatten(&key, v, out)?;
            }
        }
        _ => return Err(format!("{prefix}: expected a string, a plural object or a group")),
    }
    Ok(())
}

pub fn catalogs() -> &'static [Catalog] {
    static CATALOGS: OnceLock<Vec<Catalog>> = OnceLock::new();
    CATALOGS.get_or_init(|| LOCALES.iter().map(|(code, json)| Catalog::parse(code, json).unwrap_or_else(|e| panic!("{e}"))).collect())
}

static ACTIVE: AtomicUsize = AtomicUsize::new(0);

/// (code, own name) of every bundled language, English first.
pub fn languages() -> impl Iterator<Item = (&'static str, &'static str)> {
    catalogs().iter().map(|c| (c.code, c.name.as_str()))
}

/// Switch the UI language: a locale code, or "" to follow macOS.
pub fn set(code: &str) {
    let all = catalogs();
    let ix = if code.is_empty() {
        preferred_languages().iter().find_map(|pref| best_match(all, pref)).unwrap_or(0)
    } else {
        all.iter().position(|c| c.code == code).unwrap_or(0)
    };
    ACTIVE.store(ix, Ordering::Relaxed);
}

/// The code of the active language, e.g. "en" or "zh-CN".
pub fn active() -> &'static str {
    catalogs()[ACTIVE.load(Ordering::Relaxed)].code
}

/// The bundled locale for a BCP 47 tag: exact, else the same base language
/// ("zh-Hans-CN" → "zh-CN").
fn best_match(all: &[Catalog], tag: &str) -> Option<usize> {
    let base = |t: &str| t.split(['-', '_']).next().unwrap_or("").to_ascii_lowercase();
    all.iter().position(|c| c.code.eq_ignore_ascii_case(tag)).or_else(|| all.iter().position(|c| base(c.code) == base(tag)))
}

fn message(key: &str) -> Option<&'static Message> {
    let all = catalogs();
    all[ACTIVE.load(Ordering::Relaxed)].messages.get(key).or_else(|| all[0].messages.get(key))
}

/// A message without arguments (a plural gives its "other" form).
pub fn text(key: &str) -> &'static str {
    match message(key) {
        Some(m) => m.form(None),
        None => {
            debug_assert!(false, "unknown message key {key:?}");
            Box::leak(key.to_string().into_boxed_str())
        }
    }
}

/// A message in English whatever the UI language (search matches both).
pub fn english_text(key: &str) -> &'static str {
    catalogs()[0].messages.get(key).map_or("", |m| m.form(None))
}

/// A message with its placeholders filled; `count` also picks the plural form.
pub fn format(key: &str, args: &[(&str, &dyn Display)]) -> String {
    let rendered: Vec<(&str, String)> = args.iter().map(|(k, v)| (*k, v.to_string())).collect();
    let count = rendered.iter().find(|(k, _)| *k == "count").and_then(|(_, v)| v.parse().ok());
    let template = message(key).map_or(key, |m| m.form(count));
    fill(template, &rendered)
}

/// Replace `{name}` with its argument; unknown names stay as written.
fn fill(template: &str, args: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push_str(&tail[..1]);
            rest = &tail[2..];
        } else if let (true, Some(end)) = (tail.starts_with('{'), tail.find('}')) {
            let name = &tail[1..end];
            match args.iter().find(|(k, _)| *k == name) {
                Some((_, v)) => out.push_str(v),
                None => out.push_str(&tail[..=end]),
            }
            rest = &tail[end + 1..];
        } else {
            out.push_str(&tail[..1]);
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

/// The placeholder names in a template.
#[cfg(test)]
pub fn placeholders(template: &str) -> std::collections::BTreeSet<&str> {
    let mut out = std::collections::BTreeSet::new();
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}']) {
        let tail = &rest[i..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            rest = &tail[2..];
        } else if let (true, Some(end)) = (tail.starts_with('{'), tail.find('}')) {
            out.insert(&tail[1..end]);
            rest = &tail[end + 1..];
        } else {
            rest = &tail[1..];
        }
    }
    out
}

/// System preferred languages, most preferred first.
fn preferred_languages() -> Vec<String> {
    sys_locale::get_locales().collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn english() -> &'static Catalog {
        &catalogs()[0]
    }

    #[test]
    fn placeholders_plurals_and_matching() {
        let args = [("count", "1".to_string()), ("name", "A".to_string())];
        assert_eq!(fill("{count} item for {name} {missing} {{x}}", &args), "1 item for A {missing} {x}");
        assert_eq!(placeholders("{a} {{b}} {c}").into_iter().collect::<Vec<_>>(), ["a", "c"]);
        let m = Message::Plural { one: Some("one".into()), other: "many".into() };
        assert_eq!((m.form(Some(1.0)), m.form(Some(2.0)), m.form(None)), ("one", "many", "many"));
        let all = catalogs();
        assert_eq!(best_match(all, "zh-Hans-CN").map(|i| all[i].code), Some("zh-CN"));
        assert_eq!(best_match(all, "en-GB").map(|i| all[i].code), Some("en"));
        assert_eq!(best_match(all, "fr-FR"), None);
    }

    /// Translations only use keys and placeholders that English has.
    #[test]
    fn translations_match_english() {
        let en = english();
        let mut errors = Vec::new();
        for c in &catalogs()[1..] {
            for (key, msg) in &c.messages {
                let Some(source) = en.messages.get(key) else {
                    errors.push(format!("{}: {key} is not in en.json", c.code));
                    continue;
                };
                let allowed: BTreeSet<&str> = source.forms().into_iter().flat_map(placeholders).collect();
                for form in msg.forms() {
                    let unknown: Vec<_> = placeholders(form).difference(&allowed).copied().collect();
                    if !unknown.is_empty() {
                        errors.push(format!("{}: {key} uses unknown placeholders {unknown:?}", c.code));
                    }
                }
            }
        }
        assert!(errors.is_empty(), "{}", errors.join("\n"));
    }

    /// Every key the code asks for is in en.json, and every English key is used.
    #[test]
    fn keys_used_by_code_exist_and_none_are_unused() {
        let mut used = BTreeSet::new();
        let mut dirs = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let src = std::fs::read_to_string(path).unwrap();
                    // Whole-word `t!` calls only, not the tail of `format!`.
                    // rustfmt may break the line after `t!(`, so skip whitespace before the key.
                    // (Pattern spelled in two halves so this scanner doesn't read itself.)
                    for (i, _) in src.match_indices(concat!("t", "!(")) {
                        if i > 0 && src[..i].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
                            continue;
                        }
                        let Some(rest) = src[i + 3..].trim_start().strip_prefix('"') else { continue };
                        used.insert(rest[..rest.find('"').unwrap()].to_string());
                    }
                }
            }
        }
        // Keys built from tables.
        used.extend(crate::app::keymap::BINDINGS.iter().map(|b| format!("shortcut.{}", b.id)));
        used.extend(crate::app::keymap::GROUPS.iter().map(|g| format!("group.{g}")));
        used.extend(crate::text::inline::SLASH.iter().map(|c| format!("slash.{}", c.id)));
        used.extend(crate::ui::theme::ACCENTS.iter().map(|(name, ..)| format!("settings.appearance.accent.{}", name.to_lowercase())));
        let en = english();
        let missing: Vec<_> = used.iter().filter(|k| !en.messages.contains_key(*k)).collect();
        let mut unused: Vec<_> = en.messages.keys().filter(|k| !used.contains(*k)).collect();
        unused.sort();
        assert!(missing.is_empty(), "keys missing from en.json: {missing:?}");
        assert!(unused.is_empty(), "unused keys in en.json: {unused:?}");
    }

    /// UI text must go through `t!`: flag English literals handed straight to
    /// `.child(…)`, toasts or key labels. Names and key symbols are allowed.
    #[test]
    fn no_raw_ui_text() {
        const ALLOWED: &[&str] = &["Flowie", "Claude Code", "#tag", "fn", "esc"];
        const CALLS: &[&str] = &[".child(\"", "toast(\"", "Text(\"", ".tooltip(\""];
        let mut found = Vec::new();
        let mut dirs = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let src = std::fs::read_to_string(&path).unwrap();
                // The agent protocol (MCP tool descriptions) is for models, not the UI.
                let agent = path.components().any(|c| c.as_os_str() == "agent");
                for (n, line) in src.lines().enumerate() {
                    if line.trim_start().starts_with("//") {
                        continue;
                    }
                    for call in CALLS {
                        for (i, _) in line.match_indices(call) {
                            let rest = &line[i + call.len()..];
                            let Some(end) = rest.find('"') else { continue };
                            let text = &rest[..end];
                            if text.chars().any(|c| c.is_alphabetic()) && !ALLOWED.contains(&text) {
                                found.push(format!("{}:{}: {text:?}", path.file_name().unwrap().to_string_lossy(), n + 1));
                            }
                        }
                    }
                    // A literal alone on its line, e.g. an `if` arm feeding `.child(…)`.
                    let t = line.trim().trim_end_matches(',');
                    let inner = t.get(1..t.len().saturating_sub(1)).unwrap_or("");
                    let plain = !inner.contains('"') && !inner.contains('{');
                    if !agent
                        && t.len() > 2
                        && t.starts_with('"')
                        && t.ends_with('"')
                        && plain
                        && inner.contains(' ')
                        && !ALLOWED.contains(&inner)
                    {
                        found.push(format!("{}:{}: {t}", path.file_name().unwrap().to_string_lossy(), n + 1));
                    }
                }
            }
        }
        assert!(found.is_empty(), "untranslated UI text (use t!):\n{}", found.join("\n"));
    }

    /// Coverage per language; run with `--nocapture` for the report.
    #[test]
    fn translation_coverage() {
        let en = english();
        let total = en.messages.len();
        println!("\n{:<8} {:>9}  {:>8}  name", "locale", "messages", "coverage");
        for c in catalogs() {
            let done = en.messages.keys().filter(|k| c.messages.contains_key(*k)).count();
            println!("{:<8} {:>4}/{:<4}  {:>7.1}%  {}", c.code, done, total, 100.0 * done as f64 / total as f64, c.name);
            let mut missing: Vec<_> = en.messages.keys().filter(|k| !c.messages.contains_key(*k)).collect();
            missing.sort();
            for k in missing {
                println!("    missing {k}");
            }
        }
    }
}
