//! What's New: a read-only page, like Trash, built from the release notes
//! compiled into the app. `CHANGELOG.md` is the English source; translations
//! live in `locales/whats-new/<code>.md` with the same `## [x.y.z] - date`
//! sections, and each version shows in the UI language when its translation
//! has it, else in English.

use std::cmp::Ordering;
use std::collections::HashSet;

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, MouseButton, StyledText};

use crate::i18n::{self, t};
use crate::outline::{BlockSel, Outline, Target, WhatsNew as WhatsNewAction};
use crate::rich::{self, Piece, Style};
use crate::ui::button::{button, Variant};

const CHANGELOG: &str = include_str!("../../../CHANGELOG.md");
include!(concat!(env!("OUT_DIR"), "/whats_new.rs"));

const RELEASES_URL: &str = "https://github.com/wesleyel/flowie/releases/tag/v";

/// One version's notes.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    /// "1.0.0-rc.3", or "Unreleased".
    pub version: String,
    pub date: String,
    pub body: String,
}

/// The `## [version] - date` sections of a changelog, in file order (newest
/// first). Link definitions at the end are left out.
fn sections(md: &str) -> Vec<Release> {
    let mut out: Vec<Release> = Vec::new();
    for line in md.lines() {
        if let Some(rest) = line.strip_prefix("## [") {
            if let Some((version, after)) = rest.split_once(']') {
                let date = after.trim().trim_start_matches('-').trim().to_string();
                out.push(Release { version: version.to_string(), date, body: String::new() });
                continue;
            }
        }
        let Some(r) = out.last_mut() else { continue };
        // `[1.0.0]: https://…` link definitions end the file.
        if line.starts_with('[') && line.contains("]: ") {
            continue;
        }
        r.body.push_str(line);
        r.body.push('\n');
    }
    for r in &mut out {
        r.body = r.body.trim().to_string();
    }
    out
}

/// Semantic version order: 1.0.0-rc.2 < 1.0.0-rc.10 < 1.0.0 < 1.1.0. `None`
/// for something that isn't a version ("Unreleased").
pub fn compare(a: &str, b: &str) -> Option<Ordering> {
    fn parse(v: &str) -> Option<([u64; 3], Option<Vec<String>>)> {
        let v = v.trim().trim_start_matches('v');
        let (core, pre) = match v.split_once('-') {
            Some((c, p)) => (c, Some(p.split('.').map(String::from).collect())),
            None => (v, None),
        };
        let mut nums = core.split('.').map(|n| n.parse::<u64>().ok());
        let out = [nums.next()??, nums.next()??, nums.next()??];
        Some((out, pre))
    }
    let (a, b) = (parse(a)?, parse(b)?);
    Some(a.0.cmp(&b.0).then_with(|| match (&a.1, &b.1) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => {
            for (p, q) in x.iter().zip(y) {
                let o = match (p.parse::<u64>(), q.parse::<u64>()) {
                    (Ok(m), Ok(n)) => m.cmp(&n),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => p.cmp(q),
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
            x.len().cmp(&y.len())
        }
    }))
}

/// The translation for a locale code, if bundled.
fn translation(code: &str) -> Option<&'static str> {
    WHATS_NEW.iter().find(|(c, _)| *c == code).map(|(_, md)| *md)
}

/// The releases to show, newest first, each in `code`'s language when
/// translated. `[Unreleased]` only in development builds.
pub fn releases(code: &str) -> Vec<Release> {
    let local = translation(code).map(sections).unwrap_or_default();
    sections(CHANGELOG)
        .into_iter()
        .filter(|r| cfg!(debug_assertions) || compare(&r.version, "0.0.0").is_some())
        .map(|r| {
            local
                .iter()
                .find(|l| l.version == r.version && !l.body.is_empty())
                .cloned()
                .map_or(r.clone(), |l| Release { date: r.date.clone(), ..l })
        })
        .collect()
}

/// Released versions a translation has no section for (the i18n report).
#[cfg(test)]
pub fn untranslated(code: &str) -> Vec<String> {
    let local: HashSet<String> = translation(code).map(sections).unwrap_or_default().into_iter().map(|r| r.version).collect();
    sections(CHANGELOG)
        .into_iter()
        .filter(|r| compare(&r.version, "0.0.0").is_some() && !local.contains(&r.version))
        .map(|r| r.version)
        .collect()
}

/// What updating from `seen` to `running` means for the page.
#[derive(Debug, PartialEq, Eq)]
pub enum OnLaunch {
    /// Show it: there are notes for a version after `seen`.
    Show,
    /// Record `running` without showing anything (first launch, a downgrade,
    /// the same version, or no notes in between).
    Record,
}

