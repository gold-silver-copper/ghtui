//! Terminal capability detection.

use std::time::Duration;

use crate::color::Rgb;
use crate::{ColorDepth, Mode};

/// Asks the terminal for its background color (OSC 11).
///
/// Must run before entering the alternate screen and raw mode. Terminals that
/// don't support the query are usually detected immediately (via a DA1
/// request); otherwise we give up after `timeout`. Returns `None` when the
/// terminal doesn't answer, including under GNU Screen and `TERM=dumb`.
pub fn detect_background(timeout: Duration) -> Option<Rgb> {
    let mut options = terminal_colorsaurus::QueryOptions::default();
    options.timeout = timeout;
    match terminal_colorsaurus::background_color(options) {
        Ok(c) => {
            let high = |v: u16| v.to_be_bytes()[0];
            Some(Rgb::new(high(c.r), high(c.g), high(c.b)))
        }
        Err(err) => {
            tracing::debug!(%err, "terminal background query failed");
            None
        }
    }
}

/// Dark backgrounds get the dark scheme. Uses HCT tone (perceptual lightness),
/// splitting at the midpoint.
pub fn mode_for_background(bg: Rgb) -> Mode {
    if bg.to_hct().get_tone() < 50.0 {
        Mode::Dark
    } else {
        Mode::Light
    }
}

/// Truecolor if `COLORTERM` advertises it, else 256 colors.
pub fn detect_color_depth(env: impl Fn(&str) -> Option<String>) -> ColorDepth {
    match env("COLORTERM")
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("truecolor" | "24bit") => ColorDepth::TrueColor,
        _ => ColorDepth::Ansi256,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_from_background() {
        assert_eq!(mode_for_background(Rgb::from_u32(0x1e1e1e)), Mode::Dark);
        assert_eq!(mode_for_background(Rgb::from_u32(0xfdf6e3)), Mode::Light);
        assert_eq!(mode_for_background(Rgb::from_u32(0x002b36)), Mode::Dark);
    }

    #[test]
    fn color_depth_from_env() {
        let env = |v: &'static str| move |k: &str| (k == "COLORTERM").then(|| v.to_owned());
        assert_eq!(detect_color_depth(env("truecolor")), ColorDepth::TrueColor);
        assert_eq!(detect_color_depth(env("24BIT")), ColorDepth::TrueColor);
        assert_eq!(detect_color_depth(env("")), ColorDepth::Ansi256);
        assert_eq!(detect_color_depth(|_| None), ColorDepth::Ansi256);
    }
}
