//! Flowie's macOS controls: menus, popovers, sheets, list rows, buttons,
//! segmented controls, checkboxes, key caps and toasts, drawn with GPUI from
//! the tokens in `crate::ui::theme`.
//!
//! Metrics and colors follow gpui-macos (<https://github.com/Rhein-Industries/gpui-macos>,
//! MIT, © gpui-macos contributors and Jason Lee), reduced to the controls
//! Flowie already has. See `LICENSE-gpui-macos` here.
//!
//! Each control is a plain function returning a styled element; callers add
//! children and handlers.

pub mod button;
pub mod controls;
pub mod list;
pub mod menu;
pub mod prompt;
pub mod scrollbar;
pub mod segmented;
pub mod surface;
pub mod text;

pub(crate) mod anim;
pub(crate) mod icons;
pub(crate) mod theme;
