//! Window chrome, the outline list, and its overlays.

use flowie_core::{markup, NodeId, ROOT};
use gpui::{
    canvas, div, list, prelude::*, px, AnyElement, Context, DispatchPhase, FontWeight, HitboxBehavior, KeyContext, MouseButton, Pixels,
    ScrollEvent, Window,
};

use super::navigation::{SEARCH_CLOSED_W, SEARCH_OPEN_W};
use super::structure::DragNode;
use super::*;
use crate::app::i18n::t;
use crate::overlays::popover::PopoverKind;
use crate::ui::anim::ease_out;

pub const TITLE_SIZE: f32 = 26.0;
/// Scrolling this far collapses the page title into the top bar.
const TITLE_COLLAPSE: f32 = 44.0;

impl Outline {
    /// A node's location as one line: `Home › … › parent`. Levels that
    /// don't fit collapse into "…"; with `popovers`, hovering "…" (or a
    /// truncated level) lists the hidden levels / full text, and pressing
    /// "…" pins that list. Every level is clickable.
    pub(crate) fn render_path(&self, id: NodeId, width: f32, popovers: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mut levels = vec![ROOT];
        levels.extend(self.ws.tree.ancestors(id));
        self.render_crumbs(&levels, "path-part", self.theme.muted, width, popovers, false, window, cx)
    }

