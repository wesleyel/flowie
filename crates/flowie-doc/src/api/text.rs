//! A bullet's text outside the desktop's editor: its markup runs, to show it formatted, and the
//! format commands, both in UTF-16 offsets.

use flowie_core::{markup, NodeId};

use super::{FormattedText, MarkupRun, RunKind, TextFormat};
use crate::utf16;

/// The markup runs of a bullet's text, for showing it formatted when it isn't being edited
/// (wiki: iOS-Cross-Platform §2.3); the row being edited shows the text as stored.
pub fn markup(text: &str) -> Vec<MarkupRun> {
    utf16::markup_runs(text)
        .map(|(range, run)| {
            let kind = match run.kind {
                markup::Kind::Text => RunKind::Text,
                markup::Kind::Marker => RunKind::Marker,
                markup::Kind::Code => RunKind::Code,
                markup::Kind::Math { display, .. } => RunKind::Math { display },
                markup::Kind::Link { url } => {
                    let bullet = url.strip_prefix("flowie:").and_then(|id| id.parse::<NodeId>().ok()).map(|id| id.to_string());
                    RunKind::Link { url, bullet }
                }
                markup::Kind::Tag => RunKind::Tag,
                markup::Kind::Mention => RunKind::Mention,
                markup::Kind::Date => RunKind::Date,
            };
            MarkupRun {
                start: range.start as u32,
                end: range.end as u32,
                kind,
                bold: run.bold,
                italic: run.italic,
                strike: run.strike,
                highlight: run.highlight,
                color: run.color,
            }
        })
        .collect()
}

/// Format the selection `start..end` (UTF-16) of `text` as the desktop does
/// (`markup::toggle_wrap`, `markup::insert_link`): wrapped in the markup and still selected, or
/// unwrapped when it already is; at a caret, the pair with the caret between. Link makes the
/// selection (or `link`) a link's label and selects its address.
pub fn toggle_format(text: &str, start: u32, end: u32, format: TextFormat) -> FormattedText {
    let (a, b) = (utf16::from_utf16(text, start as usize), utf16::from_utf16(text, end as usize));
    let sel = a.min(b)..a.max(b);
    let edit = match format {
        TextFormat::Bold => markup::toggle_wrap(text, sel, "**", "**"),
        TextFormat::Italic => markup::toggle_wrap(text, sel, "*", "*"),
        TextFormat::Strikethrough => markup::toggle_wrap(text, sel, "~~", "~~"),
        TextFormat::Code => markup::toggle_wrap(text, sel, "`", "`"),
        TextFormat::Highlight => markup::toggle_wrap(text, sel, "==", "=="),
        TextFormat::Link => markup::insert_link(text, sel),
    };
    let text = edit.apply(text);
    let start = utf16::to_utf16(&text, edit.sel.start) as u32;
    let end = utf16::to_utf16(&text, edit.sel.end) as u32;
    FormattedText { text, start, end }
}
