//! Window chrome, the outline list, and its overlays.

use flowie_core::{markup, NodeId, ROOT};
use gpui::{
    canvas, div, list, prelude::*, px, AnyElement, Context, DispatchPhase, FontWeight, HitboxBehavior, KeyContext, MouseButton, Pixels,
    ScrollEvent, Window,
};

use super::structure::DragNode;
use super::*;
use crate::app::i18n::t;
use crate::overlays::popover::PopoverKind;
use crate::ui::anim::ease_out;

pub const TITLE_SIZE: f32 = 26.0;

impl Outline {
    /// A node's location as one line: `Home › … › parent`. Levels that
    /// don't fit collapse into "…"; with `popovers`, hovering "…" (or a
    /// truncated level) lists the hidden levels / full text, and pressing
    /// "…" pins that list. Every level is clickable.
    pub(crate) fn render_path(&self, id: NodeId, width: f32, popovers: bool, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        const MAX: f32 = 180.;
        const SEP: f32 = 18.;
        const ELLIPSIS: f32 = 44.;
        let theme = self.theme;
        let mut levels = vec![ROOT];
        levels.extend(self.tree.ancestors(id));
        let labels: Vec<String> = levels.iter().map(|&l| self.crumb_label(l)).collect();
        let font = window.text_style().font();
        let widths: Vec<f32> = labels
            .iter()
            .map(|t| {
                let run = gpui::TextRun {
                    len: t.len(),
                    font: font.clone(),
                    color: theme.muted,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                f32::from(window.text_system().shape_line(t.clone().into(), px(13.), &[run], None).width) + 8.
            })
            .collect();
        let width = width.max(80.);
        // Keep Home and as many trailing levels as fit; the last always shows.
        let mut tail = 1;
        let mut used = widths[0];
        for ix in (1..levels.len()).rev() {
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
            let max = if ix + 1 == levels.len() && levels.len() > 1 { last_max } else { MAX };
            let truncated = widths[ix] > max;
            slots.push((Some(levels[ix]), labels[ix].clone(), max, truncated.then(|| vec![levels[ix]])));
        }
        let count = slots.len();
        let children: Vec<AnyElement> = slots
            .into_iter()
            .enumerate()
            .map(|(ix, (target, label, max, hidden))| {
                let mut crumb = div()
                    .id(("path-part", ix))
                    .min_w_0()
                    .max_w(px(max))
                    .px_1()
                    .rounded_sm()
                    .cursor_pointer()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .hover(|s| s.bg(theme.hover).text_color(theme.text))
                    .child(label);
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
            .text_color(theme.muted)
            .children(children)
            .into_any_element()
    }

    fn render_title(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.zoom == ROOT {
            return None;
        }
        let id = self.zoom;
        let text = self.render_text(id, false, window, cx);
        let references = self.render_reference_count(id, cx);
        let note = (!self.tree.node(self.tree.source(id)).note.is_empty()
            || self.edit.as_ref().is_some_and(|e| e.target == Target::Note(id)))
        .then(|| self.render_text(id, true, window, cx));
        Some(
            div()
                .px(px(PAD_X + 24.))
                .pt_2()
                .pb_3()
                .child(
                    div()
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
        let overlay = self.page.is_some();
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

        let zoom_p = self.zoom_tween.map(|t| ease_out(t.t())).unwrap_or(1.0);
        let starred = self.zoom != ROOT && self.starred.contains(&self.zoom);
        let tool = |id: &'static str, icon: &'static str, color: gpui::Hsla| crate::ui::button::tool_button(theme, id, icon, color);
        let saved = !self.query.trim().is_empty() && self.saved_searches.iter().any(|q| q == self.query.trim());
        let tools = div()
            .flex()
            .items_center()
            .gap_1()
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
            .child(
                tool("calendar", "calendar", theme.muted)
                    .on_click(cx.listener(|this, _, window, cx| this.show_calendar(&ShowCalendar, window, cx))),
            )
            .child(div().w(px(6.)))
            .child(self.render_search(window, cx));
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .pl(px(PAD_X + 40.))
            .pr(px(PAD_X))
            .pt(px(10.))
            .pb(px(6.))
            .gap_3()
            .child(div().flex_1().min_w_0().child(self.render_path(
                self.zoom,
                f32::from(window.viewport_size().width) - 520.,
                true,
                window,
                cx,
            )))
            .child(tools.flex_none());
        let title_el = if overlay { None } else { self.render_title(window, cx) };

        let entity = cx.entity();
        let body: AnyElement = if let Some(page) = self.render_page(window, cx) {
            page
        } else {
            // A popover sits at fixed window coordinates: scrolling the page
            // under it closes it.
            self.wheel.step(&self.list);
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
                            list(self.list.clone(), move |ix, window, cx| entity.update(cx, |this, cx| this.render_row(ix, window, cx)))
                                .size_full(),
                        ),
                )
                .child(crate::ui::scrollbar::scrollbar("outline-bar", &self.list, self.theme))
                .into_any_element()
        };
        let app_menu_bar = self.render_app_menu_bar(window, cx);
        let palette = self.render_palette(window, cx);
        let package_export = self.render_package_export(window, cx);
        let package_import = self.render_package_import(window, cx);
        let image_preview = self.render_image_preview(cx);
        let settings = self.render_settings(window, cx);
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

        let empty_hint = (!overlay && self.rows.is_empty() && !self.query.is_empty())
            .then(|| div().px(px(PAD_X + 24.)).py_4().text_color(theme.muted).child(t!("outline.no_match", query = self.query)));

        if self.is_animating() {
            window.request_animation_frame();
        }

        div()
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
                if this.image_preview.is_some() {
                    this.image_preview_key(ev, cx);
                    if overlay && ev.keystroke.key == "escape" {
                        this.leave_volatile_page(true, cx);
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
            .when(!overlay, |page| page.child(header))
            .child(
                div()
                    .id("page")
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .opacity(zoom_p)
                    .mt(px((1.0 - zoom_p) * 14.0))
                    .children(title_el)
                    .children(empty_hint)
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
            .children(toast)
    }
}

fn window_title(o: &Outline) -> String {
    if let Some(page) = &o.page {
        let title = page.title();
        format!("{title} — Flowie")
    } else if o.zoom == ROOT {
        "Flowie".into()
    } else {
        let text = markup::plain_text(&o.tree.node(o.zoom).text);
        if text.is_empty() {
            format!("{} — Flowie", t!("common.untitled"))
        } else {
            format!("{text} — Flowie")
        }
    }
}
