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
/// Breathing border for a loading pane, dark teal to bright cyan.
pub const PULSE: [Color; 8] = [
    Color::Rgb(32, 64, 72),
    Color::Rgb(37, 83, 92),
    Color::Rgb(43, 102, 113),
    Color::Rgb(48, 121, 133),
    Color::Rgb(53, 140, 153),
    Color::Rgb(58, 159, 173),
    Color::Rgb(64, 178, 194),
    Color::Rgb(69, 197, 214),
];
/// Marching ghost-card border, dark gold to bright yellow.
pub const GHOST_RAMP: [Color; 5] = [
    Color::Rgb(74, 60, 16),
    Color::Rgb(107, 87, 22),
    Color::Rgb(140, 113, 27),
    Color::Rgb(186, 152, 42),
    Color::Rgb(232, 197, 61),
];
/// Skeleton and shimmer grays: dark, mid, bright.
pub const SHIM: [Color; 3] = [
    Color::Rgb(44, 52, 57),
    Color::Rgb(66, 85, 91),
    Color::Rgb(140, 158, 164),
];
