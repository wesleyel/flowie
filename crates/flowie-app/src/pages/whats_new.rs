//! What's New: a temporary editable view built from the release notes
//! compiled into the app. `CHANGELOG.md` is the English source; translations
//! live in the root `CHANGELOG.zh-CN.md` with the same `## [x.y.z] - date`
//! sections. Each version shows in the UI language when translated, else in
//! English.

use flowie_core::{flatten, BlockKind, Item, Tree, ROOT};
use gpui::{div, prelude::*, px, AnyElement, Context, ScrollHandle};
use std::cmp::Ordering;

use super::OutlineSelection;
use crate::app::i18n::{self, t};
use crate::outline::{Outline, Target, WhatsNew as WhatsNewAction, PAD_X};

const CHANGELOG: &str = include_str!("../../../../CHANGELOG.md");
const CHANGELOG_ZH: &str = include_str!("../../../../CHANGELOG.zh-CN.md");

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
    (code == "zh-CN").then_some(CHANGELOG_ZH)
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

pub(crate) struct VolatilePage {
    notes: WhatsNewPage,
    scroll: ScrollHandle,
    selection: OutlineSelection,
}

/// The open What's New page.
pub struct WhatsNewPage {
    /// Versions after this are marked New (the version seen before this update).
    pub new_after: Option<String>,
    pub tree: Tree,
    locale: &'static str,
}

