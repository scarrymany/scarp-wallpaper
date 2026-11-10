//! Accent color of a cover: the most prominent saturated hue, for themes that
//! tint the interface with the music.

/// Hue buckets of 15 degrees.
const HUE_BINS: usize = 24;
/// Pixels below these are too gray or too dark to define a color.
const MIN_SATURATION: f32 = 0.25;
const MIN_VALUE: f32 = 0.2;
/// Share of the colorful weight the winning hue needs to beat the noise.
const MIN_SHARE: f32 = 0.02;

/// The dominant vivid color of a BGRA image as 0xRRGGBB, or `None` for a
/// grayscale or near-black image.
pub fn dominant(bgra: &[u8]) -> Option<u32> {
    let mut weight = [0.0f32; HUE_BINS];
    let mut sum = [[0.0f32; 3]; HUE_BINS];

    for pixel in bgra.as_chunks::<4>().0 {
        let rgb = [pixel[2], pixel[1], pixel[0]].map(|c| c as f32 / 255.0);
        let (hue, saturation, value) = hsv(rgb);
        if saturation < MIN_SATURATION || value < MIN_VALUE {
            continue;
        }
        // Vivid, bright pixels count most.
        let w = saturation * saturation * value;
        let bin = ((hue / 360.0 * HUE_BINS as f32) as usize).min(HUE_BINS - 1);
        weight[bin] += w;
        for (acc, c) in sum[bin].iter_mut().zip(rgb) {
            *acc += c * w;
        }
    }

    // Neighbouring bins belong to the same hue; pick the best window of three.
    let window = |i: usize| (0..3).map(move |d| (i + HUE_BINS - 1 + d) % HUE_BINS);
    let best = (0..HUE_BINS).max_by(|a, b| {
        let score = |i| window(i).map(|j| weight[j]).sum::<f32>();
        score(*a).total_cmp(&score(*b))
    })?;
    let best_weight: f32 = window(best).map(|j| weight[j]).sum();
    let pixel_count = (bgra.len() / 4).max(1) as f32;
    if best_weight <= 0.0 || best_weight / pixel_count < MIN_SHARE {
        return None;
    }
    let rgb = [0, 1, 2].map(|c| window(best).map(|j| sum[j][c]).sum::<f32>() / best_weight);
    Some(to_hex(rgb))
}

/// Re-tones a color to a fixed lightness, keeping its hue, with saturation
/// clamped to `saturation`; keeps accents readable on a given background.
pub fn tone(color: u32, lightness: f32, saturation: (f32, f32)) -> u32 {
    let (hue, s, _) = hsl(from_hex(color));
    to_hex(from_hsl(hue, s.clamp(saturation.0, saturation.1), lightness))
}

fn from_hex(color: u32) -> [f32; 3] {
    [16, 8, 0].map(|shift| ((color >> shift) & 0xFF) as f32 / 255.0)
}

fn to_hex(rgb: [f32; 3]) -> u32 {
    let [r, g, b] = rgb.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u32);
    r << 16 | g << 8 | b
}

fn hsv([r, g, b]: [f32; 3]) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let saturation = if max > 0.0 { delta / max } else { 0.0 };
    (hue(r, g, b, max, delta), saturation, max)
}

fn hsl([r, g, b]: [f32; 3]) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let lightness = (max + min) / 2.0;
    let saturation = if delta == 0.0 { 0.0 } else { delta / (1.0 - (2.0 * lightness - 1.0).abs()) };
    (hue(r, g, b, max, delta), saturation.min(1.0), lightness)
}

fn hue(r: f32, g: f32, b: f32, max: f32, delta: f32) -> f32 {
    if delta == 0.0 {
        return 0.0;
    }
    let h = if max == r {
        ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        (b - r) / delta + 2.0
    } else {
        (r - g) / delta + 4.0
    };
    h * 60.0
}

fn from_hsl(hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let x = chroma * (1.0 - ((hue / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = lightness - chroma / 2.0;
    let (r, g, b) = match (hue.rem_euclid(360.0) / 60.0) as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    [r + m, g + m, b + m]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(colors: &[(u32, usize)]) -> Vec<u8> {
        let mut bgra = Vec::new();
        for (color, count) in colors {
            for _ in 0..*count {
                bgra.extend([*color as u8, (*color >> 8) as u8, (*color >> 16) as u8, 255]);
            }
        }
        bgra
    }

    fn hue_of(color: u32) -> f32 {
        hsl(from_hex(color)).0
    }

    #[test]
    fn picks_the_vivid_hue_over_gray() {
        let color = dominant(&image(&[(0x808080, 3000), (0x101010, 500), (0xe0306a, 600)])).unwrap();
        assert!((hue_of(color) - hue_of(0xe0306a)).abs() < 10.0, "{color:06x}");
    }

    #[test]
    fn grayscale_has_no_accent() {
        assert_eq!(dominant(&image(&[(0x808080, 4000), (0xffffff, 96)])), None);
    }

    #[test]
    fn larger_area_wins() {
        let color = dominant(&image(&[(0x2050e0, 3000), (0xe02020, 1000)])).unwrap();
        assert!((hue_of(color) - hue_of(0x2050e0)).abs() < 10.0, "{color:06x}");
    }

    #[test]
    fn tone_keeps_hue_and_sets_lightness() {
        let toned = tone(0x200810, 0.64, (0.55, 0.9));
        let (h, s, l) = hsl(from_hex(toned));
        assert!((h - hue_of(0x200810)).abs() < 3.0);
        assert!((l - 0.64).abs() < 0.01);
        assert!(s >= 0.54);
    }

    #[test]
    fn hex_round_trips_through_hsl() {
        for color in [0x000000, 0xffffff, 0xff0000, 0x12ab34, 0x7c3aed] {
            let (h, s, l) = hsl(from_hex(color));
            assert_eq!(to_hex(from_hsl(h, s, l)), color);
        }
    }
}
