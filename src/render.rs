//! Turns a tiny cover image into a smooth full-screen gradient.
//!
//! The cover is blurred at 64 px, where blurring costs nothing, and then
//! upscaled with a cubic B-spline. The B-spline is C2-continuous, so the
//! enlarged result has no visible grid, and dithering hides 8-bit banding.
//! Output rows are produced one at a time and never held as a full frame.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;

pub const COVER_SIZE: u32 = 64;
pub const COVER_BYTES: usize = (COVER_SIZE * COVER_SIZE * 4) as usize;

const BLUR_PASSES: usize = 3;
const WRITE_BUFFER_BYTES: usize = 1 << 16;
const BMP_FILE_HEADER_BYTES: u32 = 14;
const BMP_INFO_HEADER_BYTES: u32 = 40;
const BMP_PIXELS_PER_METER: i32 = 2835;
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// Box blur radius in cover pixels.
    pub blur_radius: usize,
    pub saturation: f32,
    pub brightness: f32,
}

/// Blurred and graded low-resolution image, stored as sRGB values in 0..=255.
pub struct Gradient {
    pixels: Vec<[f32; 3]>,
    width: usize,
    height: usize,
}

impl Gradient {
    /// `cover` is `COVER_SIZE`x`COVER_SIZE` BGRA. The cover is center-cropped
    /// to the `aspect_width`:`aspect_height` ratio of the target image.
    pub fn new(cover: &[u8], look: &Look, aspect_width: u32, aspect_height: u32) -> Self {
        assert_eq!(cover.len(), COVER_BYTES, "cover must be {COVER_SIZE}x{COVER_SIZE} BGRA");

        let (width, height) = grid_size(aspect_width, aspect_height);
        let side = COVER_SIZE as usize;
        let (left, top) = ((side - width) / 2, (side - height) / 2);
        let to_linear = linear_table();

        let mut pixels = Vec::with_capacity(width * height);
        for y in top..top + height {
            for x in left..left + width {
                let i = (y * side + x) * 4;
                pixels.push([
                    to_linear[cover[i + 2] as usize],
                    to_linear[cover[i + 1] as usize],
                    to_linear[cover[i] as usize],
                ]);
            }
        }

        blur(&mut pixels, width, height, look.blur_radius);
        for pixel in &mut pixels {
            *pixel = grade(*pixel, look).map(|c| linear_to_srgb(c) * 255.0);
        }

        Self { pixels, width, height }
    }

    /// Streams the gradient into a 24-bit BMP without buffering the frame.
    pub fn write_bmp(&self, width: u32, height: u32, path: &Path) -> io::Result<()> {
        if width == 0 || height == 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty image size"));
        }

        let row_bytes = width as usize * 3;
        let stride = (row_bytes + 3) & !3;
        let image_bytes = u32::try_from(stride * height as usize)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "image is too large"))?;
        let header_bytes = BMP_FILE_HEADER_BYTES + BMP_INFO_HEADER_BYTES;

        let mut header = Vec::with_capacity(header_bytes as usize);
        header.extend_from_slice(b"BM");
        header.extend_from_slice(&(header_bytes + image_bytes).to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(&header_bytes.to_le_bytes());
        header.extend_from_slice(&BMP_INFO_HEADER_BYTES.to_le_bytes());
        header.extend_from_slice(&(width as i32).to_le_bytes());
        header.extend_from_slice(&(height as i32).to_le_bytes());
        header.extend_from_slice(&1u16.to_le_bytes());
        header.extend_from_slice(&24u16.to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(&image_bytes.to_le_bytes());
        header.extend_from_slice(&BMP_PIXELS_PER_METER.to_le_bytes());
        header.extend_from_slice(&BMP_PIXELS_PER_METER.to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes());

        let mut out = BufWriter::with_capacity(WRITE_BUFFER_BYTES, File::create(path)?);
        out.write_all(&header)?;

        let mut upscaler = Upscaler::new(self, width, height);
        let mut row = vec![0u8; stride];
        // BMP stores rows bottom-up.
        for y in (0..height).rev() {
            upscaler.row(y, &mut row[..row_bytes], 3);
            out.write_all(&row)?;
        }
        out.flush()
    }

    /// Renders a top-down BGRA image, used for the settings preview.
    pub fn to_bgra(&self, width: u32, height: u32) -> Vec<u8> {
        let row_bytes = width as usize * 4;
        let mut pixels = vec![0u8; row_bytes * height as usize];
        if row_bytes == 0 {
            return pixels;
        }

        let mut upscaler = Upscaler::new(self, width, height);
        for (y, row) in pixels.chunks_exact_mut(row_bytes).enumerate() {
            upscaler.row(y as u32, row, 4);
        }
        pixels
    }
}

