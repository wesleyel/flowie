//! Author identity and general preferences.

use gpui::{div, prelude::*, px, AnyElement, Context, FontWeight, MouseButton, Window};
use serde::{Deserialize, Serialize};

use crate::app::i18n::t;
use crate::outline::{Field, Outline, Target};

/// Who this user is when sharing (share packages name their author). One
/// per person: the same id on every device the person uses.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Author {
    /// A UUID; generated on first launch, or pasted from another device.
    #[serde(default)]
    pub id: String,
    /// Empty: the OS's full name for the user.
    #[serde(default)]
    pub name: String,
}

impl Author {
    pub fn new_id() -> String {
        uuid::Uuid::new_v4().hyphenated().to_string()
    }

    /// An author id as stored: hyphenated lowercase UUID, if `s` is one.
    pub fn parse_id(s: &str) -> Option<String> {
        uuid::Uuid::parse_str(s.trim()).ok().map(|u| u.hyphenated().to_string())
    }
}

impl Outline {
    /// The author name shared packages carry: the one set in Settings, else the OS's.
    pub fn author_name(&self) -> String {
        let name = self.config.author.name.trim();
        if name.is_empty() {
            crate::app::platform::full_user_name()
        } else {
            name.to_string()
        }
    }

    /// Give this install an author id if it has none (first launch).
    pub fn ensure_author(&mut self) {
        match Author::parse_id(&self.config.author.id) {
            Some(id) if id == self.config.author.id => {}
            Some(id) => {
                self.config.author.id = id;
                self.save_config();
            }
            None => {
                self.config.author.id = Author::new_id();
                self.save_config();
            }
        }
    }

    /// Typing in the author name field: saved as it changes.
    pub fn set_author_name(&mut self, name: String) {
        self.config.author.name = name;
        self.save_config();
    }

    pub(super) fn render_general(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let scroll = self.settings.as_ref().map(|s| s.scroll.clone()).unwrap_or_default();
        let theme = self.theme;
        let editing = self.edit.as_ref().is_some_and(|e| e.target == Target::Field(Field::AuthorName));
        let name = self.config.author.name.clone();
        let inner = if editing {
            let (font, _) = self.base_style(window, false);
            let run =
                gpui::TextRun { len: name.len(), font, color: theme.text, background_color: None, underline: None, strikethrough: None };
            let ed = self.editor_element(name, vec![run], theme.caret, theme.selection, cx);
            self.editor_mouse(cx).child(ed).into_any_element()
        } else if name.trim().is_empty() {
            div().text_color(theme.faint).child(self.author_name()).into_any_element()
        } else {
            div().text_color(theme.text).child(name).into_any_element()
        };
        let name_field = crate::ui::controls::search_field(theme, "author-name", editing)
            .w(px(260.))
            .child(div().flex_1().min_w_0().child(inner))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    if this.edit.as_ref().map(|e| e.target) != Some(Target::Field(Field::AuthorName)) {
                        let len = this.config.author.name.len();
                        this.start_edit(Target::Field(Field::AuthorName), len, cx);
                    }
                }),
            );
        let id_label = self.label("author-id", self.config.author.id.clone(), cx);
        let id_row = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().font_family(crate::app::platform::MONO_FONT).text_size(px(12.)).child(id_label))
            .child(
                crate::ui::button::button(theme, "author-id-copy", crate::ui::button::Variant::Link)
                    .child(t!("settings.general.author_id.copy"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(this.config.author.id.clone()));
                        this.toast(t!("settings.general.author_id.copied"));
                        cx.notify();
                    })),
            )
            .child(
                crate::ui::button::button(theme, "author-id-paste", crate::ui::button::Variant::Link)
                    .child(t!("settings.general.author_id.paste"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        let pasted = cx.read_from_clipboard().and_then(|c| c.text()).and_then(|t| Author::parse_id(&t));
                        match pasted {
                            Some(id) => {
                                this.config.author.id = id;
                                this.save_config();
                                this.toast(t!("settings.general.author_id.pasted"));
                            }
                            None => this.toast(t!("settings.general.author_id.invalid")),
                        }
                        cx.notify();
                    })),
            );
        let on = self.config.whats_new_after_updates;
        let whats_new = div()
            .id("whats-new-after-updates")
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .child(crate::ui::controls::checkbox(theme, on))
            .child(t!("settings.general.whats_new.toggle"))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.config.whats_new_after_updates = !on;
                this.save_config();
                cx.notify();
            }));
        let field = |label: &'static str, hint: String, control: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .py_3()
                .border_b_1()
                .border_color(theme.divider)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(label))
                        .child(div().text_size(px(12.)).text_color(theme.muted).child(hint)),
                )
                .child(control)
        };
        div()
            .id("general")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&scroll)
            .px_4()
            .pt_3()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.end_field_edit() {
                        cx.notify();
                    }
                }),
            )
            .child(crate::ui::text::page_title(theme, t!("settings.general.title"), None).pb_1())
            .child(field(
                t!("settings.general.author_name.label"),
                t!("settings.general.author_name.hint", os = crate::app::platform::name()),
                name_field.into_any_element(),
            ))
            .child(field(t!("settings.general.author_id.label"), t!("settings.general.author_id.hint").into(), id_row.into_any_element()))
            .child(field(
                t!("settings.general.whats_new.label"),
                t!("settings.general.whats_new.hint").into(),
                whats_new.into_any_element(),
            ))
            .into_any_element()
    }
}
