//! Color palettes of the three window themes.

use super::canvas::Rgb;
use crate::accent;
use crate::settings::Theme;

pub const FONT: &str = "Segoe UI";
pub const FONT_MONO: &str = "Consolas";
pub const FONT_ICONS: &str = "Segoe MDL2 Assets";

/// Used when the cover has no clear color (grayscale art, no cover yet).
const ACCENT_FALLBACK: u32 = 0xf0568e;
const DARK_ACCENT_LIGHTNESS: f32 = 0.64;
const DARK_ACCENT_SATURATION: (f32, f32) = (0.55, 0.9);
/// Darker on the light theme so white text on it stays readable.
const LIGHT_ACCENT_LIGHTNESS: f32 = 0.42;
const LIGHT_ACCENT_SATURATION: (f32, f32) = (0.5, 0.75);
/// WCAG AA for the labels drawn on the accent.
const MIN_CONTRAST: f32 = 4.5;
/// Lightness step while searching for a readable accent.
const LIGHTNESS_STEP: f32 = 0.02;

macro_rules! palette {
    ($($(#[$doc:meta])* $field:ident),* $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub struct Palette {
            $($(#[$doc])* pub $field: Rgb,)*
        }

        impl Palette {
            /// Mixes every color towards `other`.
            pub fn lerp(&self, other: &Self, t: f32) -> Self {
                Self { $($field: self.$field.lerp(other.$field, t),)* }
            }
        }
    };
}

palette! {
    bg,
    surface,
    /// Header separator and the window border.
    line,
    /// Mode control: container, selected pill and its label.
    segment,
    pill,
    pill_text,
    /// Hover fill of the header buttons.
    hover,
    strong,
    text,
    muted,
    dim,
    ok,
    err,
    /// Switches, slider fill, the primary button and the logo.
    accent,
    /// Text and glyphs drawn on the accent.
    on_accent,
    switch_off,
    knob_on,
    knob_off,
    slider_track,
    slider_knob,
    /// Outline of the slider knob; equal to the knob when not needed.
    knob_ring,
    /// Secondary button fill.
    secondary,
}

impl Palette {
    /// `cover_color` is the dominant color of the current picture, if any.
    pub fn new(theme: Theme, cover_color: Option<u32>) -> Self {
        let base = cover_color.unwrap_or(ACCENT_FALLBACK);
        match theme {
            Theme::Graphite => Self::graphite(),
            Theme::Accent => {
                // Blues look dark at the same lightness: brighten until the
                // dark label reads.
                let accent = readable(base, DARK_ACCENT_LIGHTNESS, LIGHTNESS_STEP, DARK_ACCENT_SATURATION, |a| {
                    on_dark_accent(a)
                });
                Self::accent(accent)
            }
            Theme::Pastel => {
                let accent = readable(base, LIGHT_ACCENT_LIGHTNESS, -LIGHTNESS_STEP, LIGHT_ACCENT_SATURATION, |_| {
                    Rgb::hex(0xffffff)
                });
                Self::pastel(accent)
            }
        }
    }

    fn graphite() -> Self {
        Self {
            bg: Rgb::hex(0x0a0a0a),
            surface: Rgb::hex(0x141414),
            line: Rgb::hex(0x1e1e1e),
            segment: Rgb::hex(0x141414),
            pill: Rgb::hex(0x262626),
            pill_text: Rgb::hex(0xf5f5f5),
            hover: Rgb::hex(0x1e1e1e),
            strong: Rgb::hex(0xf5f5f5),
            text: Rgb::hex(0xd4d4d4),
            muted: Rgb::hex(0xa3a3a3),
            dim: Rgb::hex(0x737373),
            ok: Rgb::hex(0x4ade80),
            err: Rgb::hex(0xf87171),
            accent: Rgb::hex(0xffffff),
            on_accent: Rgb::hex(0x000000),
            switch_off: Rgb::hex(0x262626),
            knob_on: Rgb::hex(0x0a0a0a),
            knob_off: Rgb::hex(0x737373),
            slider_track: Rgb::hex(0x262626),
            slider_knob: Rgb::hex(0xffffff),
            knob_ring: Rgb::hex(0xffffff),
            secondary: Rgb::hex(0x1e1e1e),
        }
    }

    fn accent(accent: Rgb) -> Self {
        Self {
            bg: Rgb::hex(0x0f0f13),
            surface: Rgb::hex(0x17171d),
            line: Rgb::hex(0x22222a),
            segment: Rgb::hex(0x17171d),
            pill: Rgb::hex(0x2a2a33),
            pill_text: Rgb::hex(0xffffff),
            hover: Rgb::hex(0x22222a),
            strong: Rgb::hex(0xf4f4f8),
            text: Rgb::hex(0xececf1),
            muted: Rgb::hex(0xb4b4c0),
            dim: Rgb::hex(0x8b8b98),
            ok: Rgb::hex(0x4ade80),
            err: Rgb::hex(0xf87171),
            accent,
            on_accent: on_dark_accent(accent),
            switch_off: Rgb::hex(0x2a2a33),
            knob_on: Rgb::hex(0xffffff),
            knob_off: Rgb::hex(0x8b8b98),
            slider_track: Rgb::hex(0x2a2a33),
            slider_knob: Rgb::hex(0xffffff),
            knob_ring: Rgb::hex(0xffffff),
            secondary: Rgb::hex(0x22222a),
        }
    }

    fn pastel(accent: Rgb) -> Self {
        let white = Rgb::hex(0xffffff);
        let bg = white.lerp(accent, 0.05);
        let soft = bg.lerp(accent, 0.12);
        let dim = Rgb::hex(0x786d72);
        Self {
            bg,
            surface: white,
            line: bg.lerp(dim, 0.15),
            segment: soft,
            pill: white,
            pill_text: accent,
            hover: soft,
            strong: Rgb::hex(0x241c20),
            text: Rgb::hex(0x2e2629),
            muted: Rgb::hex(0x6a5f64),
            dim,
            ok: Rgb::hex(0x1f8a4c),
            err: Rgb::hex(0xc2362f),
            accent,
            on_accent: white,
            switch_off: bg.lerp(dim, 0.2),
            knob_on: white,
            knob_off: white,
            slider_track: soft,
            slider_knob: white,
            knob_ring: bg.lerp(dim, 0.35),
            secondary: white,
        }
    }
}

/// Near-black tinted with the accent, for labels on a bright accent.
fn on_dark_accent(accent: Rgb) -> Rgb {
    accent.lerp(Rgb::hex(0x000000), 0.88)
}

/// Tones `base` starting at `lightness` and moving by `step` until the label
/// color from `label` reaches `MIN_CONTRAST` on it.
fn readable(base: u32, lightness: f32, step: f32, saturation: (f32, f32), label: impl Fn(Rgb) -> Rgb) -> Rgb {
    let mut lightness = lightness;
    loop {
        let accent = Rgb::hex(accent::tone(base, lightness, saturation));
        let next = lightness + step;
        if contrast(accent, label(accent)) >= MIN_CONTRAST || !(0.05..=0.95).contains(&next) {
            return accent;
        }
        lightness = next;
    }
}

fn luminance(color: Rgb) -> f32 {
    let channel = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * channel(color.0) + 0.7152 * channel(color.1) + 0.0722 * channel(color.2)
}

/// WCAG contrast ratio.
fn contrast(a: Rgb, b: Rgb) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

pub fn name(theme: Theme) -> &'static str {
    match theme {
        Theme::Graphite => "Графит",
        Theme::Accent => "Акцент",
        Theme::Pastel => "Пастель",
    }
}

pub fn description(theme: Theme) -> &'static str {
    match theme {
        Theme::Graphite => "Монохром в стиле scarp.cc",
        Theme::Accent => "Тёмная, цвет из обложки",
        Theme::Pastel => "Светлая, цвет из обложки",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COVERS: [Option<u32>; 8] = [
        None,
        Some(0xff0000),
        Some(0xffff00),
        Some(0x00ff00),
        Some(0x00ffff),
        Some(0x0000ff),
        Some(0x7c3aed),
        Some(0x202020),
    ];

    #[test]
    fn text_on_accent_stays_readable_for_any_cover() {
        for theme in Theme::ALL {
            for cover in COVERS {
                let p = Palette::new(theme, cover);
                let ratio = contrast(p.accent, p.on_accent);
                assert!(ratio >= 4.5, "{theme:?} {cover:06x?}: {ratio}");
            }
        }
    }

    #[test]
    fn body_text_is_readable() {
        for theme in Theme::ALL {
            let p = Palette::new(theme, Some(0xe0306a));
            for (name, color) in [("strong", p.strong), ("text", p.text), ("muted", p.muted)] {
                assert!(contrast(color, p.surface) >= 4.5, "{theme:?} {name}");
            }
            assert!(contrast(p.dim, p.bg) >= 3.0, "{theme:?} dim");
        }
    }

    #[test]
    fn graphite_ignores_the_cover() {
        assert_eq!(Palette::new(Theme::Graphite, Some(0xff0000)), Palette::new(Theme::Graphite, None));
        assert_ne!(Palette::new(Theme::Accent, Some(0xff0000)), Palette::new(Theme::Accent, Some(0x0000ff)));
    }
}
