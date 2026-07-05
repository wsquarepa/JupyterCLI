//! Pure geometry for the terminal card grid. Both the App state machine and
//! the renderer call these functions, so cursor math and pixels can never
//! disagree about how many cards fit a row.

pub const CARD_WIDTH: u16 = 16;
pub const CARD_HEIGHT: u16 = 6;
/// Blank rows between consecutive card rows.
pub const CARD_VGAP: u16 = 1;
/// The TUI never renders more than this many cards (user decision; the CLI
/// manages terminals beyond it).
pub const DISPLAY_CAP: usize = 999;

/// Width of the server list pane (the 20% column) for a full frame width.
pub fn server_pane_width(frame_width: u16) -> u16 {
    (u32::from(frame_width) * 20 / 100) as u16
}

/// Inner width of the grid pane: the 80% column minus its two border columns.
pub fn grid_inner_width(frame_width: u16) -> u16 {
    frame_width
        .saturating_sub(server_pane_width(frame_width))
        .saturating_sub(2)
}

/// Cards per row: as many as fit leaving at least one gap column everywhere
/// (n cards need 16n + (n+1) columns), never fewer than one.
pub fn columns_for_width(inner_width: u16) -> usize {
    usize::max(1, usize::from(inner_width.saturating_sub(1)) / 17)
}

/// X offsets (relative to the pane's inner left edge) for each card in a row
/// of `columns` cards. There are `columns + 1` gaps including both edges;
/// leftover width beyond the cards spreads as evenly as possible, the
/// leftmost gaps taking one extra column each when it does not divide evenly.
pub fn row_offsets(inner_width: u16, columns: usize) -> Vec<u16> {
    let columns_u16 = columns as u16;
    let leftover = inner_width.saturating_sub(CARD_WIDTH * columns_u16);
    let gaps = columns_u16 + 1;
    let base = leftover / gaps;
    let extra = leftover % gaps;
    let mut offsets: Vec<u16> = Vec::with_capacity(columns);
    let mut x = 0u16;
    for i in 0..columns_u16 {
        x += base + u16::from(i < extra);
        offsets.push(x);
        x += CARD_WIDTH;
    }
    offsets
}

/// Whole card rows that fit an inner pane height. Each row is 6 tall with 1
/// blank row between rows, so k rows need 7k - 1 <= height.
pub fn visible_card_rows(inner_height: u16) -> usize {
    usize::from((inner_height + 1) / (CARD_HEIGHT + CARD_VGAP))
}

/// Peek pane height within the main area: 25% of the main-area height,
/// floored at 3 so the border plus one content line always fit.
pub fn peek_height(main_height: u16) -> u16 {
    ((u32::from(main_height) * 25 / 100) as u16)
        .max(3)
        .min(main_height)
}

/// Card label. Numeric terminado names up to three digits render zero-padded
/// ("7" -> "Terminal 007"); longer numeric names shorten the word to keep the
/// number whole ("1234" -> "Term 1234"); anything else renders as the raw
/// name. The result truncates to the 12 columns available inside a card.
pub fn card_label(name: &str) -> String {
    let numeric = !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit());
    let label = if numeric && name.len() <= 3 {
        format!("Terminal {name:0>3}")
    } else if numeric {
        format!("Term {name}")
    } else {
        name.to_string()
    };
    label.chars().take(12).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_fit_with_minimum_gaps() {
        assert_eq!(columns_for_width(0), 1);
        assert_eq!(columns_for_width(16), 1);
        assert_eq!(columns_for_width(34), 1);
        assert_eq!(columns_for_width(35), 2);
        assert_eq!(columns_for_width(52), 3);
        assert_eq!(columns_for_width(80), 4);
        assert_eq!(columns_for_width(200), 11);
    }

    #[test]
    fn row_offsets_spread_leftover_left_first() {
        // width 40, 2 cards: leftover 8 over 3 gaps = 3, 3, 2
        assert_eq!(row_offsets(40, 2), vec![3, 22]);
        // width 35, 2 cards: leftover 3 over 3 gaps = 1, 1, 1
        assert_eq!(row_offsets(35, 2), vec![1, 18]);
        // exact fit leaves zero gaps
        assert_eq!(row_offsets(32, 2), vec![0, 16]);
        // degenerate: a card wider than the pane still renders at 0
        assert_eq!(row_offsets(10, 1), vec![0]);
    }

    #[test]
    fn visible_rows_and_peek_height() {
        assert_eq!(visible_card_rows(5), 0);
        assert_eq!(visible_card_rows(6), 1);
        assert_eq!(visible_card_rows(12), 1);
        assert_eq!(visible_card_rows(13), 2);
        assert_eq!(peek_height(40), 10);
        assert_eq!(peek_height(8), 3);
    }

    #[test]
    fn card_labels() {
        assert_eq!(card_label("7"), "Terminal 007");
        assert_eq!(card_label("42"), "Terminal 042");
        assert_eq!(card_label("999"), "Terminal 999");
        assert_eq!(card_label("1234"), "Term 1234");
        assert_eq!(card_label("build"), "build");
        assert_eq!(card_label("averyverylongname"), "averyverylon");
    }

    #[test]
    fn pane_split_is_deterministic() {
        assert_eq!(server_pane_width(100), 20);
        assert_eq!(grid_inner_width(100), 78);
        assert_eq!(server_pane_width(80), 16);
        assert_eq!(grid_inner_width(80), 62);
        assert_eq!(grid_inner_width(0), 0);
    }
}
