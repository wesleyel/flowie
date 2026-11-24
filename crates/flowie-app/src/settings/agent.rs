//! Agent access and connection instructions in Settings.

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight};

use crate::app::i18n::t;
use crate::app::keymap;
use crate::outline::Outline;

impl Outline {
    pub(super) fn render_agent(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let scroll = self.settings.as_ref().map(|s| s.scroll.clone()).unwrap_or_default();
        use crate::agent::AgentMode;
        let theme = self.theme;
        let mode = self.config.agent;
        let modes = [
            (AgentMode::Off, "off", t!("settings.agent.mode.off")),
            (AgentMode::ReadOnly, "read-only", t!("settings.agent.mode.read_only")),
            (AgentMode::ReadWrite, "read-write", t!("settings.agent.mode.read_write")),
        ];
        let segmented = crate::ui::segmented::track(theme).children(modes.into_iter().map(|(m, id, label)| {
            crate::ui::segmented::segment(theme, id, m == mode).child(label).on_click(cx.listener(move |this, _, _, cx| {
                this.config.agent = m;
                this.save_config();
                cx.notify();
            }))
        }));
        let setup = crate::agent::mcp::Setup::new(self.db_path.as_deref());
        let (claude, json_cfg) = (setup.claude_command(), setup.client_config());
        let claude_label = self.label("mcp-claude", claude, cx);
        let json_label = self.label("mcp-json", json_cfg, cx);
        let block = |label: gpui::Entity<crate::text::selectable::SelectableText>| {
            div().p_2().rounded_md().bg(theme.code_bg).font_family(crate::app::platform::MONO_FONT).text_size(px(12.)).child(label)
        };
        let desc = match mode {
            AgentMode::Off => t!("settings.agent.desc.off").to_string(),
            AgentMode::ReadOnly => t!("settings.agent.desc.read_only").to_string(),
            AgentMode::ReadWrite => t!("settings.agent.desc.read_write", undo = keymap::hint("undo", &self.config.keys)),
        };
        let follow = self.config.agent_follow_edits;
        let follow_toggle = div()
            .id("agent-follow-edits")
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .child(crate::ui::controls::checkbox(theme, follow))
            .child(t!("settings.agent.follow_edits.toggle"))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.config.agent_follow_edits = !follow;
                this.save_config();
                cx.notify();
            }));
        div()
            .id("agent")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .px_4()
            .pt_3()
            .pb_4()
            .child(crate::ui::text::page_title(theme, t!("settings.agent.title"), None).pb_1())
            .child(div().text_size(px(12.)).text_color(theme.muted).pb_3().child(t!("settings.agent.intro")))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .pb_3()
                    .border_b_1()
                    .border_color(theme.divider)
                    .child(segmented)
                    .child(div().text_size(px(12.)).text_color(theme.muted).child(desc)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .py_3()
                    .border_b_1()
                    .border_color(theme.divider)
                    .child(follow_toggle)
                    .child(div().text_size(px(12.)).text_color(theme.muted).child(t!("settings.agent.follow_edits.hint"))),
            )
            .child(div().pt_3().pb_1().font_weight(FontWeight::SEMIBOLD).child("Claude Code"))
            .child(block(claude_label))
            .child(div().pt_3().pb_1().font_weight(FontWeight::SEMIBOLD).child(t!("settings.agent.other_clients")))
            .child(block(json_label))
            .child(div().pt_3().text_size(px(12.)).text_color(theme.muted).child(t!("settings.agent.tools")))
            .into_any_element()
    }
}