    /// `levels` as one line of clickable crumbs, folded to fit `width` (see
    /// `render_path`). `key` keeps element ids apart when several show at once;
    /// with `home_icon`, Home is a house icon instead of its name.
    #[allow(clippy::too_many_arguments)]
    fn render_crumbs(
        &self,
        levels: &[NodeId],
        key: &'static str,
        color: gpui::Hsla,
        width: f32,
        popovers: bool,
        home_icon: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        const MAX: f32 = 180.;
        const ICON_W: f32 = crate::ui::icons::SM + 8.;
        let home_icon = home_icon && levels.first() == Some(&ROOT);
        const SEP: f32 = 18.;
        const ELLIPSIS: f32 = 44.;
        let theme = self.theme;
        let labels: Vec<String> = levels.iter().map(|&l| self.crumb_label(l)).collect();
        let font = window.text_style().font();
        let widths: Vec<f32> = labels
            .iter()
            .map(|t| {
                let run =
                    gpui::TextRun { len: t.len(), font: font.clone(), color, background_color: None, underline: None, strikethrough: None };
                f32::from(window.text_system().shape_line(t.clone().into(), px(13.), &[run], None).width) + 8.
            })
            .collect();
        let mut widths = widths;
        if home_icon {
            widths[0] = ICON_W;
        }
        let width = width.max(80.);
        // Everything fits at full length: show it all, untruncated.
        let full = widths.iter().sum::<f32>() + SEP * (levels.len() - 1) as f32;
        let fits = full <= width;
        // Otherwise keep Home and as many trailing levels as fit; the last always shows.
        let mut tail = 1;
        let mut used = widths[0];
        for ix in (1..levels.len()).rev() {
            if fits {
                tail = 1;
                break;
            }
            let reserve = if ix > 1 { ELLIPSIS } else { 0. };
            if ix == levels.len() - 1 || used + widths[ix].min(MAX) + SEP + reserve <= width {
                used += widths[ix].min(MAX) + SEP;
                tail = ix;
            } else {
                break;
            }
        }
        // (target, label, max width, hidden levels for the popover)
        let mut slots: Vec<(Option<NodeId>, String, f32, Option<Vec<NodeId>>)> = vec![(Some(ROOT), labels[0].clone(), MAX, None)];
        if tail > 1 {
            slots.push((None, "…".into(), ELLIPSIS, Some(levels[1..tail].to_vec())));
        }
        let last_max = (width - 58. - if tail > 1 { ELLIPSIS } else { 0. }).clamp(24., MAX);
        for ix in tail..levels.len() {
            let max = if fits {
                widths[ix] + 1.
            } else if ix + 1 == levels.len() && levels.len() > 1 {
                last_max
            } else {
                MAX
            };
            let truncated = widths[ix] > max;
            slots.push((Some(levels[ix]), labels[ix].clone(), max, truncated.then(|| vec![levels[ix]])));
        }
        let count = slots.len();
        let children: Vec<AnyElement> = slots
            .into_iter()
            .enumerate()
            .map(|(ix, (target, label, max, hidden))| {
                let mut crumb = div()
                    .id((key, ix))
                    .min_w_0()
                    .max_w(px(max))
                    .px_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .hover(|s| s.bg(theme.hover).text_color(theme.text));
                crumb = if ix == 0 && home_icon {
                    crumb.flex().items_center().h(px(20.)).child(crate::ui::icons::icon("house", crate::ui::icons::SM, color))
                } else {
                    crumb.child(label)
                };
                if let (true, Some(levels)) = (popovers, hidden) {
                    crumb = self.popover_anchor(crumb, PopoverKind::Path(levels), target.is_none(), cx);
                }
                if let Some(target) = target {
                    crumb = crumb.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).on_click(cx.listener(
                        move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.zoom_to(target, cx);
                        },
                    ));
                }
                let level = div().min_w_0().flex().items_center().gap_1();
                if ix + 1 < count {
                    level
                        .flex_none()
                        .child(crumb)
                        .child(crate::ui::icons::icon("chevron-right", crate::ui::icons::SM, theme.faint))
                        .into_any_element()
                } else {
                    level.flex_1().child(crumb).into_any_element()
                }
            })
            .collect();
        div()
            .min_w_0()
            .w_full()
            .overflow_hidden()
            .flex()
            .items_center()
            .gap_1()
            .text_size(px(13.))
            .line_height(px(20.))
            .text_color(color)
            .children(children)
            .into_any_element()
    }

    /// The page header, the list's first item: the zoomed node's title (with
    /// its note and reference count), fading as it scrolls under the top bar,
    /// and the no-match hint of a search. Empty on Home.
    pub(super) fn render_page_header(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let title = self.render_title(window, cx);
        let empty_hint = (self.rows.is_empty() && !self.query.is_empty())
            .then(|| div().px(px(PAD_X + 24.)).py_4().text_color(self.theme.muted).child(t!("outline.no_match", query = self.query)));
        div().children(title).children(empty_hint).into_any_element()
    }

    /// How far the page title has collapsed into the top bar: 0 (shown in
    /// full) to 1 (scrolled away), following the scroll position.
    pub(super) fn title_collapse(&self) -> f32 {
        if self.zoom == ROOT || self.is_auxiliary_page_open() {
            return 0.;
        }
        (self.scroll_px() / TITLE_COLLAPSE).clamp(0., 1.)
    }

    /// A soft edge where rows scroll under the top bar: a strip that fades from the
    /// background to clear, so a row passing under the bar fades out instead of
    /// being cut off. It appears as the page scrolls and is gone at the top.
    fn render_top_fade(&self) -> AnyElement {
        const FADE: f32 = 24.;
        let shown = (self.scroll_px() / FADE).clamp(0., 1.);
        let mut clear = self.theme.bg;
        clear.a = 0.;
        div()
            .absolute()
            .top_0()
            .left_0()
            .w_full()
            .h(px(FADE))
            .opacity(shown)
            .bg(gpui::linear_gradient(180., gpui::linear_color_stop(self.theme.bg, 0.), gpui::linear_color_stop(clear, 1.)))
            .into_any_element()
    }

    fn scroll_px(&self) -> f32 {
        self.heights.scroll_top(&self.list, self.zoom).unwrap_or_else(|| f32::from(-self.list.scroll_px_offset_for_scrollbar().y).max(0.))
    }

    /// After each scroll, wait for it to stop; then a title left half
    /// collapsed snaps open or shut, like a large title on iOS.
    fn watch_title_scroll(&mut self, cx: &mut Context<Self>) {
        let s = self.scroll_px();
        if (s - self.title_scroll).abs() < 0.5 {
            return;
        }
        self.title_scroll = s;
        if self.zoom == ROOT || s <= 0. || s >= TITLE_COLLAPSE {
            self.title_settle = None;
            return;
        }
        self.title_settle = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(180)).await;
            this.update(cx, |this, cx| this.settle_title(cx)).ok();
        }));
    }

    fn settle_title(&mut self, cx: &mut Context<Self>) {
        self.title_settle = None;
        let s = self.scroll_px();
        if self.zoom == ROOT || self.wheel.running() || s <= 0. || s >= TITLE_COLLAPSE {
            return;
        }
        let target = if s < TITLE_COLLAPSE / 2. { 0. } else { TITLE_COLLAPSE };
        if self.config.smooth_scroll {
            self.wheel.push(px(target - s));
        } else {
            self.list.set_offset_from_scrollbar(gpui::point(Pixels::ZERO, px(-target)));
        }
        cx.notify();
    }

    /// The top bar's path. In full: `Home › … › parent of this page`. As
    /// the title collapses, it fades into `⌂ › … › this page › … › parent of
    /// the topmost visible bullet`, which takes over from the title.
    fn render_header_path(&self, collapse: f32, width: f32, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let color = self.theme.text;
        let mut levels = vec![ROOT];
        levels.extend(self.ws.tree.ancestors(self.zoom));
        let full = (collapse < 1.).then(|| self.render_crumbs(&levels, "path-part", color, width, true, false, window, cx));
        let collapsed = (collapse > 0.).then(|| {
            levels.push(self.zoom);
            levels.extend(self.top_row_path());
            self.render_crumbs(&levels, "title-path-part", color, width, true, true, window, cx)
        });
        let overlap = full.is_some();
        div()
            .relative()
            .w_full()
            .min_w_0()
            .font_weight(FontWeight::MEDIUM)
            .children(full.map(|p| div().opacity(1. - collapse).child(p)))
            .children(collapsed.map(|p| {
                let layer = div().opacity(collapse).child(p);
                if overlap {
                    layer.absolute().top_0().left_0().right_0()
                } else {
                    layer
                }
            }))
            .into_any_element()
    }

    /// The levels between this page and the topmost visible bullet: this
    /// page's descendants down to that bullet's parent.
    fn top_row_path(&self) -> Vec<NodeId> {
        let view = self.list.viewport_bounds();
        let first = self.list.logical_scroll_top().item_ix.saturating_sub(HEAD);
        let top_row = (first..self.display.len().min(first + 8))
            .find(|&ix| self.list.bounds_for_item(ix + HEAD).is_some_and(|b| b.bottom() > view.top()))
            .map(|ix| self.ws.tree.resolve(self.display[ix].row.id))
            .filter(|r| self.ws.tree.contains(*r));
        let Some(row) = top_row else { return Vec::new() };
        let chain = self.ws.tree.ancestors(row);
        match chain.iter().position(|&a| a == self.zoom) {
            Some(at) => chain[at + 1..].to_vec(),
            None => Vec::new(),
        }
    }

    pub(super) fn render_title(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.zoom == ROOT {
            return None;
        }
        let id = self.zoom;
        let text = self.render_text(id, false, window, cx);
        let references = self.render_reference_count(id, cx);
        let note = (!self.ws.tree.node(self.ws.tree.source(id)).note.is_empty()
            || self.edit.as_ref().is_some_and(|e| e.target == Target::Note(id)))
        .then(|| self.render_text(id, true, window, cx));
        Some(
            div()
                .px(px(PAD_X + 24.))
                .pt_2()
                .pb_3()
                .child(
                    div()
                        // The heading fades into the top bar; the note just scrolls.
                        // (`title_collapsed`, not `title_collapse()`: the list is laying out its items.)
                        .opacity(1.0 - self.title_collapsed)
                        .text_size(px(TITLE_SIZE))
                        .line_height(px(TITLE_SIZE * 1.35))
                        .font_weight(FontWeight::BOLD)
                        .flex()
                        .items_start()
                        .child(div().flex_1().min_w_0().child(text))
                        .children(references),
                )
                .children(note.map(|n| div().pt_1().text_size(px(14.)).child(n)))
                .into_any_element(),
        )
    }
}