pub fn on_launch(seen: Option<&str>, running: &str) -> OnLaunch {
    let Some(seen) = seen else { return OnLaunch::Record };
    if compare(running, seen) != Some(Ordering::Greater) {
        return OnLaunch::Record;
    }
    let notes = sections(CHANGELOG).into_iter().any(|r| {
        compare(&r.version, seen) == Some(Ordering::Greater)
            && compare(&r.version, running) != Some(Ordering::Greater)
            && !r.body.is_empty()
    });
    if notes {
        OnLaunch::Show
    } else {
        OnLaunch::Record
    }
}

/// The open What's New page.
pub struct WhatsNewPage {
    /// Versions after this are marked New (the version seen before this update).
    pub new_after: Option<String>,
    /// Older versions the user unfolded.
    pub open: HashSet<String>,
    /// Where the user was: put back on close.
    return_to: Option<(Target, usize)>,
    block: Option<BlockSel>,
}

/// The version this build is: Cargo.toml stays at `X.Y.Z` through the
/// release candidates, so it is the newest changelog section for `X.Y.Z`
/// (a release commit moves its notes under `## [X.Y.Z-rc.N]`).
pub fn running_version() -> &'static str {
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    VERSION.get_or_init(|| release_for(env!("CARGO_PKG_VERSION"), CHANGELOG))
}

fn release_for(cargo: &str, changelog: &str) -> String {
    let core = |v: &str| v.split('-').next().unwrap_or(v).to_string();
    sections(changelog)
        .into_iter()
        .map(|r| r.version)
        .filter(|v| compare(v, "0.0.0").is_some() && core(v) == core(cargo))
        .max_by(|a, b| compare(a, b).unwrap_or(Ordering::Equal))
        .unwrap_or_else(|| cargo.to_string())
}

impl Outline {
    pub fn whats_new(&mut self, _: &WhatsNewAction, _: &mut gpui::Window, cx: &mut Context<Self>) {
        let new_after = self.config.seen_version.clone();
        self.open_whats_new(new_after, cx);
    }

    /// Show the page; versions after `new_after` are marked New.
    pub fn open_whats_new(&mut self, new_after: Option<String>, cx: &mut Context<Self>) {
        if self.whats_new.is_some() {
            return;
        }
        self.trash = None;
        self.palette = None;
        self.inline = None;
        self.calendar = None;
        self.menu = None;
        self.popover.close();
        let return_to = self.edit.take().map(|e| (e.target, e.caret()));
        let block = self.block.take();
        self.whats_new = Some(WhatsNewPage { new_after, open: HashSet::new(), return_to, block });
        self.page_scroll = gpui::ScrollHandle::new();
        // Written when shown, not when closed: a crash while it's open doesn't show it again.
        self.mark_whats_new_seen();
        cx.notify();
    }

    pub fn mark_whats_new_seen(&mut self) {
        if self.config.seen_version.as_deref() != Some(running_version()) {
            self.config.seen_version = Some(running_version().to_string());
            self.config.save(&self.config_path);
        }
    }

    /// Close the page; `restore` puts the caret back where it was (Esc, ←),
    /// not when the user went somewhere else.
    pub fn close_whats_new(&mut self, restore: bool, cx: &mut Context<Self>) -> bool {
        let Some(p) = self.whats_new.take() else { return false };
        if restore {
            match p.return_to.filter(|(t, _)| matches!(t, Target::Text(id) | Target::Note(id) if self.tree.contains(*id))) {
                Some((target, offset)) => self.start_edit(target, offset, cx),
                None => self.block = p.block.filter(|b| self.tree.contains(b.anchor) && self.tree.contains(b.head)),
            }
        }
        cx.notify();
        true
    }

    /// At launch: open the page after an update, or just record the version.
    pub fn whats_new_on_launch(&mut self, background: bool, cx: &mut Context<Self>) {
        match on_launch(self.config.seen_version.as_deref(), running_version()) {
            OnLaunch::Show if self.config.whats_new_after_updates => {
                let seen = self.config.seen_version.clone();
                if background {
                    // Nobody is looking: show it the first time the window is.
                    self.whats_new_pending = Some(seen);
                } else {
                    self.open_whats_new(seen, cx);
                }
            }
            _ => self.mark_whats_new_seen(),
        }
    }

    /// The window was shown or activated: a page held back by a background launch opens now.
    pub fn whats_new_window_shown(&mut self, cx: &mut Context<Self>) {
        if let Some(seen) = self.whats_new_pending.take() {
            self.open_whats_new(seen, cx);
        }
    }

