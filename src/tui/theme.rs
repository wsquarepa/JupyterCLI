//! Fixed dark palette adopted from LibLLM's defaults (user decision: no color
//! customization, so plain constants instead of a theme struct).

use ratatui::style::Color;

pub const BORDER_FOCUSED: Color = Color::Cyan;
pub const BORDER_UNFOCUSED: Color = Color::DarkGray;
pub const SELECTION_FG: Color = Color::Black;
pub const SELECTION_BG: Color = Color::Cyan;
pub const STATUS_BAR_FG: Color = Color::White;
pub const STATUS_BAR_BG: Color = Color::DarkGray;
pub const STATUS_INFO_FG: Color = Color::White;
pub const STATUS_INFO_BG: Color = Color::Blue;
pub const STATUS_ERROR_FG: Color = Color::White;
pub const STATUS_ERROR_BG: Color = Color::Red;
pub const DIMMED: Color = Color::DarkGray;
pub const DIALOG: Color = Color::Yellow;
pub const SPINNER: Color = Color::Yellow;
pub const STATE_READY: Color = Color::Green;
pub const STATE_PENDING: Color = Color::Yellow;
pub const STATE_STOPPED: Color = Color::DarkGray;