/// Builds a cover from four corner colors (0xRRGGBB: top-left, top-right,
/// bottom-left, bottom-right), so custom gradients reuse the cover pipeline.
pub fn corner_cover(corners: [u32; 4]) -> Vec<u8> {
    let side = COVER_SIZE as usize;
    let last = (side - 1) as f32;
    let mut cover = Vec::with_capacity(COVER_BYTES);
    for y in 0..side {
        let v = y as f32 / last;
        for x in 0..side {
            let u = x as f32 / last;
            let weights = [(1.0 - u) * (1.0 - v), u * (1.0 - v), (1.0 - u) * v, u * v];
            // BGRA byte order: blue is the lowest byte of 0xRRGGBB.
            for shift in [0, 8, 16] {
                let value: f32 =
                    corners.iter().zip(weights).map(|(color, w)| ((color >> shift) & 0xFF) as f32 * w).sum();
                cover.push(value.round() as u8);
            }
            cover.push(u8::MAX);
        }
    }
    cover
}

/// Grid that keeps the target aspect ratio inside the square cover.
fn grid_size(width: u32, height: u32) -> (usize, usize) {
    let side = COVER_SIZE as f32;
    let (width, height) = (width.max(1) as f32, height.max(1) as f32);
    let fit = |short: f32, long: f32| ((side * short / long).round() as usize).max(1);
    if width >= height { (COVER_SIZE as usize, fit(height, width)) } else { (fit(width, height), COVER_SIZE as usize) }
}

fn linear_table() -> [f32; 256] {
    std::array::from_fn(|i| srgb_to_linear(i as f32 / 255.0))
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

fn grade(rgb: [f32; 3], look: &Look) -> [f32; 3] {
    let luma = rgb[0] * LUMA[0] + rgb[1] * LUMA[1] + rgb[2] * LUMA[2];
    rgb.map(|c| ((luma + (c - luma) * look.saturation) * look.brightness).clamp(0.0, 1.0))
}

/// Three box blur passes per axis approximate a Gaussian.
fn blur(pixels: &mut [[f32; 3]], width: usize, height: usize, radius: usize) {
    if radius == 0 {
        return;
    }

    let mut line = Vec::with_capacity(height);
    let mut scratch = Vec::with_capacity(width.max(height));
    for _ in 0..BLUR_PASSES {
        for row in pixels.chunks_exact_mut(width) {
            box_blur(row, radius, &mut scratch);
        }
        for x in 0..width {
            line.clear();
            line.extend((0..height).map(|y| pixels[y * width + x]));
            box_blur(&mut line, radius, &mut scratch);
            for (y, pixel) in line.iter().enumerate() {
                pixels[y * width + x] = *pixel;
            }
        }
    }
}

fn box_blur(line: &mut [[f32; 3]], radius: usize, scratch: &mut Vec<[f32; 3]>) {
    scratch.clear();
    scratch.extend_from_slice(line);
    let last = line.len() as isize - 1;
    let r = radius as isize;
    let at = |i: isize| scratch[i.clamp(0, last) as usize];
    let scale = 1.0 / (2 * radius + 1) as f32;

    let mut sum = [0.0f32; 3];
    for i in -r..=r {
        let p = at(i);
        for c in 0..3 {
            sum[c] += p[c];
        }
    }
    for (x, pixel) in line.iter_mut().enumerate() {
        *pixel = sum.map(|s| s * scale);
        let x = x as isize;
        let (incoming, outgoing) = (at(x + r + 1), at(x - r));
        for c in 0..3 {
            sum[c] += incoming[c] - outgoing[c];
        }
    }
}

#[derive(Clone, Copy)]
struct Taps {
    index: [usize; 4],
    weight: [f32; 4],
}

impl Taps {
    fn new(out_index: u32, out_len: u32, src_len: usize) -> Self {
        let pos = (out_index as f32 + 0.5) * src_len as f32 / out_len as f32 - 0.5;
        let base = pos.floor();
        let t = pos - base;
        let last = src_len as isize - 1;
        let base = base as isize;
        Self { index: std::array::from_fn(|k| (base - 1 + k as isize).clamp(0, last) as usize), weight: bspline(t) }
    }
}

/// Uniform cubic B-spline basis; weights always sum to 1.
fn bspline(t: f32) -> [f32; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    let u = 1.0 - t;
    [u * u * u / 6.0, (3.0 * t3 - 6.0 * t2 + 4.0) / 6.0, (-3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0) / 6.0, t3 / 6.0]
}

/// Separable upscaler: one vertical mix per output row, then four
/// horizontal taps per output pixel.
struct Upscaler<'a> {
    gradient: &'a Gradient,
    columns: Vec<Taps>,
    out_height: u32,
    mixed: Vec<[f32; 3]>,
}

impl<'a> Upscaler<'a> {
    fn new(gradient: &'a Gradient, out_width: u32, out_height: u32) -> Self {
        Self {
            gradient,
            columns: (0..out_width).map(|x| Taps::new(x, out_width, gradient.width)).collect(),
            out_height,
            mixed: vec![[0.0; 3]; gradient.width],
        }
    }