    pub fn render_whats_new(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme;
        let page = self.whats_new.as_ref();
        let new_after = page.and_then(|p| p.new_after.clone());
        let open = page.map(|p| p.open.clone()).unwrap_or_default();
        let (font, color) = self.base_style(window, false);
        let style = Style { theme: &self.theme, font, color, completed: false, highlights: &[], code: false };
        let mut sections: Vec<AnyElement> = Vec::new();
        let mut first = true;
        for r in releases(i18n::active()) {
            let unreleased = compare(&r.version, "0.0.0").is_none();
            let is_new = unreleased || new_after.as_deref().is_some_and(|seen| compare(&r.version, seen) == Some(Ordering::Greater));
            // New versions and the newest one show; older ones fold.
            let expanded = is_new || first || open.contains(&r.version);
            first = false;
            let version = r.version.clone();
            let title = if unreleased { t!("whats_new.unreleased").to_string() } else { r.version.clone() };
            let header = div()
                .id(gpui::SharedString::from(format!("release-{version}")))
                .flex()
                .items_center()
                .gap_2()
                .pt_4()
                .pb_1()
                .cursor_pointer()
                .child(crate::icons::icon(if expanded { "chevron-down" } else { "chevron-right" }, crate::icons::SM, theme.muted))
                .child(div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child(title))
                .children(is_new.then(|| {
                    div()
                        .px(px(6.))
                        .rounded(px(4.))
                        .bg(theme.accent)
                        .text_color(theme.on_accent)
                        .text_size(px(11.))
                        .child(t!("whats_new.new"))
                }))
                .child(div().flex_1())
                .child(div().text_size(px(12.)).text_color(theme.muted).child(r.date.clone()))
                .children((!unreleased).then(|| {
                    let url = format!("{RELEASES_URL}{version}");
                    button(theme, gpui::SharedString::from(format!("release-link-{version}")), Variant::Link)
                        .child(t!("whats_new.release_page"))
                        .on_click(cx.listener(move |_, _, _, cx| cx.open_url(&url)))
                }))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(p) = this.whats_new.as_mut() {
                        if !p.open.remove(&version) {
                            p.open.insert(version.clone());
                        }
                    }
                    cx.notify();
                }));
            let mut body: Vec<AnyElement> = Vec::new();
            if expanded {
                for block in blocks(&r.body) {
                    body.push(match block {
                        Block::Heading(h) => div()
                            .pt_2()
                            .pb_1()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.muted)
                            .child(h)
                            .into_any_element(),
                        Block::Item(text) => div()
                            .flex()
                            .items_start()
                            .gap_2()
                            .py(px(2.))
                            .child(div().flex_none().mt(px(9.)).size(px(5.)).rounded_full().bg(theme.bullet))
                            .child(div().flex_1().min_w_0().child(rich_line(&text, &style, cx)))
                            .into_any_element(),
                        Block::Para(text) => div().py(px(2.)).child(rich_line(&text, &style, cx)).into_any_element(),
                    });
                }
            }
            sections.push(div().border_b_1().border_color(theme.divider).pb_2().child(header).children(body).into_any_element());
        }
        let content = div()
            .id("whats-new")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.page_scroll)
            .px(px(crate::outline::PAD_X + 24.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pt_2()
                    .child(div().text_size(px(crate::view::TITLE_SIZE)).font_weight(FontWeight::BOLD).child(t!("whats_new.title")))
                    .child(
                        button(theme, "whats-new-back", Variant::Plain)
                            .child(crate::icons::icon("arrow-left", crate::icons::SM, Variant::Plain.fg(theme)))
                            .child(t!("common.back"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.close_whats_new(true, cx);
                            })),
                    ),
            )
            .child(div().max_w(px(720.)).pb_8().children(sections));
        crate::ui::scrollbar::area("whats-new-bar", &self.page_scroll, theme, content).size_full().into_any_element()
    }
}

/// One line of notes with Flowie's inline markup; links open in the browser.
fn rich_line(text: &str, style: &Style, cx: &mut Context<Outline>) -> AnyElement {
    let Some(Piece::Text { text, runs, clicks, .. }) =
        rich::build(text, false, style).into_iter().find(|p| matches!(p, Piece::Text { .. }))
    else {
        return div().into_any_element();
    };
    let styled = StyledText::new(text).with_runs(runs);
    let layout = styled.layout().clone();
    div()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |_, ev: &gpui::MouseDownEvent, _, cx| {
                let Ok(ix) = layout.index_for_position(ev.position) else { return };
                if let Some((_, rich::Click::Link(url))) = clicks.iter().find(|(r, _)| r.contains(&ix)) {
                    if url.starts_with("http") {
                        cx.open_url(url);
                    }
                }
            }),
        )
        .child(styled)
        .into_any_element()
}

