//! Color math: WCAG contrast, sRGB compositing, xterm-256 quantization.

use material_colors::color::{Lab, Rgb as McRgb};
use material_colors::hct::Hct;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub const fn from_u32(hex: u32) -> Self {
        let [_, r, g, b] = hex.to_be_bytes();
        Self::new(r, g, b)
    }

    /// Parses `#rrggbb` or `rrggbb`.
    pub fn parse_hex(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('#');
        if s.len() != 6 || !s.is_ascii() {
            return None;
        }
        u32::from_str_radix(s, 16).ok().map(Self::from_u32)
    }

    /// WCAG 2.x relative luminance.
    pub fn luminance(self) -> f64 {
        fn channel(c: u8) -> f64 {
            let c = f64::from(c) / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        0.2126 * channel(self.r) + 0.7152 * channel(self.g) + 0.0722 * channel(self.b)
    }

    /// Composites `over` at `opacity` onto `self` (straight sRGB alpha, like a
    /// Material state layer).
    pub fn blend(self, over: Rgb, opacity: f64) -> Rgb {
        let mix = |a: u8, b: u8| channel(f64::from(a) + (f64::from(b) - f64::from(a)) * opacity);
        Rgb::new(
            mix(self.r, over.r),
            mix(self.g, over.g),
            mix(self.b, over.b),
        )
    }

    pub fn to_hct(self) -> Hct {
        Hct::new(self.into())
    }

    pub fn from_hct(hue: f64, chroma: f64, tone: f64) -> Self {
        Hct::from(hue, chroma, tone).into()
    }
}

impl std::fmt::Display for Rgb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

impl From<McRgb> for Rgb {
    fn from(c: McRgb) -> Self {
        Self::new(c.red, c.green, c.blue)
    }
}

impl From<Rgb> for McRgb {
    fn from(c: Rgb) -> Self {
        McRgb::new(c.r, c.g, c.b)
    }
}

impl From<Hct> for Rgb {
    fn from(h: Hct) -> Self {
        McRgb::from(h).into()
    }
}

/// Rounds to the nearest channel value, saturating; NaN becomes 0. Std has no
/// checked float-to-int conversion, so this searches instead of casting.
fn channel(x: f64) -> u8 {
    let x = x.round();
    (0..=u8::MAX).rfind(|&n| f64::from(n) <= x).unwrap_or(0)
}

/// WCAG contrast ratio, 1.0 to 21.0.
pub fn contrast(a: Rgb, b: Rgb) -> f64 {
    let (la, lb) = (a.luminance(), b.luminance());
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// xterm's defaults for the 16 ANSI colors.
const ANSI: [u32; 16] = [
    0x000000, 0xcd0000, 0x00cd00, 0xcdcd00, 0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5, 0x7f7f7f,
    0xff0000, 0x00ff00, 0xffff00, 0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff,
];

/// The xterm-256 palette entry for an index. The first 16 are user-themed and
/// deliberately never chosen; for them this returns xterm's defaults.
pub fn xterm_color(index: u8) -> Rgb {
    if let Some(&hex) = ANSI.get(usize::from(index)) {
        return Rgb::from_u32(hex);
    }
    let level = |n: u8| if n == 0 { 0 } else { 55 + 40 * n };
    match index {
        16..=231 => {
            let i = index - 16;
            Rgb::new(level(i / 36), level(i / 6 % 6), level(i % 6))
        }
        _ => {
            let v = 8 + 10 * (index - 232);
            Rgb::new(v, v, v)
        }
    }
}

/// Nearest xterm-256 index (16..=255) by CIELAB distance, with lightness
/// errors weighted heavily. The coarse 6×6×6 cube often has no color near a
/// dark or light tint; preferring the right lightness (and dropping chroma)
/// keeps the contrast the truecolor scheme was designed with.
pub fn nearest_xterm(c: Rgb) -> u8 {
    const LIGHTNESS_WEIGHT: f64 = 3.0;
    let target = Lab::from(McRgb::from(c));
    let mut best = (f64::INFINITY, 16u8);
    for index in 16..=255u8 {
        let lab = Lab::from(McRgb::from(xterm_color(index)));
        let d = (LIGHTNESS_WEIGHT * (lab.l - target.l)).powi(2)
            + (lab.a - target.a).powi(2)
            + (lab.b - target.b).powi(2);
        if d < best.0 {
            best = (d, index);
        }
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_extremes() {
        let black = Rgb::new(0, 0, 0);
        let white = Rgb::new(255, 255, 255);
        assert!((contrast(black, white) - 21.0).abs() < 1e-9);
        assert!((contrast(white, white) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn known_contrast_value() {
        // #767676 on white is the classic 4.54:1 gray.
        let ratio = contrast(Rgb::from_u32(0x767676), Rgb::from_u32(0xffffff));
        assert!((ratio - 4.54).abs() < 0.01, "{ratio}");
    }

    #[test]
    fn xterm_round_trip_on_palette_colors() {
        for index in 16..=255u8 {
            assert_eq!(
                xterm_color(nearest_xterm(xterm_color(index))),
                xterm_color(index)
            );
        }
    }

    #[test]
    fn parses_hex() {
        assert_eq!(Rgb::parse_hex("#0e8a16"), Some(Rgb::new(0x0e, 0x8a, 0x16)));
        assert_eq!(Rgb::parse_hex("d73a4a"), Some(Rgb::new(0xd7, 0x3a, 0x4a)));
        assert_eq!(Rgb::parse_hex("xyz"), None);
        assert_eq!(Rgb::parse_hex("ééé"), None);
    }

    #[test]
    fn blend_endpoints() {
        let a = Rgb::new(10, 20, 30);
        let b = Rgb::new(200, 100, 0);
        assert_eq!(a.blend(b, 0.0), a);
        assert_eq!(a.blend(b, 1.0), b);
    }
}
