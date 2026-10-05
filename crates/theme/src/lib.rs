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

use std::collections::HashMap;

use material_colors::blend::harmonize;
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
    ContainerLowest,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiffBg {
    Context,
    Added,
    Removed,
    AddedToken,
    RemovedToken,
    Moved,
}

impl DiffBg {
    pub const ALL: [DiffBg; 6] = [
        DiffBg::Context,
        DiffBg::Added,
        DiffBg::Removed,
        DiffBg::AddedToken,
        DiffBg::RemovedToken,
        DiffBg::Moved,
    ];
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Syntax {
    Default,
    Keyword,
    String,
    Comment,
    Function,
    Type,
    Number,
    Constant,
    Operator,
    Punctuation,
    Attribute,
    Property,
}

impl Syntax {
    pub const ALL: [Syntax; 12] = [
        Syntax::Default,
        Syntax::Keyword,
        Syntax::String,
        Syntax::Comment,
        Syntax::Function,
        Syntax::Type,
        Syntax::Number,
        Syntax::Constant,
        Syntax::Operator,
        Syntax::Punctuation,
        Syntax::Attribute,
        Syntax::Property,
    ];
}

impl Bg {
    pub fn all() -> Vec<Bg> {
        let mut all = vec![
            Bg::Surface,
            Bg::ContainerLowest,
            Bg::ContainerLow,
            Bg::Container,
            Bg::ContainerHigh,
            Bg::ContainerHighest,
            Bg::Selected,
            Bg::SelectedInactive,
            Bg::SelectedHigh,
            Bg::Primary,
            Bg::PrimaryContainer,
            Bg::SecondaryContainer,
            Bg::TertiaryContainer,
            Bg::ErrorContainer,
            Bg::SuccessContainer,
        ];
        all.extend(DiffBg::ALL.map(Bg::Diff));
        all.extend(DiffBg::ALL.map(Bg::DiffSelected));
        all
    }

    /// Plain UI surfaces that regular text may sit on.
    fn is_neutral(self) -> bool {
        matches!(
            self,
            Bg::Surface
                | Bg::ContainerLowest
                | Bg::ContainerLow
                | Bg::Container
                | Bg::ContainerHigh
                | Bg::ContainerHighest
                | Bg::Selected
                | Bg::SelectedInactive
                | Bg::SelectedHigh
        )
    }

    fn is_diff(self) -> bool {
        matches!(self, Bg::Diff(_) | Bg::DiffSelected(_))
    }
}

impl Fg {
    pub fn all() -> Vec<Fg> {
        let mut all = vec![
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
        all.extend(Syntax::ALL.map(Fg::Syntax));
        all
    }
}

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
        Fg::OnPrimary if bg == Bg::Primary => Some(Text),
        Fg::OnPrimaryContainer if bg == Bg::PrimaryContainer => Some(Text),
        Fg::OnSecondaryContainer if bg == Bg::SecondaryContainer => Some(Text),
        Fg::OnTertiaryContainer if bg == Bg::TertiaryContainer => Some(Text),
        Fg::OnErrorContainer if bg == Bg::ErrorContainer => Some(Text),
        Fg::OnSuccessContainer if bg == Bg::SuccessContainer => Some(Text),
        _ => None,
    }
}

/// One declared pair, resolved to the colors that will be displayed.
#[derive(Debug, Clone, Copy)]
pub struct Pair {
    pub fg: Fg,
    pub bg: Bg,
    pub requirement: Requirement,
    pub fg_color: Rgb,
    pub bg_color: Rgb,
}

impl Pair {
    pub fn ratio(&self) -> f64 {
        contrast(self.fg_color, self.bg_color)
    }

    pub fn passes(&self) -> bool {
        self.ratio() >= self.requirement.min_ratio()
    }
}