    fn row(&mut self, y: u32, out: &mut [u8], bytes_per_pixel: usize) {
        let g = self.gradient;
        let rows = Taps::new(y, self.out_height, g.height);
        for (x, mixed) in self.mixed.iter_mut().enumerate() {
            let mut sum = [0.0f32; 3];
            for k in 0..4 {
                let p = g.pixels[rows.index[k] * g.width + x];
                for c in 0..3 {
                    sum[c] += p[c] * rows.weight[k];
                }
            }
            *mixed = sum;
        }

        for ((x, taps), pixel) in self.columns.iter().enumerate().zip(out.chunks_exact_mut(bytes_per_pixel)) {
            let mut sum = [0.0f32; 3];
            for k in 0..4 {
                let p = self.mixed[taps.index[k]];
                for c in 0..3 {
                    sum[c] += p[c] * taps.weight[k];
                }
            }
            let noise = dither(x as u32, y);
            let quantize = |v: f32| (v + noise).round().clamp(0.0, 255.0) as u8;
            pixel[0] = quantize(sum[2]);
            pixel[1] = quantize(sum[1]);
            pixel[2] = quantize(sum[0]);
            if bytes_per_pixel == 4 {
                pixel[3] = u8::MAX;
            }
        }
    }
}

/// Triangular noise in -1..1 from a coordinate hash: stable between renders
/// and free of the regular patterns that ordered dithering shows on gradients.
fn dither(x: u32, y: u32) -> f32 {
    let mut h = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    ((h & 0xFF) + ((h >> 8) & 0xFF)) as f32 / 255.0 - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const NEUTRAL: Look = Look { blur_radius: 4, saturation: 1.0, brightness: 1.0 };

    fn solid_cover(b: u8, g: u8, r: u8) -> Vec<u8> {
        [b, g, r, 255].repeat((COVER_SIZE * COVER_SIZE) as usize)
    }

    #[test]
    fn grid_keeps_aspect_ratio() {
        assert_eq!(grid_size(1920, 1080), (64, 36));
        assert_eq!(grid_size(1080, 1920), (36, 64));
        assert_eq!(grid_size(1000, 1000), (64, 64));
        assert_eq!(grid_size(10_000, 1), (64, 1));
    }

    #[test]
    fn bspline_weights_sum_to_one() {
        for i in 0..=10 {
            let sum: f32 = bspline(i as f32 / 10.0).iter().sum();
            assert!((sum - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn solid_cover_stays_solid() {
        let gradient = Gradient::new(&solid_cover(40, 120, 200), &NEUTRAL, 160, 90);
        let pixels = gradient.to_bgra(160, 90);
        for px in pixels.as_chunks::<4>().0 {
            assert!(px[0].abs_diff(40) <= 1 && px[1].abs_diff(120) <= 1 && px[2].abs_diff(200) <= 1);
            assert_eq!(px[3], 255);
        }
    }

    #[test]
    fn zero_saturation_is_gray() {
        let look = Look { saturation: 0.0, ..NEUTRAL };
        let gradient = Gradient::new(&solid_cover(10, 200, 90), &look, 64, 64);
        for px in gradient.to_bgra(32, 32).as_chunks::<4>().0 {
            assert!(px[0].abs_diff(px[1]) <= 2 && px[1].abs_diff(px[2]) <= 2);
        }
    }

    #[test]
    fn corner_cover_hits_corner_colors() {
        let cover = corner_cover([0xff0000, 0x00ff00, 0x0000ff, 0xffffff]);
        let side = COVER_SIZE as usize;
        let at = |x: usize, y: usize| &cover[(y * side + x) * 4..(y * side + x) * 4 + 4];
        assert_eq!(at(0, 0), [0, 0, 255, 255]);
        assert_eq!(at(side - 1, 0), [0, 255, 0, 255]);
        assert_eq!(at(0, side - 1), [255, 0, 0, 255]);
        assert_eq!(at(side - 1, side - 1), [255, 255, 255, 255]);
    }

    #[test]
    fn bmp_has_valid_layout() {
        let path = std::env::temp_dir().join(format!("scarp-wallpaper-test-{}.bmp", std::process::id()));
        let gradient = Gradient::new(&solid_cover(1, 2, 3), &NEUTRAL, 1, 1);
        gradient.write_bmp(5, 3, &path).unwrap();
        let data = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        // 5 px * 3 bytes = 15, padded to 16 per row.
        assert_eq!(data.len(), 54 + 16 * 3);
        assert_eq!(&data[..2], b"BM");
        assert_eq!(u32::from_le_bytes(data[2..6].try_into().unwrap()), data.len() as u32);
        assert_eq!(i32::from_le_bytes(data[18..22].try_into().unwrap()), 5);
        assert_eq!(i32::from_le_bytes(data[22..26].try_into().unwrap()), 3);
    }
}
