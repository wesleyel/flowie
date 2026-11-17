//! Month calendar popup (⌘⇧C): days that have dated bullets get a dot,
//! days with a journal page get a ring. Click a day to open its journal
//! page; ⌥-click to search bullets dated that day.

use std::collections::HashMap;

use chrono::{Datelike, Local, NaiveDate};
use flowie_core::{markup, search::today};
use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, MouseButton, MouseDownEvent, SharedString, Window};

use crate::outline::{Outline, ShowCalendar};

pub struct Calendar {
    pub year: i32,
    pub month: u32,
    /// date -> number of bullets mentioning it
    pub counts: HashMap<String, usize>,
}

impl Outline {
    pub fn show_calendar(&mut self, _: &ShowCalendar, _: &mut Window, cx: &mut Context<Self>) {
        if self.calendar.take().is_some() {
            cx.notify();
            return;
        }
        let now = Local::now();
        let mut counts: HashMap<String, usize> = HashMap::new();
        for n in self.tree.nodes() {
            if n.text.len() >= 10 && n.text.contains('-') {
                for d in markup::dates(&n.text) {
                    *counts.entry(d.to_string()).or_default() += 1;
                }
            }
        }
        self.calendar = Some(Calendar { year: now.year(), month: now.month(), counts });
        cx.notify();
    }

    fn shift_month(&mut self, delta: i32, cx: &mut Context<Self>) {
        if let Some(c) = self.calendar.as_mut() {
            let m = c.year * 12 + c.month as i32 - 1 + delta;
            c.year = m.div_euclid(12);
            c.month = (m.rem_euclid(12) + 1) as u32;
        }
        cx.notify();
    }

    fn journal_days(&self) -> Vec<String> {
        match self.journal_id.filter(|j| self.tree.contains(*j)) {
            Some(j) => self.tree.children(j).iter().map(|id| self.tree.node(*id).text.chars().take(10).collect()).collect(),
            None => Vec::new(),
        }
    }

    pub fn render_calendar(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let c = self.calendar.as_ref()?;
        let theme = self.theme;
        let first = NaiveDate::from_ymd_opt(c.year, c.month, 1)?;
        let next = if c.month == 12 { NaiveDate::from_ymd_opt(c.year + 1, 1, 1)? } else { NaiveDate::from_ymd_opt(c.year, c.month + 1, 1)? };
        let days = (next - first).num_days() as u32;
        let lead = first.weekday().num_days_from_monday();
        let today = today();
        let journal = self.journal_days();
        let title: SharedString = first.format("%B %Y").to_string().into();
        let mut cells: Vec<AnyElement> = Vec::new();
        for _ in 0..lead {
            cells.push(div().size(px(36.)).into_any_element());
        }
        for d in 1..=days {
            let date = format!("{:04}-{:02}-{:02}", c.year, c.month, d);
            let count = c.counts.get(&date).copied().unwrap_or(0);
            let is_today = date == today;
            let has_page = journal.contains(&date);
            let date2 = date.clone();
            cells.push(
                div()
                    .id(SharedString::from(format!("day-{date}")))
                    .size(px(36.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.hover))
                    .when(is_today, |s| s.bg(theme.row_selected).font_weight(FontWeight::BOLD))
                    .when(has_page, |s| s.border_1().border_color(theme.accent.opacity(0.6)))
                    .child(d.to_string())
                    .child(div().size(px(4.)).rounded_full().when(count > 0, |s| s.bg(theme.accent)))
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, ev: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        if ev.modifiers.alt {
                            this.calendar = None;
                            this.search_for(&format!("date:{date2}"), cx);
                        } else {
                            this.open_day(&date2, cx);
                        }
                    }))
                    .into_any_element(),
            );
        }
        let head = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"]
            .into_iter()
            .map(|w| div().w(px(36.)).flex().justify_center().text_size(px(11.)).text_color(theme.muted).child(w));
        let nav = |icon: &'static str, delta: i32, cx: &mut Context<Self>| {
            div()
                .id(icon)
                .flex_none()
                .size(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .cursor_pointer()
                .hover(|s| s.bg(theme.hover))
                .child(crate::icons::icon(icon, crate::icons::MD, theme.muted))
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.shift_month(delta, cx);
                }))
        };
        Some(
            div()
                .absolute()
                .inset_0()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                    this.calendar = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .absolute()
                        .top(px(46.))
                        .right(px(36.))
                        .w(px(36. * 7. + 26.))
                        .p_3()
                        .bg(theme.bg)
                        .border_1()
                        .border_color(theme.border)
                        .rounded_lg()
                        .shadow_lg()
                        .text_size(px(13.))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .pb_2()
                                .child(nav("chevron-left", -1, cx))
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                                .child(nav("chevron-right", 1, cx)),
                        )
                        .child(div().flex().children(head))
                        .child(div().flex().flex_wrap().children(cells))
                        .child(
                            div()
                                .pt_2()
                                .text_size(px(11.))
                                .text_color(theme.muted)
                                .child("Click: open that day's page · ⌥-click: bullets dated that day"),
                        ),
                )
                .into_any_element(),
        )
    }
}