#[derive(Debug, Clone)]
pub struct Theme {
    pub mode: Mode,
    pub depth: ColorDepth,
    bg: HashMap<Bg, Rgb>,
    fg: HashMap<Fg, Rgb>,
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
        let dark = mode == Mode::Dark;
        let rgb = |c: material_colors::color::Rgb| Rgb::from(c);
        let seed_mc: material_colors::color::Rgb = seed.into();
        let hue_of = |hex: u32| {
            Rgb::from(harmonize(
                material_colors::color::Rgb::from_u32(hex),
                seed_mc,
            ))
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
        let text_tone = if dark { 80.0 } else { 40.0 };
        let container_tone = if dark { 30.0 } else { 90.0 };
        let on_container_tone = if dark { 90.0 } else { 10.0 };
        let (diff_tone, diff_token_tone) = if dark { (14.0, 24.0) } else { (95.0, 86.0) };
        let hct = |hue: f64, chroma: f64, tone: f64| Rgb::from_hct(hue, chroma, tone);

        let mut bg = HashMap::new();
        let surface = rgb(scheme.surface);
        let container_low = rgb(scheme.surface_container_low);
        let container_high = rgb(scheme.surface_container_high);
        bg.insert(Bg::Surface, surface);
        bg.insert(Bg::ContainerLowest, rgb(scheme.surface_container_lowest));
        bg.insert(Bg::ContainerLow, container_low);
        bg.insert(Bg::Container, rgb(scheme.surface_container));
        bg.insert(Bg::ContainerHigh, container_high);
        bg.insert(Bg::ContainerHighest, rgb(scheme.surface_container_highest));
        bg.insert(Bg::Primary, rgb(scheme.primary));
        bg.insert(Bg::PrimaryContainer, rgb(scheme.primary_container));
        bg.insert(Bg::SecondaryContainer, rgb(scheme.secondary_container));
        bg.insert(Bg::TertiaryContainer, rgb(scheme.tertiary_container));
        bg.insert(Bg::ErrorContainer, rgb(scheme.error_container));
        bg.insert(Bg::SuccessContainer, hct(green, 36.0, container_tone));

        let diff = |d: DiffBg| match d {
            DiffBg::Context => surface,
            DiffBg::Added => hct(green, 16.0, diff_tone),
            DiffBg::Removed => hct(red, 16.0, diff_tone),
            DiffBg::AddedToken => hct(green, 30.0, diff_token_tone),
            DiffBg::RemovedToken => hct(red, 30.0, diff_token_tone),
            DiffBg::Moved => hct(tertiary_hue, 16.0, diff_tone),
        };
        for d in DiffBg::ALL {
            bg.insert(Bg::Diff(d), diff(d));
        }

        // State layers: primary composited over the base surface. In 256-color
        // mode, raise the opacity until the selection is actually visible.
        let primary = rgb(scheme.primary);
        let layer = |base: Rgb, opacity: f64| -> Rgb {
            let mut opacity = opacity;
            loop {
                let c = base.blend(primary, opacity);
                if opacity >= 0.4 || displayed(depth, c) != displayed(depth, base) {
                    return c;
                }
                opacity += 0.02;
            }
        };
        bg.insert(Bg::Selected, layer(container_low, 0.12));
        bg.insert(Bg::SelectedInactive, layer(surface, 0.08));
        bg.insert(Bg::SelectedHigh, layer(container_high, 0.14));
        for d in DiffBg::ALL {
            bg.insert(Bg::DiffSelected(d), layer(diff(d), 0.14));
        }

        let mut fg = HashMap::new();
        let on_surface = rgb(scheme.on_surface);
        let on_surface_variant = rgb(scheme.on_surface_variant);
        fg.insert(Fg::OnSurface, on_surface);
        fg.insert(Fg::OnSurfaceVariant, on_surface_variant);
        fg.insert(Fg::Primary, primary);
        fg.insert(Fg::Tertiary, rgb(scheme.tertiary));
        fg.insert(Fg::Error, rgb(scheme.error));
        fg.insert(Fg::Success, hct(green, 48.0, text_tone));
        fg.insert(Fg::Disabled, surface.blend(on_surface, 0.38));
        fg.insert(Fg::OutlineVariant, rgb(scheme.outline_variant));
        fg.insert(Fg::OnPrimary, rgb(scheme.on_primary));
        fg.insert(Fg::OnPrimaryContainer, rgb(scheme.on_primary_container));
        fg.insert(Fg::OnSecondaryContainer, rgb(scheme.on_secondary_container));
        fg.insert(Fg::OnTertiaryContainer, rgb(scheme.on_tertiary_container));
        fg.insert(Fg::OnErrorContainer, rgb(scheme.on_error_container));
        fg.insert(Fg::OnSuccessContainer, hct(green, 36.0, on_container_tone));
        fg.insert(Fg::DiffAddedSign, hct(green, 48.0, text_tone));
        fg.insert(Fg::DiffRemovedSign, hct(red, 48.0, text_tone));

        // Syntax: keywords primary, strings tertiary, comments and
        // punctuation on-surface-variant, the rest spread around the seed.
        let syntax = [
            (Syntax::Default, on_surface),
            (Syntax::Keyword, hct(primary_hue, 52.0, text_tone)),
            (Syntax::String, hct(tertiary_hue, 40.0, text_tone)),
            (Syntax::Comment, on_surface_variant),
            (Syntax::Function, hct(primary_hue - 50.0, 36.0, text_tone)),
            (Syntax::Type, hct(tertiary_hue + 50.0, 36.0, text_tone)),
            (Syntax::Number, hct(orange, 48.0, text_tone)),
            (Syntax::Constant, hct(orange, 40.0, text_tone)),
            (Syntax::Operator, on_surface_variant),
            (Syntax::Punctuation, on_surface_variant),
            (Syntax::Attribute, hct(primary_hue + 180.0, 32.0, text_tone)),
            (Syntax::Property, hct(primary_hue, 16.0, text_tone)),
        ];
        for (role, c) in syntax {
            fg.insert(Fg::Syntax(role), c);
        }

        let mut theme = Theme {
            mode,
            depth,
            bg,
            fg,
        };
        for role in Fg::all() {
            let adjusted = enforce_contrast(&theme, role);
            theme.fg.insert(role, adjusted);
        }
        theme
    }

