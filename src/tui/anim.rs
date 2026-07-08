//! Cell-locked animation primitives: pure functions of the App frame counter.
//! A terminal has no sub-cell motion, so every animation is a character or
//! color choice per cell per 100ms tick; these functions pick the color level.

use ratatui::style::Color;

use super::theme;

/// Breathing color for a loading pane's border. A full breath is 20 frames
/// (2s at the 100ms tick), sampled from the 8-step ramp by a triangle wave,
/// one keyframe per tick so the border never visibly steps.
pub fn pulse_color(frame: usize) -> Color {
    let cycle = frame % 20;
    let tri = if cycle < 10 { cycle } else { 20 - cycle };
    theme::PULSE[(tri * 7 / 10).min(7)]
}

/// Gradient level (0 dim to 4 bright) for perimeter cell `index` of a
/// `len`-cell border when the marching head sits at `frame * 2 % len`:
/// a bright head with a tail fading back into the base color.
pub fn march_level(frame: usize, index: usize, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let head = (frame * 2) % len;
    let distance = (head + len - index) % len;
    match distance {
        0..2 => 4,
        2..4 => 3,
        4..6 => 2,
        6..9 => 1,
        _ => 0,
    }
}

/// Shimmer shade (0 dark, 1 mid, 2 bright) for a diagonal band sweeping
/// across skeleton content at cell (`x`, `y`) relative to the region.
pub fn shimmer_level(frame: usize, x: u16, y: u16) -> usize {
    let band = (u64::from(x) + u64::from(y) * 2 + 48 - (frame as u64 * 2 % 48)) % 48;
    match band {
        0..2 => 2,
        2..6 => 1,
        _ => 0,
    }
}

/// Deterministic per-cell noise in [0, 1] for the dissolve threshold, so a
/// dissolving card decays organically but identically across repaints.
pub fn cell_noise(seed: u32, x: u16, y: u16) -> f32 {
    let mut n = seed
        .wrapping_add(u32::from(x).wrapping_mul(31))
        .wrapping_add(u32::from(y).wrapping_mul(131));
    n = (n ^ 61) ^ (n >> 16);
    n = n.wrapping_add(n << 3);
    n ^= n >> 4;
    n = n.wrapping_mul(0x27d4_eb2d);
    n ^= n >> 15;
    n as f32 / u32::MAX as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme;

    #[test]
    fn pulse_breathes_over_twenty_frames() {
        assert_eq!(pulse_color(0), theme::PULSE[0]);
        assert_eq!(pulse_color(10), theme::PULSE[7]);
        assert_eq!(pulse_color(20), pulse_color(0));
        // Monotonic rise on the way up.
        for frame in 0..10 {
            let a = theme::PULSE
                .iter()
                .position(|c| *c == pulse_color(frame))
                .unwrap();
            let b = theme::PULSE
                .iter()
                .position(|c| *c == pulse_color(frame + 1))
                .unwrap();
            assert!(b >= a, "frame {frame}: {a} -> {b}");
        }
    }

    #[test]
    fn march_head_is_bright_and_tail_fades() {
        let len = 40;
        let head = (7 * 2) % len;
        assert_eq!(march_level(7, head, len), 4);
        assert_eq!(march_level(7, (head + len - 8) % len, len), 1);
        assert_eq!(march_level(7, (head + len - 20) % len, len), 0);
        assert_eq!(march_level(7, 3, 0), 0, "empty perimeter must not panic");
    }

    #[test]
    fn shimmer_levels_are_in_range_and_move() {
        for frame in 0..48 {
            for x in 0..20u16 {
                assert!(shimmer_level(frame, x, 3) <= 2);
            }
        }
        let before: Vec<usize> = (0..20u16).map(|x| shimmer_level(0, x, 0)).collect();
        let after: Vec<usize> = (0..20u16).map(|x| shimmer_level(5, x, 0)).collect();
        assert_ne!(before, after, "the band must move between frames");
    }

    #[test]
    fn cell_noise_is_deterministic_and_bounded() {
        for x in 0..30u16 {
            for y in 0..10u16 {
                let n = cell_noise(42, x, y);
                assert!((0.0..=1.0).contains(&n));
                assert_eq!(n, cell_noise(42, x, y));
            }
        }
        assert_ne!(cell_noise(42, 1, 1), cell_noise(43, 1, 1));
    }
}