impl Render for Outline {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.tick();
        let theme = self.theme;

        let mut kc = KeyContext::new_with_defaults(cx.platform_info().os);
        // While settings or a dialog are open, only global shortcuts apply (so keys can be recorded).
        let overlay = self.is_auxiliary_page_open();
        if overlay && !self.modal_open() {
            kc.add("Outline");
        }
        if !self.modal_open() && !overlay {
            kc.add("Outline");
            if self.edit.is_some() {
                kc.add("editing");
            }
            if self.block.is_some() {
                kc.add("selecting");
            }
        }

        let title = window_title(self);
        window.set_window_title(&title);

        let collapse = if overlay { 0. } else { self.title_collapse() };
        self.title_collapsed = collapse;
        // Once the title (and its reference count) scrolls away, the count shows in the top bar.
        let references = (collapse > 0.).then(|| self.render_reference_count(self.zoom, cx)).flatten();
        let has_references = references.is_some().then(|| self.ws.tree.link_count(self.ws.tree.source(self.zoom)));
        let search_open = self.search_openness();
        let zoom_p = self.zoom_tween.map(|t| ease_out(t.t())).unwrap_or(1.0);
        let starred = self.zoom != ROOT && self.ws.tree.is_starred(self.zoom);
        let tool = |id: &'static str, icon: &'static str, color: gpui::Hsla| crate::ui::button::tool_button(theme, id, icon, color);
        let saved = !self.query.trim().is_empty() && self.saved_searches.iter().any(|q| q == self.query.trim());
        let tools = div()
            .flex()
            .items_center()
            .gap_1()
            .children(references.map(|r| div().opacity(collapse).child(r)))
            .when(self.zoom != ROOT, |d| {
                d.child(tool("star", "star", if starred { theme.star } else { theme.muted }).on_click(cx.listener(|this, _, _, cx| {
                    let z = this.zoom;
                    this.star(z, cx);
                })))
            })
            .when(!self.query.trim().is_empty(), |d| {
                d.child(
                    tool("save-search", if saved { "bookmark-check" } else { "bookmark" }, if saved { theme.accent } else { theme.muted })
                        .on_click(cx.listener(|this, _, window, cx| this.save_search(&SaveSearch, window, cx))),
                )
            })
            .when_some(self.sync.as_ref().map(|s| s.phase().clone()), |d, phase| {
                let (icon, color) = match phase {
                    flowie_doc::SyncPhase::Syncing => ("refresh-cw", theme.muted),
                    flowie_doc::SyncPhase::Synced => ("cloud", theme.muted),
                    flowie_doc::SyncPhase::Offline(_) => ("cloud-off", theme.danger),
                };
                d.child(
                    tool("sync", icon, color).on_click(cx.listener(|this, _, _, cx| this.show_settings(crate::settings::Tab::Sync, cx))),
                )
            })
            .child(
                tool("calendar", "calendar", theme.muted)
                    .on_click(cx.listener(|this, _, window, cx| this.show_calendar(&ShowCalendar, window, cx))),
            )
            // The search box sits a little apart; its icon spaces like the other tools.
            .when(search_open > 0., |d| d.child(div().w(px(6. * search_open))))
            .child(self.render_search(search_open, window, cx));
        // What the path has to fit in: the bar's width less its padding, the gap and the tools shown.
        let path_width = {
            const TOOL: f32 = 28.;
            const GAP: f32 = 4.;
            let mut widths = vec![TOOL]; // calendar
            if self.sync.is_some() {
                widths.push(TOOL);
            }
            if self.zoom != ROOT {
                widths.push(TOOL);
            }
            if !self.query.trim().is_empty() {
                widths.push(TOOL);
            }
            if let Some(count) = has_references {
                // Margin, padding, link icon, gap, digits.
                widths.push(8. + 8. + crate::ui::icons::SM + 4. + 8. * count.to_string().len() as f32);
            }
            if search_open > 0. {
                widths.push(6. * search_open);
            }
            widths.push(SEARCH_CLOSED_W + (SEARCH_OPEN_W - SEARCH_CLOSED_W) * search_open);
            let tools: f32 = widths.iter().sum::<f32>() + GAP * (widths.len() - 1) as f32;
            f32::from(window.viewport_size().width) - (PAD_X + 20.) - PAD_X - 12. - tools
        };
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            // Crumb text (inside 4px of padding) lines up with the title and row text.
            .pl(px(PAD_X + 20.))
            .pr(px(PAD_X))
            .pt(px(10.))
            .pb(px(6.))
            .gap_3()
            .child(div().flex_1().min_w_0().child(self.render_header_path(collapse, path_width, window, cx)))
            .child(tools.flex_none());

