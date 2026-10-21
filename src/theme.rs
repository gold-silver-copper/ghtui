//! Material 3 color schemes adapted to the terminal, behind semantic roles.
//!
//! Views never touch raw colors. They ask for `theme.style(fg, bg)` with a
//! foreground role and the background role it sits on. Every combination the
//! UI may use is declared in [`requirement`]; debug builds panic on an
//! undeclared pair, and the contrast test checks every declared pair, so the
//! test can't drift from real usage.
//!
//! Colors are generated from one seed via HCT tonal palettes. Foreground
//! tones are then nudged (never backgrounds) until each foreground meets its
//! contrast requirement against every background it's declared on, measured
//! at the active color depth. In 256-color mode that measurement uses the
//! quantized palette colors that will actually be displayed.

pub mod color;
mod detect;
mod labels;

use std::cell::RefCell;
use std::collections::HashMap;

use material_colors::blend::harmonize;
use material_colors::color::Rgb as McRgb;
use material_colors::dynamic_color::Variant;
use material_colors::theme::ThemeBuilder;
use ratatui::style::{Color, Modifier, Style};

pub use color::{Rgb, contrast};
pub use detect::{detect_background, detect_color_depth, mode_for_background};

/// Default seed: a muted cobalt that reads well in both schemes.
pub const DEFAULT_SEED: Rgb = Rgb::from_u32(0x3f6fb5);

/// Shown for a role without a color. `Theme::new` fills every role, so this
/// should never appear; if it does, it's loud.
const MISSING: Rgb = Rgb::from_u32(0xff00ff);

/// Minimum contrast for text.
pub const TEXT_CONTRAST: f64 = 4.5;
/// Minimum contrast for decorative outlines; they only need to be visible.
pub const DECORATIVE_CONTRAST: f64 = 1.2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Light,
    Dark,
}