#[derive(Debug, PartialEq)]
enum Block {
    Heading(String),
    Item(String),
    Para(String),
}

/// Join a wrapped line: with a space, except between two CJK characters.
fn join(s: &mut String, next: &str) {
    let wide = |c: char| c as u32 >= 0x2E80;
    if !(s.chars().next_back().is_some_and(wide) && next.chars().next().is_some_and(wide)) {
        s.push(' ');
    }
    s.push_str(next);
}

/// A section's Markdown as headings, list items (wrapped lines joined) and paragraphs.
fn blocks(body: &str) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(h) = trimmed.strip_prefix("### ") {
            out.push(Block::Heading(h.to_string()));
        } else if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            out.push(Block::Item(item.trim().to_string()));
        } else if line.starts_with(' ') {
            match out.last_mut() {
                Some(Block::Item(s) | Block::Para(s)) => join(s, trimmed),
                _ => out.push(Block::Para(trimmed.to_string())),
            }
        } else {
            match out.last_mut() {
                Some(Block::Para(s)) => join(s, trimmed),
                _ => out.push(Block::Para(trimmed.to_string())),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_order_like_semver() {
        assert_eq!(compare("1.0.0-rc.2", "1.0.0-rc.10"), Some(Ordering::Less));
        assert_eq!(compare("1.0.0-rc.4", "1.0.0"), Some(Ordering::Less));
        assert_eq!(compare("1.1.0", "1.0.9"), Some(Ordering::Greater));
        assert_eq!(compare("1.0.0", "1.0.0"), Some(Ordering::Equal));
        assert_eq!(compare("Unreleased", "1.0.0"), None);
    }

    #[test]
    fn the_changelog_parses() {
        let all = sections(CHANGELOG);
        assert!(all.iter().any(|r| r.version == "1.0.0-rc.1" && r.date == "2026-11-18" && r.body.contains("###")));
        assert!(all.iter().all(|r| !r.body.contains("]: https://github.com")));
        // Newest first.
        let released: Vec<&Release> = all.iter().filter(|r| compare(&r.version, "0.0.0").is_some()).collect();
        assert!(released.windows(2).all(|w| compare(&w[0].version, &w[1].version) == Some(Ordering::Greater)));
    }

    #[test]
    fn blocks_join_wrapped_items() {
        let b = blocks("### Added\n\n- One **thing**\n  that wraps.\n- Two\n\nA paragraph.");
        assert_eq!(
            b,
            [
                Block::Heading("Added".into()),
                Block::Item("One **thing** that wraps.".into()),
                Block::Item("Two".into()),
                Block::Para("A paragraph.".into())
            ]
        );
        assert_eq!(blocks("- 中文\n  换行"), [Block::Item("中文换行".into())]);
    }

    #[test]
    fn the_running_version_comes_from_the_changelog() {
        let md = "## [Unreleased]\n\n## [1.0.0-rc.3] - x\n- a\n## [1.0.0-rc.2] - x\n- b\n## [0.9.0] - x\n";
        assert_eq!(release_for("1.0.0", md), "1.0.0-rc.3");
        assert_eq!(release_for("1.1.0", md), "1.1.0");
        assert_eq!(release_for("1.0.0", &format!("## [1.0.0] - x\n{md}")), "1.0.0");
    }

    #[test]
    fn opens_only_after_a_real_update() {
        assert_eq!(on_launch(None, "1.0.0-rc.3"), OnLaunch::Record);
        assert_eq!(on_launch(Some("1.0.0-rc.3"), "1.0.0-rc.3"), OnLaunch::Record);
        assert_eq!(on_launch(Some("1.0.0-rc.3"), "1.0.0-rc.2"), OnLaunch::Record);
        assert_eq!(on_launch(Some("1.0.0-rc.1"), "1.0.0-rc.3"), OnLaunch::Show);
        // Nothing written for the versions in between.
        assert_eq!(on_launch(Some("1.0.0-rc.3"), "1.0.0-rc.3.1"), OnLaunch::Record);
    }

    #[test]
    fn translations_fall_back_per_version() {
        // Every version shows, in some language.
        assert_eq!(releases("zh-CN").len(), releases("en").len());
        assert_eq!(releases("xx").iter().map(|r| &r.body).collect::<Vec<_>>(), releases("en").iter().map(|r| &r.body).collect::<Vec<_>>());
        // A report, not a failure: list what each translation lacks.
        for (code, _) in WHATS_NEW {
            let missing = untranslated(code);
            if !missing.is_empty() {
                eprintln!("whats-new/{code}.md has no notes for: {}", missing.join(", "));
            }
        }
    }
}