    /// The color shown on screen for a design color, after quantization.
    pub fn displayed(&self, c: Rgb) -> Rgb {
        displayed(self.depth, c)
    }

    fn term_color(&self, c: Rgb) -> Color {
        match self.depth {
            ColorDepth::TrueColor => Color::Rgb(c.r, c.g, c.b),
            ColorDepth::Ansi256 => Color::Indexed(color::nearest_xterm(c)),
        }
    }

    pub fn bg_rgb(&self, bg: Bg) -> Rgb {
        self.bg.get(&bg).copied().unwrap_or(MISSING)
    }

    pub fn fg_rgb(&self, fg: Fg) -> Rgb {
        self.fg.get(&fg).copied().unwrap_or(MISSING)
    }

    pub fn bg_color(&self, bg: Bg) -> Color {
        self.term_color(self.bg_rgb(bg))
    }

    pub fn fg_color(&self, fg: Fg) -> Color {
        self.term_color(self.fg_rgb(fg))
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
            Bg::Primary => Fg::OnPrimary,
            Bg::PrimaryContainer => Fg::OnPrimaryContainer,
            Bg::SecondaryContainer => Fg::OnSecondaryContainer,
            Bg::TertiaryContainer => Fg::OnTertiaryContainer,
            Bg::ErrorContainer => Fg::OnErrorContainer,
            Bg::SuccessContainer => Fg::OnSuccessContainer,
            Bg::Diff(_) | Bg::DiffSelected(_) => Fg::Syntax(Syntax::Default),
            _ => Fg::OnSurface,
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

    pub fn disabled(&self, bg: Bg) -> Style {
        self.style(Fg::Disabled, bg)
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
        self.displayed(self.bg_rgb(a)) == self.displayed(self.bg_rgb(b))
    }

    /// Whether added/removed lines can't be told from context by background
    /// alone; the diff view then colors its gutter markers more heavily.
    pub fn diff_tints_collapse(&self) -> bool {
        let context = Bg::Diff(DiffBg::Context);
        self.tones_collapse(context, Bg::Diff(DiffBg::Added))
            || self.tones_collapse(context, Bg::Diff(DiffBg::Removed))
    }

    /// Every declared pair, resolved to displayed colors.
    pub fn pairs(&self) -> Vec<Pair> {
        let mut pairs = Vec::new();
        for fg in Fg::all() {
            for bg in Bg::all() {
                if let Some(requirement) = requirement(fg, bg) {
                    pairs.push(Pair {
                        fg,
                        bg,
                        requirement,
                        fg_color: self.displayed(self.fg_rgb(fg)),
                        bg_color: self.displayed(self.bg_rgb(bg)),
                    });
                }
            }
        }
        pairs
    }

    /// Chip style for a GitHub label color (`rrggbb`). Keeps the label's hue,
    /// moves it to a container tone for this scheme and picks a readable
    /// foreground. Unparseable colors fall back to the secondary container.
    pub fn label_chip(&self, github_hex: &str) -> Style {
        match Rgb::parse_hex(github_hex) {
            Some(color) => {
                let (bg, fg) = labels::chip_colors(color, self);
                Style::new().fg(self.term_color(fg)).bg(self.term_color(bg))
            }
            None => self.fill(Bg::SecondaryContainer),
        }
    }
}

fn displayed(depth: ColorDepth, c: Rgb) -> Rgb {
    match depth {
        ColorDepth::TrueColor => c,
        ColorDepth::Ansi256 => color::xterm_color(color::nearest_xterm(c)),
    }
}

/// Moves `role`'s tone away from its backgrounds until it meets the strictest
/// requirement against all of them, as displayed.
fn enforce_contrast(theme: &Theme, role: Fg) -> Rgb {
    let base = theme.fg_rgb(role);
    let mut targets: Vec<(Rgb, f64)> = Vec::new();
    for bg in Bg::all() {
        if let Some(req) = requirement(role, bg)
            && req != Requirement::Exempt
        {
            targets.push((theme.displayed(theme.bg_rgb(bg)), req.min_ratio()));
        }
    }
    adjust_tone(base, &targets, |c| theme.displayed(c))
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
                    for pair in theme.pairs() {
                        if !pair.passes() {
                            failures.push(format!(
                                "seed {seed} {mode:?} {depth:?}: {:?} on {:?} = {:.2} (< {})",
                                pair.fg,
                                pair.bg,
                                pair.ratio(),
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

    #[test]
    fn syntax_on_diff_backgrounds_is_covered() {
        let theme = Theme::new(DEFAULT_SEED, Mode::Dark, ColorDepth::TrueColor);
        let pairs = theme.pairs();
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

    #[test]
    fn disabled_is_the_only_exempt_role() {
        for fg in Fg::all() {
            for bg in Bg::all() {
                if requirement(fg, bg) == Some(Requirement::Exempt) {
                    assert_eq!(fg, Fg::Disabled);
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
                Bg::ContainerLowest,
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