impl Mode {
    /// Tones of a tinted container, and of text on it.
    fn container_tones(self) -> (f64, f64) {
        match self {
            Mode::Dark => (30.0, 90.0),
            Mode::Light => (90.0, 10.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorDepth {
    #[serde(alias = "24bit")]
    TrueColor,
    #[serde(rename = "256")]
    Ansi256,
}

/// Background roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bg {
    /// Unfocused panes, diff context lines.
    Surface,
    /// Focused pane.
    ContainerLow,
    /// Top bar and status bar.
    Container,
    /// Popups, command palette, dialogs.
    ContainerHigh,
    ContainerHighest,
    /// Selected row in the focused pane (state layer over `ContainerLow`).
    Selected,
    /// Selected row in an unfocused pane (state layer over `Surface`).
    SelectedInactive,
    /// Selected row in a popup (state layer over `ContainerHigh`).
    SelectedHigh,
    /// Filled button.
    Primary,
    PrimaryContainer,
    /// Active tab pill.
    SecondaryContainer,
    TertiaryContainer,
    ErrorContainer,
    SuccessContainer,
    Diff(DiffBg),
    /// Cursor line in the diff (state layer over the diff background).
    DiffSelected(DiffBg),
}

words! {
    #[derive(Hash)]
    pub enum DiffBg {}
    Context;
    Added;
    Removed;
    AddedToken;
    RemovedToken;
    Moved;
}

/// Foreground roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Fg {
    OnSurface,
    OnSurfaceVariant,
    Primary,
    Tertiary,
    Error,
    Success,
    /// Exempt from contrast requirements, as WCAG allows for inactive UI.
    Disabled,
    /// Separators; only used where tone alone can't separate regions.
    OutlineVariant,
    OnPrimary,
    OnPrimaryContainer,
    OnSecondaryContainer,
    OnTertiaryContainer,
    OnErrorContainer,
    OnSuccessContainer,
    /// `+` markers and addition counts.
    DiffAddedSign,
    /// `-` markers and deletion counts.
    DiffRemovedSign,
    Syntax(Syntax),
    /// A day in the contribution graph, by level: 0 (none) to 4 (most).
    Heat(u8),
}

words! {
    #[derive(Hash)]
    pub enum Syntax {}
    Default;
    Keyword;
    String;
    Comment;
    Function;
    Type;
    Number;
    Constant;
    Operator;
    Punctuation;
    Attribute;
    Property;
}

impl Bg {
    /// Plain UI surfaces that regular text may sit on.
    const NEUTRAL: [Bg; 8] = [
        Bg::Surface,
        Bg::ContainerLow,
        Bg::Container,
        Bg::ContainerHigh,
        Bg::ContainerHighest,
        Bg::Selected,
        Bg::SelectedInactive,
        Bg::SelectedHigh,
    ];

    /// Filled backgrounds, each with the one text role that goes on it.
    const FILLED: [(Bg, Fg); 6] = [
        (Bg::Primary, Fg::OnPrimary),
        (Bg::PrimaryContainer, Fg::OnPrimaryContainer),
        (Bg::SecondaryContainer, Fg::OnSecondaryContainer),
        (Bg::TertiaryContainer, Fg::OnTertiaryContainer),
        (Bg::ErrorContainer, Fg::OnErrorContainer),
        (Bg::SuccessContainer, Fg::OnSuccessContainer),
    ];

    /// Where this role is in [`Bg::all`].
    fn slot(self) -> Option<usize> {
        let plain = Bg::NEUTRAL.len() + Bg::FILLED.len();
        match self {
            Bg::Diff(d) => Some(plain + d as usize),
            Bg::DiffSelected(d) => Some(plain + DiffBg::ALL.len() + d as usize),
            bg => (Bg::NEUTRAL.iter().position(|n| *n == bg)).or_else(|| {
                let filled = Bg::FILLED.iter().position(|(f, _)| *f == bg)?;
                Some(Bg::NEUTRAL.len() + filled)
            }),
        }
    }

    pub fn all() -> Vec<Bg> {
        let mut all = Bg::NEUTRAL.to_vec();
        all.extend(Bg::FILLED.map(|(bg, _)| bg));
        all.extend(DiffBg::ALL.map(Bg::Diff));
        all.extend(DiffBg::ALL.map(Bg::DiffSelected));
        all
    }

    fn is_neutral(self) -> bool {
        Bg::NEUTRAL.contains(&self)
    }

    fn is_diff(self) -> bool {
        matches!(self, Bg::Diff(_) | Bg::DiffSelected(_))
    }
}

impl Fg {
    const PLAIN: [Fg; 16] = [
        Fg::OnSurface,
        Fg::OnSurfaceVariant,
        Fg::Primary,
        Fg::Tertiary,
        Fg::Error,
        Fg::Success,
        Fg::Disabled,
        Fg::OutlineVariant,
        Fg::OnPrimary,
        Fg::OnPrimaryContainer,
        Fg::OnSecondaryContainer,
        Fg::OnTertiaryContainer,
        Fg::OnErrorContainer,
        Fg::OnSuccessContainer,
        Fg::DiffAddedSign,
        Fg::DiffRemovedSign,
    ];

    /// Where this role is in [`Fg::all`].
    fn slot(self) -> Option<usize> {
        let syntax = Fg::PLAIN.len();
        match self {
            Fg::Syntax(role) => Some(syntax + role as usize),
            Fg::Heat(level) => (HEAT_LEVELS.iter().position(|l| *l == level))
                .map(|i| syntax + Syntax::ALL.len() + i),
            fg => Fg::PLAIN.iter().position(|p| *p == fg),
        }
    }

    pub fn all() -> Vec<Fg> {
        let mut all = Fg::PLAIN.to_vec();
        all.extend(Syntax::ALL.map(Fg::Syntax));
        all.extend(HEAT_LEVELS.map(Fg::Heat));
        all
    }
}

/// The contribution graph's levels.
pub const HEAT_LEVELS: [u8; 5] = [0, 1, 2, 3, 4];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    /// WCAG AA for text: 4.5:1. Applies to titles too (stricter than the
    /// 3:1 WCAG allows for large text).
    Text,
    /// Must be visible, not readable.
    Decorative,
    /// Disabled UI; no requirement.
    Exempt,
}

impl Requirement {
    pub fn min_ratio(self) -> f64 {
        match self {
            Requirement::Text => TEXT_CONTRAST,
            Requirement::Decorative => DECORATIVE_CONTRAST,
            Requirement::Exempt => 1.0,
        }
    }
}

/// The declared fg/bg pairs and what each must meet. `None` means the pair
/// must never be rendered.
pub fn requirement(fg: Fg, bg: Bg) -> Option<Requirement> {
    use Requirement::{Decorative, Exempt, Text};
    match fg {
        Fg::OnSurface | Fg::OnSurfaceVariant | Fg::DiffAddedSign | Fg::DiffRemovedSign
            if bg.is_neutral() || bg.is_diff() =>
        {
            Some(Text)
        }
        Fg::Primary | Fg::Tertiary | Fg::Error | Fg::Success if bg.is_neutral() => Some(Text),
        // Syntax, and reviewed and comment-thread marks in the diff gutter.
        Fg::Success | Fg::Primary | Fg::Tertiary | Fg::Syntax(_) if bg.is_diff() => Some(Text),
        // Also line numbers GitHub won't accept comments on.
        Fg::Disabled if bg.is_neutral() || bg.is_diff() => Some(Exempt),
        Fg::OutlineVariant if bg.is_neutral() => Some(Decorative),
        // Graph cells: shades, not text (each must differ; see `Theme::new`).
        Fg::Heat(_) if bg.is_neutral() => Some(Exempt),
        _ if Bg::FILLED.contains(&(bg, fg)) => Some(Text),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub mode: Mode,
    pub depth: ColorDepth,
    /// Each role's color, by its slot.
    bg: Vec<Rgb>,
    fg: Vec<Rgb>,
    /// The same as the terminal is sent them: what [`Theme::style`] looks
    /// up (finding a 256-color one is slow).
    bg_term: Vec<Color>,
    fg_term: Vec<Color>,
}

impl Theme {
    pub fn new(seed: Rgb, mode: Mode, depth: ColorDepth) -> Self {
        let material = ThemeBuilder::with_source(seed.into())
            .variant(Variant::TonalSpot)
            .build();
        let scheme = match mode {
            Mode::Light => material.schemes.light,
            Mode::Dark => material.schemes.dark,
        };
        let rgb = |c: McRgb| Rgb::from(c);
        let hue_of = |hex| {
            rgb(harmonize(McRgb::from_u32(hex), seed.into()))
                .to_hct()
                .get_hue()
        };
        let green = hue_of(0x2da44e);
        let red = hue_of(0xcf222e);
        let orange = hue_of(0xd18616);
        let primary_hue = rgb(scheme.primary).to_hct().get_hue();
        let tertiary_hue = rgb(scheme.tertiary).to_hct().get_hue();

        // Tones for "colored text on this scheme's surfaces" and for tinted
        // backgrounds that stay close to the surface.
        let (text_tone, diff_tone, diff_token_tone) = match mode {
            Mode::Dark => (80.0, 14.0, 24.0),
            Mode::Light => (40.0, 95.0, 86.0),
        };
        let (container_tone, on_container_tone) = mode.container_tones();
        let hct = |hue: f64, chroma: f64, tone: f64| Rgb::from_hct(hue, chroma, tone);

        let surface = rgb(scheme.surface);
        let container_low = rgb(scheme.surface_container_low);
        let container_high = rgb(scheme.surface_container_high);
        let diff = |d: DiffBg| match d {
            DiffBg::Context => surface,
            DiffBg::Added => hct(green, 16.0, diff_tone),
            DiffBg::Removed => hct(red, 16.0, diff_tone),
            DiffBg::AddedToken => hct(green, 30.0, diff_token_tone),
            DiffBg::RemovedToken => hct(red, 30.0, diff_token_tone),
            DiffBg::Moved => hct(tertiary_hue, 16.0, diff_tone),
        };

        // State layers: primary composited over the base surface. In 256-color
        // mode, raise the opacity until the selection is actually visible.
        let primary = rgb(scheme.primary);
        let layer = |base: Rgb, mut opacity: f64| -> Rgb {
            let shown = displayed(depth, base);
            loop {
                let c = base.blend(primary, opacity);
                if opacity >= 0.4 || displayed(depth, c) != shown {
                    return c;
                }
                opacity += 0.02;
            }
        };
        let bg_roles = Bg::all();
        let bg = bg_roles.iter().map(|role| match *role {
            Bg::Surface => surface,
            Bg::ContainerLow => container_low,
            Bg::Container => rgb(scheme.surface_container),
            Bg::ContainerHigh => container_high,
            Bg::ContainerHighest => rgb(scheme.surface_container_highest),
            Bg::Selected => layer(container_low, 0.12),
            Bg::SelectedInactive => layer(surface, 0.08),
            Bg::SelectedHigh => layer(container_high, 0.14),
            Bg::Primary => primary,
            Bg::PrimaryContainer => rgb(scheme.primary_container),
            Bg::SecondaryContainer => rgb(scheme.secondary_container),
            Bg::TertiaryContainer => rgb(scheme.tertiary_container),
            Bg::ErrorContainer => rgb(scheme.error_container),
            Bg::SuccessContainer => hct(green, 36.0, container_tone),
            Bg::Diff(d) => diff(d),
            Bg::DiffSelected(d) => layer(diff(d), 0.14),
        });
        let bg: Vec<Rgb> = bg.collect();
        // As displayed, measured against once for every foreground.
        let bg_shown: Vec<(Bg, Rgb)> = (bg_roles.into_iter().zip(&bg))
            .map(|(role, &c)| (role, displayed(depth, c)))
            .collect();

        // The contribution graph: a faint surface shade, then the primary
        // hue from faint to strong. Each level must differ from the last (and
        // the first from the page) once quantized, so tones step on until
        // they do.
        let (heat_tones, step) = match mode {
            Mode::Dark => ([22.0, 34.0, 48.0, 64.0, 82.0], 3.0),
            Mode::Light => ([90.0, 80.0, 66.0, 50.0, 34.0], -3.0),
        };
        let mut previous = container_low;
        let heat: Vec<Rgb> = HEAT_LEVELS
            .into_iter()
            .zip(heat_tones)
            .map(|(level, tone)| {
                let chroma = if level == 0 { 6.0 } else { 48.0 };
                let mut tone: f64 = tone;
                let mut color = hct(primary_hue, chroma, tone);
                let below = displayed(depth, previous);
                while displayed(depth, color) == below && (0.0..=100.0).contains(&tone) {
                    tone += step;
                    color = hct(primary_hue, chroma, tone);
                }
                previous = color;
                color
            })
            .collect();

        let on_surface = rgb(scheme.on_surface);
        let on_surface_variant = rgb(scheme.on_surface_variant);
        let fg = Fg::all().into_iter().map(|role| {
            let color = match role {
                Fg::OnSurface => on_surface,
                Fg::OnSurfaceVariant => on_surface_variant,
                Fg::Primary => primary,
                Fg::Tertiary => rgb(scheme.tertiary),
                Fg::Error => rgb(scheme.error),
                Fg::Success | Fg::DiffAddedSign => hct(green, 48.0, text_tone),
                Fg::Disabled => surface.blend(on_surface, 0.38),
                Fg::OutlineVariant => rgb(scheme.outline_variant),
                Fg::OnPrimary => rgb(scheme.on_primary),
                Fg::OnPrimaryContainer => rgb(scheme.on_primary_container),
                Fg::OnSecondaryContainer => rgb(scheme.on_secondary_container),
                Fg::OnTertiaryContainer => rgb(scheme.on_tertiary_container),
                Fg::OnErrorContainer => rgb(scheme.on_error_container),
                Fg::OnSuccessContainer => hct(green, 36.0, on_container_tone),
                Fg::DiffRemovedSign => hct(red, 48.0, text_tone),
                // Keywords primary, strings tertiary, comments and
                // punctuation on-surface-variant, the rest spread around
                // the seed.
                Fg::Syntax(syntax) => match syntax {
                    Syntax::Default => on_surface,
                    Syntax::Keyword => hct(primary_hue, 52.0, text_tone),
                    Syntax::String => hct(tertiary_hue, 40.0, text_tone),
                    Syntax::Comment | Syntax::Operator | Syntax::Punctuation => on_surface_variant,
                    Syntax::Function => hct(primary_hue - 50.0, 36.0, text_tone),
                    Syntax::Type => hct(tertiary_hue + 50.0, 36.0, text_tone),
                    Syntax::Number => hct(orange, 48.0, text_tone),
                    Syntax::Constant => hct(orange, 40.0, text_tone),
                    Syntax::Attribute => hct(primary_hue + 180.0, 32.0, text_tone),
                    Syntax::Property => hct(primary_hue, 16.0, text_tone),
                },
                Fg::Heat(level) => heat.get(usize::from(level)).copied().unwrap_or(MISSING),
            };
            enforce_contrast(depth, &bg_shown, role, color)
        });
        let fg: Vec<Rgb> = fg.collect();

        Theme {
            mode,
            depth,
            bg_term: bg.iter().map(|&c| term_color(depth, c)).collect(),
            fg_term: fg.iter().map(|&c| term_color(depth, c)).collect(),
            bg,
            fg,
        }
    }

    /// The color shown on screen for a design color, after quantization.
    pub fn displayed(&self, c: Rgb) -> Rgb {
        displayed(self.depth, c)
    }

    pub fn bg_rgb(&self, bg: Bg) -> Rgb {
        slot_of(&self.bg, bg.slot()).unwrap_or(MISSING)
    }

    pub fn fg_rgb(&self, fg: Fg) -> Rgb {
        slot_of(&self.fg, fg.slot()).unwrap_or(MISSING)
    }

    pub fn bg_color(&self, bg: Bg) -> Color {
        slot_of(&self.bg_term, bg.slot()).unwrap_or_else(|| term_color(self.depth, MISSING))
    }

    pub fn fg_color(&self, fg: Fg) -> Color {
        slot_of(&self.fg_term, fg.slot()).unwrap_or_else(|| term_color(self.depth, MISSING))
    }

    /// The style for `fg` text on `bg`. Panics in debug builds if the pair
    /// isn't declared in [`requirement`].
    pub fn style(&self, fg: Fg, bg: Bg) -> Style {
        debug_assert!(
            requirement(fg, bg).is_some(),
            "undeclared theme pair: {fg:?} on {bg:?}"
        );
        Style::new().fg(self.fg_color(fg)).bg(self.bg_color(bg))
    }

    /// A background fill with the default text color for that surface.
    pub fn fill(&self, bg: Bg) -> Style {
        let fg = match bg {
            Bg::Diff(_) | Bg::DiffSelected(_) => Fg::Syntax(Syntax::Default),
            _ => Bg::FILLED
                .iter()
                .find(|(filled, _)| *filled == bg)
                .map_or(Fg::OnSurface, |&(_, on)| on),
        };
        self.style(fg, bg)
    }

    pub fn title(&self, bg: Bg) -> Style {
        self.style(Fg::OnSurface, bg).add_modifier(Modifier::BOLD)
    }

    pub fn body(&self, bg: Bg) -> Style {
        self.style(Fg::OnSurface, bg)
    }

    pub fn meta(&self, bg: Bg) -> Style {
        self.style(Fg::OnSurfaceVariant, bg)
    }

    pub fn accent(&self, bg: Bg) -> Style {
        self.style(Fg::Primary, bg)
    }

    pub fn error(&self, bg: Bg) -> Style {
        self.style(Fg::Error, bg)
    }

    pub fn separator(&self, bg: Bg) -> Style {
        self.style(Fg::OutlineVariant, bg)
    }

    /// Whether two background roles render identically at this color depth,
    /// meaning a boundary between them needs an outline instead of tone.
    pub fn tones_collapse(&self, a: Bg, b: Bg) -> bool {
        self.bg_color(a) == self.bg_color(b)
    }

    /// Whether added/removed lines can't be told from context by background
    /// alone; the diff view then colors its gutter markers more heavily.
    pub fn diff_tints_collapse(&self) -> bool {
        let context = Bg::Diff(DiffBg::Context);
        self.tones_collapse(context, Bg::Diff(DiffBg::Added))
            || self.tones_collapse(context, Bg::Diff(DiffBg::Removed))
    }

    /// Chip style for a GitHub label color (`rrggbb`). Keeps the label's hue,
    /// moves it to a container tone for this scheme and picks a readable
    /// foreground. Unparseable colors fall back to the secondary container.
    pub fn label_chip(&self, github_hex: &str) -> Style {
        thread_local! {
            /// Chips drawn so far: finding a readable one takes many tone
            /// steps, and the same labels are drawn every frame.
            static CHIPS: RefCell<HashMap<(Rgb, Mode, ColorDepth), Style>> =
                RefCell::new(HashMap::new());
        }
        let Some(color) = Rgb::parse_hex(github_hex) else {
            return self.fill(Bg::SecondaryContainer);
        };
        CHIPS.with_borrow_mut(|chips| {
            *chips
                .entry((color, self.mode, self.depth))
                .or_insert_with(|| {
                    let (bg, fg) = labels::chip_colors(color, self);
                    let color = |c| term_color(self.depth, c);
                    Style::new().fg(color(fg)).bg(color(bg))
                })
        })
    }
}

/// The color in `slot` of `colors`, a role's place in [`Bg::all`] or
/// [`Fg::all`].
fn slot_of<T: Copy>(colors: &[T], slot: Option<usize>) -> Option<T> {
    colors.get(slot?).copied()
}

fn term_color(depth: ColorDepth, c: Rgb) -> Color {
    match depth {
        ColorDepth::TrueColor => Color::Rgb(c.r, c.g, c.b),
        ColorDepth::Ansi256 => Color::Indexed(color::nearest_xterm(c)),
    }
}

fn displayed(depth: ColorDepth, c: Rgb) -> Rgb {
    match depth {
        ColorDepth::TrueColor => c,
        ColorDepth::Ansi256 => color::xterm_color(color::nearest_xterm(c)),
    }
}

/// Moves `role`'s tone (`base`) away from its backgrounds (`bg`, each as
/// displayed) until it meets the strictest requirement against all of them,
/// as displayed.
fn enforce_contrast(depth: ColorDepth, bg: &[(Bg, Rgb)], role: Fg, base: Rgb) -> Rgb {
    let targets: Vec<(Rgb, f64)> = (bg.iter())
        .filter_map(|&(on, shown)| {
            let req = requirement(role, on).filter(|r| *r != Requirement::Exempt)?;
            Some((shown, req.min_ratio()))
        })
        .collect();
    adjust_tone(base, &targets, |c| displayed(depth, c))
}

/// Shifts `base` in HCT tone (keeping hue and, where the gamut allows,
/// chroma) until it meets every `(background, ratio)` target. Moves toward
/// white if the color is lighter than the backgrounds on average, else toward
/// black.
pub(crate) fn adjust_tone(base: Rgb, targets: &[(Rgb, f64)], display: impl Fn(Rgb) -> Rgb) -> Rgb {
    let ok = |c: Rgb| {
        let shown = display(c);
        targets
            .iter()
            .all(|&(bg, ratio)| contrast(shown, bg) >= ratio)
    };
    if targets.is_empty() || ok(base) {
        return base;
    }
    let hct = base.to_hct();
    let mean_bg = targets.iter().map(|(bg, _)| bg.luminance()).sum::<f64>() / targets.len() as f64;
    let step = if base.luminance() >= mean_bg {
        1.0
    } else {
        -1.0
    };
    let mut tone = hct.get_tone();
    loop {
        tone += step;
        if !(0.0..=100.0).contains(&tone) {
            return Rgb::from_hct(hct.get_hue(), hct.get_chroma(), tone.clamp(0.0, 100.0));
        }
        let c = Rgb::from_hct(hct.get_hue(), hct.get_chroma(), tone);
        if ok(c) {
            return c;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One declared pair, resolved to the colors that will be displayed.
    struct Pair {
        fg: Fg,
        bg: Bg,
        requirement: Requirement,
        ratio: f64,
    }

    /// Every declared pair of `theme`.
    fn pairs(theme: &Theme) -> Vec<Pair> {
        let pair = |fg, bg| {
            Some(Pair {
                fg,
                bg,
                requirement: requirement(fg, bg)?,
                ratio: contrast(
                    theme.displayed(theme.fg_rgb(fg)),
                    theme.displayed(theme.bg_rgb(bg)),
                ),
            })
        };
        Fg::all()
            .into_iter()
            .flat_map(|fg| Bg::all().into_iter().filter_map(move |bg| pair(fg, bg)))
            .collect()
    }

    #[test]
    fn each_role_has_its_own_slot() {
        let slots: Vec<_> = Bg::all().into_iter().map(Bg::slot).collect();
        assert_eq!(slots, (0..slots.len()).map(Some).collect::<Vec<_>>());
        let slots: Vec<_> = Fg::all().into_iter().map(Fg::slot).collect();
        assert_eq!(slots, (0..slots.len()).map(Some).collect::<Vec<_>>());
        assert_eq!(Fg::Heat(9).slot(), None);
    }

    fn seeds() -> Vec<Rgb> {
        let mut seeds = vec![
            DEFAULT_SEED,
            Rgb::from_u32(0x6750a4),
            Rgb::from_u32(0x000000),
            Rgb::from_u32(0xffffff),
            Rgb::from_u32(0xff0000),
            Rgb::from_u32(0xffff00),
            Rgb::from_u32(0x00ff00),
        ];
        // A sweep around the hue circle at a typical seed chroma.
        for hue in (0..360).step_by(15) {
            seeds.push(Rgb::from_hct(f64::from(hue), 48.0, 50.0));
        }
        seeds
    }

    /// The contrast test from the spec: every declared pair, both schemes,
    /// both color depths, many seeds.
    #[test]
    fn every_declared_pair_meets_its_requirement() {
        let mut failures = Vec::new();
        for seed in seeds() {
            for mode in [Mode::Light, Mode::Dark] {
                for depth in [ColorDepth::TrueColor, ColorDepth::Ansi256] {
                    let theme = Theme::new(seed, mode, depth);
                    for pair in pairs(&theme) {
                        if pair.ratio < pair.requirement.min_ratio() {
                            failures.push(format!(
                                "seed {seed} {mode:?} {depth:?}: {:?} on {:?} = {:.2} (< {})",
                                pair.fg,
                                pair.bg,
                                pair.ratio,
                                pair.requirement.min_ratio()
                            ));
                        }
                    }
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} failures:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    /// The contribution graph reads in every scheme and depth: each level
    /// shows differently from the one below it, and none as the page.
    #[test]
    fn heat_levels_stay_apart() {
        for seed in seeds() {
            for mode in [Mode::Light, Mode::Dark] {
                for depth in [ColorDepth::TrueColor, ColorDepth::Ansi256] {
                    let theme = Theme::new(seed, mode, depth);
                    let shown = |fg| theme.displayed(theme.fg_rgb(fg));
                    let page = theme.displayed(theme.bg_rgb(Bg::ContainerLow));
                    let levels: Vec<Rgb> = HEAT_LEVELS.map(|l| shown(Fg::Heat(l))).to_vec();
                    assert_ne!(levels[0], page, "seed {seed} {mode:?} {depth:?}");
                    for pair in levels.windows(2) {
                        assert_ne!(
                            pair[0], pair[1],
                            "seed {seed} {mode:?} {depth:?}: {levels:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn syntax_on_diff_backgrounds_is_covered() {
        let theme = Theme::new(DEFAULT_SEED, Mode::Dark, ColorDepth::TrueColor);
        let pairs = pairs(&theme);
        for role in Syntax::ALL {
            for d in DiffBg::ALL {
                for bg in [Bg::Diff(d), Bg::DiffSelected(d)] {
                    assert!(
                        pairs.iter().any(|p| p.fg == Fg::Syntax(role) && p.bg == bg),
                        "{role:?} on {bg:?} missing"
                    );
                }
            }
        }
    }

    /// Only disabled text and the graph's shades (kept apart by
    /// `heat_levels_stay_apart`) skip contrast.
    #[test]
    fn only_disabled_text_and_graph_shades_are_exempt() {
        for fg in Fg::all() {
            for bg in Bg::all() {
                if requirement(fg, bg) == Some(Requirement::Exempt) {
                    assert!(matches!(fg, Fg::Disabled | Fg::Heat(_)), "{fg:?}");
                }
            }
        }
    }

    #[test]
    fn surfaces_step_in_tone() {
        for mode in [Mode::Light, Mode::Dark] {
            let theme = Theme::new(DEFAULT_SEED, mode, ColorDepth::TrueColor);
            let tone = |bg| theme.bg_rgb(bg).to_hct().get_tone();
            let levels = [
                Bg::ContainerLow,
                Bg::Container,
                Bg::ContainerHigh,
                Bg::ContainerHighest,
            ];
            for pair in levels.windows(2) {
                let (a, b) = (tone(pair[0]), tone(pair[1]));
                match mode {
                    Mode::Dark => assert!(a < b, "{mode:?} {:?} {a} !< {:?} {b}", pair[0], pair[1]),
                    Mode::Light => {
                        assert!(a > b, "{mode:?} {:?} {a} !> {:?} {b}", pair[0], pair[1]);
                    }
                }
            }
        }
    }

    #[test]
    fn selection_stays_visible_in_256_colors() {
        for seed in seeds() {
            for mode in [Mode::Light, Mode::Dark] {
                let theme = Theme::new(seed, mode, ColorDepth::Ansi256);
                assert!(!theme.tones_collapse(Bg::Selected, Bg::ContainerLow));
                assert!(!theme.tones_collapse(Bg::SelectedInactive, Bg::Surface));
                assert!(!theme.tones_collapse(Bg::SelectedHigh, Bg::ContainerHigh));
            }
        }
    }

    #[test]
    fn truecolor_tones_never_collapse() {
        let theme = Theme::new(DEFAULT_SEED, Mode::Dark, ColorDepth::TrueColor);
        assert!(!theme.tones_collapse(Bg::Surface, Bg::ContainerLow));
        assert!(!theme.diff_tints_collapse());
    }

    #[test]
    fn diff_backgrounds_are_tinted_not_saturated() {
        for mode in [Mode::Light, Mode::Dark] {
            let theme = Theme::new(DEFAULT_SEED, mode, ColorDepth::TrueColor);
            let surface = theme.bg_rgb(Bg::Surface).to_hct().get_tone();
            let added = theme.bg_rgb(Bg::Diff(DiffBg::Added)).to_hct();
            assert!((added.get_tone() - surface).abs() < 12.0);
            assert!(added.get_chroma() < 24.0);
        }
    }

    #[test]
    #[should_panic(expected = "undeclared theme pair")]
    #[cfg(debug_assertions)]
    fn undeclared_pair_panics_in_debug() {
        let theme = Theme::new(DEFAULT_SEED, Mode::Dark, ColorDepth::TrueColor);
        let _ = theme.style(Fg::Syntax(Syntax::Keyword), Bg::Primary);
    }
}
