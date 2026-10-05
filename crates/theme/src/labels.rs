//! GitHub label colors adapted to tonal chips.

use crate::color::Rgb;
use crate::{Mode, TEXT_CONTRAST, Theme, adjust_tone};

/// Chip background and foreground for a label color: same hue, chroma capped
/// so chips don't shout, background at the scheme's container tone, and a
/// foreground nudged until it's readable on that background as displayed.
pub(crate) fn chip_colors(label: Rgb, theme: &Theme) -> (Rgb, Rgb) {
    let hct = label.to_hct();
    let hue = hct.get_hue();
    let chroma = hct.get_chroma().min(40.0);
    let (bg_tone, fg_tone) = match theme.mode {
        Mode::Dark => (30.0, 90.0),
        Mode::Light => (90.0, 10.0),
    };
    let bg = Rgb::from_hct(hue, chroma, bg_tone);
    let fg = Rgb::from_hct(hue, chroma.min(24.0), fg_tone);
    let fg = adjust_tone(fg, &[(theme.displayed(bg), TEXT_CONTRAST)], |c| {
        theme.displayed(c)
    });
    (bg, fg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorDepth, DEFAULT_SEED, contrast};

    #[test]
    fn label_chips_are_readable_for_any_label_color() {
        let mut samples = vec![
            Rgb::from_u32(0x000000),
            Rgb::from_u32(0xffffff),
            Rgb::from_u32(0xd73a4a), // bug
            Rgb::from_u32(0x0075ca), // documentation
            Rgb::from_u32(0xa2eeef), // enhancement
            Rgb::from_u32(0xfef2c0),
            Rgb::from_u32(0x7057ff), // good first issue
        ];
        for r in (0..=255u8).step_by(51) {
            for g in (0..=255u8).step_by(51) {
                for b in (0..=255u8).step_by(51) {
                    samples.push(Rgb::new(r, g, b));
                }
            }
        }
        for mode in [Mode::Light, Mode::Dark] {
            for depth in [ColorDepth::TrueColor, ColorDepth::Ansi256] {
                let theme = Theme::new(DEFAULT_SEED, mode, depth);
                for label in &samples {
                    let (bg, fg) = chip_colors(*label, &theme);
                    let ratio = contrast(theme.displayed(fg), theme.displayed(bg));
                    assert!(
                        ratio >= TEXT_CONTRAST,
                        "{label} {mode:?} {depth:?}: {ratio:.2}"
                    );
                }
            }
        }
    }
}