        let entity = cx.entity();
        let body: AnyElement = if let Some(page) = self.render_whats_new(window, cx).or_else(|| self.render_trash(cx)) {
            page
        } else {
            // A popover sits at fixed window coordinates: scrolling the page
            // under it closes it.
            self.wheel.step(&self.list);
            self.watch_title_scroll(cx);
            self.update_heights(window);
            let wheel_target = cx.entity();
            div()
                .relative()
                .size_full()
                .child(
                    // Takes mouse-wheel steps before the list does, to animate them.
                    canvas(
                        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                        move |_, hitbox, window, _| {
                            window.on_mouse_event(move |e: &ScrollEvent, phase, window, cx| {
                                if phase != DispatchPhase::Capture || e.delta.precise() || !hitbox.should_handle_scroll(window) {
                                    return;
                                }
                                let dy = e.delta.pixel_delta(px(20.)).y;
                                let taken = dy != Pixels::ZERO
                                    && wheel_target.update(cx, |this, cx| {
                                        if !this.config.smooth_scroll {
                                            return false;
                                        }
                                        this.wheel.push(-dy);
                                        this.popover.close();
                                        this.close_menu();
                                        this.close_app_menu();
                                        cx.notify();
                                        true
                                    });
                                if taken {
                                    cx.stop_propagation();
                                }
                            });
                        },
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
                .child(
                    div()
                        .id("outline-scroll")
                        .size_full()
                        .on_scroll(cx.listener(|this, _, _, cx| {
                            if this.popover.close() | this.close_menu() | this.close_app_menu() {
                                cx.notify();
                            }
                        }))
                        .child(
                            list(self.list.clone(), move |ix, window, cx| {
                                entity.update(cx, |this, cx| {
                                    let item = this.render_row(ix, window, cx);
                                    this.in_content_column(item)
                                })
                            })
                            .size_full(),
                        ),
                )
                .child(crate::ui::scrollbar::scrollbar("outline-bar", self.heights.scroll_model(&self.list), self.theme))
                .child(self.render_top_fade())
                .into_any_element()
        };
        let app_menu_bar = self.render_app_menu_bar(window, cx);
        let palette = self.render_palette(window, cx);
        let package_export = self.render_package_export(window, cx);
        let package_import = self.render_package_import(window, cx);
        let image_preview = self.render_image_preview(cx);
        let settings = self.render_settings(window, cx);
        let close_dialog = self.render_close_dialog(cx);
        let inline = self.render_inline(cx);
        let calendar = self.render_calendar(cx);
        let popover = self.render_popover(window, cx);
        let menu = self.render_menu(window, cx);
        let toast = self.toast.as_ref().filter(|(_, t)| !t.done()).map(|(msg, t)| {
            let tt = t.t();
            let o = if tt < 0.08 {
                tt / 0.08
            } else if tt > 0.85 {
                (1.0 - tt) / 0.15
            } else {
                1.0
            };
            crate::ui::surface::toast(theme, msg.clone().into(), o)
        });

        if self.follow_edit > 0 {
            self.follow_edit_frame();
        }
        if self.is_animating() {
            window.request_animation_frame();
        }

        let outline = div()
            .id("outline")
            .key_context(kc)
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.bg)
            .text_color(theme.text)
            .font_family(crate::ui::theme::UI_FONT)
            .text_size(px(FONT_SIZE))
            .line_height(px(LINE_H))
            .when(!overlay, |outline| {
                outline
                    .on_action(cx.listener(Self::newline))
                    .on_action(cx.listener(Self::toggle_note))
                    .on_action(cx.listener(Self::indent))
                    .on_action(cx.listener(Self::outdent))
                    .on_action(cx.listener(Self::backspace))
                    .on_action(cx.listener(Self::delete))
                    .on_action(cx.listener(Self::delete_word_left))
                    .on_action(cx.listener(Self::kill_line))
                    .on_action(cx.listener(Self::yank))
                    .on_action(cx.listener(Self::left))
                    .on_action(cx.listener(Self::right))
                    .on_action(cx.listener(Self::up))
                    .on_action(cx.listener(Self::down))
                    .on_action(cx.listener(Self::word_left))
                    .on_action(cx.listener(Self::word_right))
                    .on_action(cx.listener(Self::select_left))
                    .on_action(cx.listener(Self::select_right))
                    .on_action(cx.listener(Self::select_word_left))
                    .on_action(cx.listener(Self::select_word_right))
                    .on_action(cx.listener(Self::select_up))
                    .on_action(cx.listener(Self::select_down))
                    .on_action(cx.listener(Self::home))
                    .on_action(cx.listener(Self::end))
                    .on_action(cx.listener(Self::select_home))
                    .on_action(cx.listener(Self::select_end))
                    .on_action(cx.listener(Self::select_all))
                    .on_action(cx.listener(Self::toggle_complete))
                    .on_action(cx.listener(Self::collapse))
                    .on_action(cx.listener(Self::expand))
                    .on_action(cx.listener(Self::zoom_in))
                    .on_action(cx.listener(Self::zoom_out))
                    .on_action(cx.listener(Self::zoom_home))
                    .on_action(cx.listener(Self::page_back))
                    .on_action(cx.listener(Self::page_forward))
                    .on_action(cx.listener(Self::move_up))
                    .on_action(cx.listener(Self::move_down))
                    .on_action(cx.listener(Self::delete_node))
                    .on_action(cx.listener(Self::undo))
                    .on_action(cx.listener(Self::redo))
                    .on_action(cx.listener(Self::copy))
                    .on_action(cx.listener(Self::cut))
                    .on_action(cx.listener(Self::paste))
                    .on_action(cx.listener(Self::bold))
                    .on_action(cx.listener(Self::italic))
                    .on_action(cx.listener(Self::inline_code))
                    .on_action(cx.listener(Self::wrap_math))
                    .on_action(cx.listener(Self::insert_link))
                    .on_action(cx.listener(Self::find))
                    .on_action(cx.listener(Self::escape))
                    .on_action(cx.listener(Self::toggle_completed_visible))
                    .on_action(cx.listener(Self::cycle_content_width))
                    .on_action(cx.listener(Self::scroll_to_top))
                    .on_action(cx.listener(Self::scroll_to_bottom))
                    .on_action(cx.listener(Self::jump))
                    .on_action(cx.listener(Self::move_to_action))
                    .on_action(cx.listener(Self::link_to_bullet))
                    .on_action(cx.listener(Self::mirror_bullet))
                    .on_action(cx.listener(Self::toggle_star))
                    .on_action(cx.listener(Self::expand_all))
                    .on_action(cx.listener(Self::level1))
                    .on_action(cx.listener(Self::level2))
                    .on_action(cx.listener(Self::level3))
                    .on_action(cx.listener(Self::level4))
                    .on_action(cx.listener(Self::import))
                    .on_action(cx.listener(Self::export_opml))
                    .on_action(cx.listener(Self::export_markdown))
                    .on_action(cx.listener(Self::export_package))
                    .on_action(cx.listener(Self::insert_image))
                    .on_action(cx.listener(Self::whats_new))
                    .on_action(cx.listener(Self::export_text))
                    .on_action(cx.listener(Self::show_trash))
                    .on_action(cx.listener(Self::save_search))
                    .on_action(cx.listener(Self::highlight))
                    .on_action(cx.listener(Self::color_red))
                    .on_action(cx.listener(Self::color_orange))
                    .on_action(cx.listener(Self::color_yellow))
                    .on_action(cx.listener(Self::color_green))
                    .on_action(cx.listener(Self::color_blue))
                    .on_action(cx.listener(Self::color_purple))
                    .on_action(cx.listener(Self::color_gray))
                    .on_action(cx.listener(Self::color_none))
                    .on_action(cx.listener(Self::cycle_color))
                    .on_action(cx.listener(Self::kind_bullet))
                    .on_action(cx.listener(Self::kind_h1))
                    .on_action(cx.listener(Self::kind_h2))
                    .on_action(cx.listener(Self::kind_h3))
                    .on_action(cx.listener(Self::kind_paragraph))
                    .on_action(cx.listener(Self::kind_todo))
                    .on_action(cx.listener(Self::kind_numbered))
                    .on_action(cx.listener(Self::kind_quote))
                    .on_action(cx.listener(Self::kind_code))
                    .on_action(cx.listener(Self::today))
                    .on_action(cx.listener(Self::next_match))
                    .on_action(cx.listener(Self::prev_match))
                    .on_action(cx.listener(Self::show_calendar))
            })
            .when(overlay, |page| {
                let page = page
                    .on_action(cx.listener(Self::escape))
                    .on_action(cx.listener(Self::page_back))
                    .on_action(cx.listener(Self::find))
                    .on_action(cx.listener(Self::show_calendar))
                    .on_action(cx.listener(Self::whats_new))
                    .on_action(cx.listener(Self::show_trash))
                    .on_action(cx.listener(|this, _: &Undo, window, cx| this.escape(&Escape, window, cx)));
                if self.edit.as_ref().is_some_and(|edit| matches!(edit.target, Target::Field(Field::WhatsNew(_)))) {
                    page.on_action(cx.listener(Self::select_all))
                        .on_action(cx.listener(Self::copy))
                        .on_action(cx.listener(Self::cut))
                        .on_action(cx.listener(Self::paste))
                } else {
                    page
                }
            })
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::open_about))
            .on_action(cx.listener(Self::send_feedback))
            .on_action(cx.listener(Self::open_tutorial))
            .on_action(cx.listener(Self::open_privacy_policy))
            .on_action(cx.listener(Self::open_terms))
            .on_key_down(cx.listener(move |this, ev: &gpui::KeyDownEvent, window, cx| {
                if this.close_dialog.is_some() {
                    this.close_dialog_key(ev, window, cx);
                } else if this.image_preview.is_some() {
                    this.image_preview_key(ev, cx);
                    if overlay && ev.keystroke.key == "escape" {
                        this.close_auxiliary_page(true, cx);
                        cx.notify();
                    }
                } else if !overlay
                    && !this.modal_open()
                    && ev.keystroke.key == "space"
                    && !ev.keystroke.modifiers.modified()
                    && this.edit.is_none()
                    && this.preview_selected_image(cx)
                {
                    cx.stop_propagation();
                } else if this.settings.is_some() {
                    this.settings_key(ev, window, cx);
                } else if this.palette.is_some() {
                    // A picker opened from a dialog ("Put in"): its keys are the outline's.
                } else if this.package_export.is_some() {
                    this.package_export_key(ev, window, cx);
                } else if this.package_import.is_some() {
                    this.package_import_key(ev, window, cx);
                } else if overlay {
                    let key = &ev.keystroke;
                    let primary = if crate::app::platform::OS == crate::app::platform::Os::Mac {
                        key.modifiers.platform
                    } else {
                        key.modifiers.control
                    };
                    if key.key == "escape" || (primary && key.key == "z" && !key.modifiers.shift) {
                        this.escape(&Escape, window, cx);
                    } else if !this.field_key(key, window, cx) {
                        return;
                    }
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Navigate(gpui::NavigationDirection::Back),
                cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.mouse_shortcut(gpui::NavigationDirection::Back, window, cx);
                }),
            )
            .on_mouse_down(
                MouseButton::Navigate(gpui::NavigationDirection::Forward),
                cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.mouse_shortcut(gpui::NavigationDirection::Forward, window, cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.mouse_block = false;
                    if let Some(e) = this.edit.as_mut() {
                        e.selecting_with_mouse = false;
                    }
                }),
            )
            .relative()
            .children(app_menu_bar)
            .child(header)
            .children(self.load_error.as_ref().map(|message| {
                let mut bg = theme.danger;
                bg.a = if theme.dark { 0.18 } else { 0.10 };
                div()
                    .id("load-error-banner")
                    .px(px(PAD_X))
                    .py_2()
                    .bg(bg)
                    .border_b_1()
                    .border_color(theme.danger)
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(theme.text)
                    .child(crate::ui::icons::icon("circle-x", crate::ui::icons::SM, theme.danger))
                    .child(div().flex_1().min_w_0().child(message.clone()))
            }))
            .children(self.save_error.as_ref().map(|error| {
                let mut bg = theme.danger;
                bg.a = if theme.dark { 0.18 } else { 0.10 };
                div()
                    .id("save-error-banner")
                    .px(px(PAD_X))
                    .py_2()
                    .bg(bg)
                    .border_b_1()
                    .border_color(theme.danger)
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(theme.text)
                    .child(crate::ui::icons::icon("circle-x", crate::ui::icons::SM, theme.danger))
                    .child(div().flex_1().min_w_0().child(t!("outline.save_error", error = error)))
                    .child(
                        crate::ui::button::button(theme, "save-retry", crate::ui::button::Variant::Link)
                            .child(crate::ui::icons::icon("rotate-ccw", crate::ui::icons::SM, theme.accent))
                            .child(t!("outline.save_retry"))
                            .on_click(cx.listener(|this, _, _, _| {
                                if let Some(w) = &this.ws.writer {
                                    w.retry_now();
                                }
                            })),
                    )
            }))
            .child(
                div()
                    .id("page")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .opacity(zoom_p)
                    .mt(px((1.0 - zoom_p) * 14.0))
                    .child(div().flex_1().min_h_0().child(body))
                    .when(!overlay, |page| page.on_drop(cx.listener(|this, drag: &DragNode, _, cx| this.perform_drop(drag.id, cx)))),
            )
            .children(inline)
            .children(calendar)
            .children(popover)
            .children(menu)
            .children(package_export)
            .children(package_import)
            .children(palette)
            .children(image_preview)
            .children(settings)
            .children(close_dialog)
            .children(toast);
        // Linux under GNOME: the border, shadow and resize edges Flowie draws itself.
        crate::app::window_frame::frame(outline, theme, window)
    }
}

fn window_title(o: &Outline) -> String {
    if o.zoom == ROOT {
        "Flowie".into()
    } else {
        let text = markup::plain_text(&o.ws.tree.node(o.zoom).text);
        if text.is_empty() {
            format!("{} — Flowie", t!("common.untitled"))
        } else {
            format!("{text} — Flowie")
        }
    }
}
