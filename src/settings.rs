use std::fs;
use std::io;
use std::ops::RangeInclusive;
use std::path::Path;

use crate::render::Look;

pub const BLUR_RANGE: RangeInclusive<u32> = 1..=10;
pub const SATURATION_RANGE: RangeInclusive<u32> = 0..=200;
pub const BRIGHTNESS_RANGE: RangeInclusive<u32> = 20..=120;

/// Cover pixels of blur radius per slider step.
const BLUR_STEP: u32 = 2;
const PERCENT: f32 = 100.0;
const MAX_COLOR: u32 = 0xFF_FFFF;

/// Corner colors as 0xRRGGBB: top-left, top-right, bottom-left, bottom-right.
pub type Corners = [u32; 4];

pub const PRESETS: [Corners; 6] = [
    [0x1e1b4b, 0xdb2777, 0x7c3aed, 0xf97316],
    [0x022c22, 0x0891b2, 0x0f766e, 0x38bdf8],
    [0x0f172a, 0xa21caf, 0x2563eb, 0x22d3ee],
    [0x052e16, 0x15803d, 0x365314, 0xa3e635],
    [0x7c2d12, 0xfb923c, 0xbe185d, 0xfde68a],
    [0x0a0a0a, 0x262626, 0x171717, 0x525252],
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Gradient follows the cover of the playing track.
    Music,
    /// Fixed gradient from user-picked corner colors.
    Custom,
}

impl Mode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Music => "music",
            Self::Custom => "custom",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "music" => Some(Self::Music),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub mode: Mode,
    pub colors: Corners,
    pub spotify: bool,
    pub browsers: bool,
    pub blur: u32,
    pub saturation: u32,
    pub brightness: u32,
    pub restore_on_exit: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: Mode::Music,
            colors: PRESETS[0],
            spotify: true,
            browsers: true,
            blur: 6,
            saturation: 130,
            brightness: 80,
            restore_on_exit: true,
        }
    }
}

impl Settings {
    /// Missing or malformed values fall back to defaults.
    pub fn load(path: &Path) -> Self {
        let mut settings = Self::default();
        let Ok(text) = fs::read_to_string(path) else {
            return settings;
        };

        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "mode" => settings.mode = Mode::parse(value).unwrap_or(settings.mode),
                "colors" => settings.colors = parse_colors(value).unwrap_or(settings.colors),
                "spotify" => settings.spotify = parse_bool(value, settings.spotify),
                "browsers" => settings.browsers = parse_bool(value, settings.browsers),
                "blur" => settings.blur = parse_in(value, BLUR_RANGE, settings.blur),
                "saturation" => settings.saturation = parse_in(value, SATURATION_RANGE, settings.saturation),
                "brightness" => settings.brightness = parse_in(value, BRIGHTNESS_RANGE, settings.brightness),
                "restore_on_exit" => settings.restore_on_exit = parse_bool(value, settings.restore_on_exit),
                _ => {}
            }
        }
        settings
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let colors = self.colors.map(|c| format!("{c:06x}")).join(",");
        let text = format!(
            "mode={}\ncolors={colors}\nspotify={}\nbrowsers={}\nblur={}\nsaturation={}\nbrightness={}\nrestore_on_exit={}\n",
            self.mode.as_str(),
            self.spotify,
            self.browsers,
            self.blur,
            self.saturation,
            self.brightness,
            self.restore_on_exit,
        );
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, path)
    }

    pub fn look(&self) -> Look {
        Look {
            blur_radius: (self.blur * BLUR_STEP) as usize,
            saturation: self.saturation as f32 / PERCENT,
            brightness: self.brightness as f32 / PERCENT,
        }
    }
}

fn parse_bool(value: &str, fallback: bool) -> bool {
    match value {
        "true" | "1" => true,
        "false" | "0" => false,
        _ => fallback,
    }
}

fn parse_in(value: &str, range: RangeInclusive<u32>, fallback: u32) -> u32 {
    value.parse().ok().filter(|v| range.contains(v)).unwrap_or(fallback)
}

fn parse_colors(value: &str) -> Option<Corners> {
    let mut parts = value.split(',');
    let mut colors = Corners::default();
    for color in &mut colors {
        *color = u32::from_str_radix(parts.next()?.trim(), 16).ok().filter(|c| *c <= MAX_COLOR)?;
    }
    parts.next().is_none().then_some(colors)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_round_trip() {
        let path = std::env::temp_dir().join(format!("scarp-wallpaper-settings-{}.ini", std::process::id()));
        let settings = Settings {
            mode: Mode::Custom,
            colors: [0x000000, 0xffffff, 0x123456, 0xabcdef],
            spotify: false,
            blur: 3,
            saturation: 0,
            brightness: 120,
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        let loaded = Settings::load(&path);
        fs::remove_file(&path).unwrap();
        assert_eq!(loaded, settings);
    }

    #[test]
    fn invalid_values_fall_back_to_defaults() {
        let path = std::env::temp_dir().join(format!("scarp-wallpaper-invalid-{}.ini", std::process::id()));
        fs::write(
            &path,
            "blur=99\nsaturation=abc\nspotify=maybe\nbrowsers=false\nmode=disco\ncolors=ff0000,00ff00\nnoise\n",
        )
        .unwrap();
        let loaded = Settings::load(&path);
        fs::remove_file(&path).unwrap();
        assert_eq!(loaded, Settings { browsers: false, ..Settings::default() });
    }

    #[test]
    fn colors_reject_out_of_range_and_extra_values() {
        assert_eq!(parse_colors("0,1,2,3"), Some([0, 1, 2, 3]));
        assert_eq!(parse_colors("1000000,1,2,3"), None);
        assert_eq!(parse_colors("0,1,2,3,4"), None);
    }
}