fn notes_tree(locale: &str, new_after: Option<&str>) -> Tree {
    let mut tree = Tree::new();
    for (index, release) in releases(locale).into_iter().enumerate() {
        let unreleased = release.version == "Unreleased";
        let is_new = unreleased || new_after.is_some_and(|seen| compare(&release.version, seen) == Some(Ordering::Greater));
        let mut title = if unreleased { t!("whats_new.unreleased").to_string() } else { release.version };
        if !release.date.is_empty() {
            title.push_str(&format!(" · {}", release.date));
        }
        if is_new {
            title.push_str(&format!(" · {}", t!("whats_new.new")));
        }
        let mut version = Item::new(&title);
        version.collapsed = index > 0 && !is_new;
        let mut group: Option<usize> = None;
        for block in blocks(&release.body) {
            match block {
                Block::Heading(text) => {
                    group = Some(version.children.len());
                    version.children.push(Item { kind: BlockKind::H2, ..Item::new(&text) });
                }
                Block::Para(text) | Block::Item(text) => {
                    let children = match group {
                        Some(index) => &mut version.children[index].children,
                        None => &mut version.children,
                    };
                    children.push(Item::new(&text));
                }
            }
        }
        tree.insert_items(ROOT, index, &[version]);
    }
    tree.take_journal();
    tree
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
    pub fn notes_page(&self) -> Option<&WhatsNewPage> {
        self.volatile_page.as_ref().map(|page| &page.notes)
    }

    pub fn notes_page_mut(&mut self) -> Option<&mut WhatsNewPage> {
        self.volatile_page.as_mut().map(|page| &mut page.notes)
    }

    pub fn whats_new(&mut self, _: &WhatsNewAction, _: &mut gpui::Window, cx: &mut Context<Self>) {
        let new_after = self.config.seen_version.clone();
        self.open_whats_new(new_after, cx);
    }

    /// Show the page; versions after `new_after` are marked New.
    pub fn open_whats_new(&mut self, new_after: Option<String>, cx: &mut Context<Self>) {
        if self.volatile_page.is_some() {
            return;
        }
        self.close_trash(true, cx);
        let locale = i18n::active();
        let tree = notes_tree(locale, new_after.as_deref());
        let selection = self.suspend_outline_selection();
        self.volatile_page = Some(VolatilePage { notes: WhatsNewPage { new_after, locale, tree }, scroll: ScrollHandle::new(), selection });
        // Written when shown, not when closed: a crash while it's open doesn't show it again.
        self.mark_whats_new_seen();
        cx.notify();
    }

    pub(super) fn close_whats_new(&mut self, restore: bool, cx: &mut Context<Self>) -> bool {
        let Some(page) = self.volatile_page.take() else { return false };
        self.finish_auxiliary_page(page.selection, restore, cx);
        true
    }

    pub fn mark_whats_new_seen(&mut self) {
        if self.config.seen_version.as_deref() != Some(running_version()) {
            self.config.seen_version = Some(running_version().to_string());
            self.config.save(&self.config_path);
        }
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

    pub(crate) fn render_whats_new(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = self.theme;
        let page = self.volatile_page.as_mut()?;
        let locale = i18n::active();
        if page.notes.locale != locale {
            page.notes.locale = locale;
            page.notes.tree = notes_tree(locale, page.notes.new_after.as_deref());
            if self.edit.as_ref().is_some_and(|edit| matches!(edit.target, Target::Field(crate::outline::Field::WhatsNew(_)))) {
                self.edit = None;
            }
        }
        let scroll = page.scroll.clone();
        let rows = flatten(&page.notes.tree, ROOT, &Default::default());
        let content = div()
            .id("whats-new")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .child(div().px(px(PAD_X + 24.)).child(self.render_auxiliary_title(t!("whats_new.title"), "whats-new-back", cx)))
            .child(div().pb_8().children(rows.into_iter().map(|row| self.render_whats_new_row(row, window, cx))));
        Some(crate::ui::scrollbar::area("whats-new-bar", &scroll, theme, content).size_full().into_any_element())
    }

    fn render_whats_new_row(&mut self, row: flowie_core::Row, window: &mut gpui::Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = self.theme;
        let id = row.id;
        let tree = &self.notes_page().unwrap().tree;
        let node = tree.node(id);
        let collapsed = node.collapsed;
        let (font_size, line_h, top_pad) = crate::outline::row_metrics(node.kind);
        let has_children = tree.has_children(id);
        let toggle = move |this: &mut Outline, _: &gpui::ClickEvent, _: &mut gpui::Window, cx: &mut Context<Outline>| {
            let Some(page) = this.notes_page_mut() else { return };
            page.tree.set_collapsed(id, !collapsed);
            page.tree.take_journal();
            this.end_field_edit();
            cx.notify();
        };
        let marker = div()
            .id(("whats-new-marker", id.as_usize()))
            .w(px(18.))
            .h(px(line_h))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .child(crate::outline::bullet_dot(theme, collapsed, false, false))
            .when(has_children, |marker| marker.cursor_pointer().on_click(cx.listener(toggle)));
        let target = Target::Field(crate::outline::Field::WhatsNew(id));
        let editing = self.edit.as_ref().is_some_and(|edit| edit.target == target);
        crate::outline::bullet_row(row.depth, top_pad)
            .group("notes-row")
            .when(has_children, |row_el| {
                row_el.child(
                    div()
                        .id(("notes-fold", id.as_usize()))
                        .absolute()
                        .left(px(crate::outline::PAD_X + row.depth as f32 * crate::outline::INDENT - 20.))
                        .top(px(top_pad))
                        .w(px(18.))
                        .h(px(line_h))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .opacity(if collapsed { 1. } else { 0. })
                        .group_hover("notes-row", |style| style.opacity(1.))
                        .child(crate::ui::icons::icon(if collapsed { "chevron-right" } else { "chevron-down" }, 10., theme.muted))
                        .on_click(cx.listener(toggle)),
                )
            })
            .children(crate::outline::indent_guides(row.depth, theme))
            .when(editing, |row| row.bg(theme.row_selected.opacity(0.45)))
            .child(marker)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .pl(px(6.))
                    .text_size(px(font_size))
                    .line_height(px(line_h))
                    .child(self.render_node_text(id, target, window, cx)),
            )
            .into_any_element()
    }
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
        let headings = |md| sections(md).into_iter().map(|release| (release.version, release.date)).collect::<Vec<_>>();
        assert_eq!(headings(CHANGELOG), headings(CHANGELOG_ZH));
    }
}
