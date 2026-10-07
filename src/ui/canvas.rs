//! Minimal drawing layer: anti-aliased shapes rasterized directly into a
//! 32-bit DIB, text through GDI on the same bitmap.

use std::ffi::c_void;

use windows::Win32::Foundation::{COLORREF, RECT, SIZE};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLEARTYPE_QUALITY, CreateCompatibleDC, CreateDIBSection, CreateFontIndirectW,
    DEFAULT_CHARSET, DIB_RGB_COLORS, DRAW_TEXT_FORMAT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX, DT_RIGHT,
    DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW, FONT_QUALITY, GdiFlush, GetTextExtentPoint32W,
    HBITMAP, HDC, HFONT, HGDIOBJ, LOGFONTW, SelectObject, SetBkMode, SetTextCharacterExtra, SetTextColor, TRANSPARENT,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn hex(value: u32) -> Self {
        Self((value >> 16) as u8, (value >> 8) as u8, value as u8)
    }

    fn pixel(self) -> u32 {
        self.to_hex()
    }

    pub fn from_colorref(color: COLORREF) -> Self {
        Self(color.0 as u8, (color.0 >> 8) as u8, (color.0 >> 16) as u8)
    }

    pub const fn to_hex(self) -> u32 {
        (self.0 as u32) << 16 | (self.1 as u32) << 8 | self.2 as u32
    }

    pub fn colorref(self) -> COLORREF {
        COLORREF((self.2 as u32) << 16 | (self.1 as u32) << 8 | self.0 as u32)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn right(&self) -> i32 {
        self.x + self.w
    }

    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub const fn center_y(&self) -> i32 {
        self.y + self.h / 2
    }

    pub const fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    pub const fn outset(&self, d: i32) -> Self {
        Self::new(self.x - d, self.y - d, self.w + 2 * d, self.h + 2 * d)
    }

    pub fn scale(&self, dpi: u32) -> Self {
        Self::new(scale(self.x, dpi), scale(self.y, dpi), scale(self.w, dpi), scale(self.h, dpi))
    }

    fn to_win32(self) -> RECT {
        RECT { left: self.x, top: self.y, right: self.right(), bottom: self.bottom() }
    }
}

pub const BASE_DPI: u32 = 96;

pub fn scale(value: i32, dpi: u32) -> i32 {
    (value as i64 * dpi as i64 / BASE_DPI as i64) as i32
}

/// Off-screen 32-bit top-down DIB selected into a memory DC.
pub struct BackBuffer {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u32,
    pub width: i32,
    pub height: i32,
}

impl BackBuffer {
    pub fn new(width: i32, height: i32) -> Option<Self> {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.is_invalid() {
                return None;
            }
            let mut bits: *mut c_void = std::ptr::null_mut();
            let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
                let _ = DeleteDC(dc);
                return None;
            };
            let previous = SelectObject(dc, bitmap.into());
            SetBkMode(dc, TRANSPARENT);
            Some(Self { dc, bitmap, previous, bits: bits.cast(), width, height })
        }
    }

    pub fn dc(&self) -> HDC {
        self.dc
    }

    /// Pixel access; pending GDI text from the previous frame is flushed first.
    pub fn canvas(&mut self) -> Canvas<'_> {
        unsafe {
            let _ = GdiFlush();
            let len = (self.width * self.height) as usize;
            Canvas { pixels: std::slice::from_raw_parts_mut(self.bits, len), width: self.width, height: self.height }
        }
    }
}

impl Drop for BackBuffer {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

pub struct Canvas<'a> {
    pixels: &'a mut [u32],
    width: i32,
    height: i32,
}

impl Canvas<'_> {
    pub fn fill(&mut self, r: Rect, color: Rgb) {
        let Some((x0, y0, x1, y1)) = self.clip(r) else {
            return;
        };
        let pixel = color.pixel();
        for y in y0..y1 {
            let row = (y * self.width) as usize;
            self.pixels[row + x0 as usize..row + x1 as usize].fill(pixel);
        }
    }

    pub fn fill_round(&mut self, r: Rect, radius: f32, color: Rgb) {
        self.shade(r, radius, |d| (0.5 - d).clamp(0.0, 1.0), color);
    }

    /// One-pixel border along the inside of a rounded rectangle.
    pub fn stroke_round(&mut self, r: Rect, radius: f32, color: Rgb) {
        self.shade(r, radius, |d| (0.5 - d).clamp(0.0, 1.0) - (-0.5 - d).clamp(0.0, 1.0), color);
    }

    /// Paints `background` outside the rounded shape, rounding off the
    /// corners of whatever is already inside `r`.
    pub fn round_corners(&mut self, r: Rect, radius: f32, background: Rgb) {
        self.shade(r, radius, |d| (d + 0.5).clamp(0.0, 1.0), background);
    }

    /// Copies a top-down BGRA image of exactly `r.w`x`r.h` pixels.
    pub fn blit(&mut self, r: Rect, bgra: &[u8]) {
        if bgra.len() != (r.w * r.h * 4) as usize {
            return;
        }
        let Some((x0, y0, x1, y1)) = self.clip(r) else {
            return;
        };
        for y in y0..y1 {
            let src_row = ((y - r.y) * r.w) as usize * 4;
            let dst_row = (y * self.width) as usize;
            for x in x0..x1 {
                let s = src_row + (x - r.x) as usize * 4;
                self.pixels[dst_row + x as usize] =
                    (bgra[s + 2] as u32) << 16 | (bgra[s + 1] as u32) << 8 | bgra[s] as u32;
            }
        }
    }

    /// Rasterizes a rounded box by its signed distance field; `coverage`
    /// maps the distance to the pixel center into opacity.
    fn shade(&mut self, r: Rect, radius: f32, coverage: impl Fn(f32) -> f32, color: Rgb) {
        let Some((x0, y0, x1, y1)) = self.clip(r) else {
            return;
        };
        let (half_w, half_h) = (r.w as f32 / 2.0, r.h as f32 / 2.0);
        let (cx, cy) = (r.x as f32 + half_w, r.y as f32 + half_h);
        let radius = radius.min(half_w).min(half_h);
        let pixel = color.pixel();

        for y in y0..y1 {
            let qy = (y as f32 + 0.5 - cy).abs() - (half_h - radius);
            let row = (y * self.width) as usize;
            for x in x0..x1 {
                let qx = (x as f32 + 0.5 - cx).abs() - (half_w - radius);
                let distance = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - radius;
                let alpha = coverage(distance);
                if alpha > 0.0 {
                    let dst = &mut self.pixels[row + x as usize];
                    *dst = blend(*dst, pixel, alpha);
                }
            }
        }
    }

    fn clip(&self, r: Rect) -> Option<(i32, i32, i32, i32)> {
        let (x0, y0) = (r.x.max(0), r.y.max(0));
        let (x1, y1) = (r.right().min(self.width), r.bottom().min(self.height));
        (x0 < x1 && y0 < y1).then_some((x0, y0, x1, y1))
    }
}

fn blend(dst: u32, src: u32, alpha: f32) -> u32 {
    if alpha >= 1.0 {
        return src;
    }
    let mix = |shift: u32| {
        let (d, s) = ((dst >> shift) & 0xFF, (src >> shift) & 0xFF);
        ((d as f32 + (s as f32 - d as f32) * alpha).round() as u32) << shift
    };
    mix(16) | mix(8) | mix(0)
}

pub struct Font(HFONT);

impl Font {
    pub fn new(face: &str, px: i32, weight: i32, dpi: u32) -> Self {
        let mut font = LOGFONTW {
            lfHeight: -scale(px, dpi),
            lfWeight: weight,
            lfCharSet: DEFAULT_CHARSET,
            lfQuality: FONT_QUALITY(CLEARTYPE_QUALITY.0),
            ..Default::default()
        };
        for (dst, src) in font.lfFaceName.iter_mut().zip(face.encode_utf16().take(31)) {
            *dst = src;
        }
        Self(unsafe { CreateFontIndirectW(&font) })
    }
}

impl Drop for Font {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.0.into());
        }
    }
}

#[derive(Clone, Copy)]
pub enum Align {
    Left,
    Center,
    Right,
}

pub struct TextStyle<'a> {
    pub font: &'a Font,
    pub color: Rgb,
    pub align: Align,
    /// Extra spacing between characters in physical pixels.
    pub tracking: i32,
}

pub fn draw_text(dc: HDC, r: Rect, text: &str, style: &TextStyle) {
    let align = match style.align {
        Align::Left => DT_LEFT,
        Align::Center => DT_CENTER,
        Align::Right => DT_RIGHT,
    };
    let format: DRAW_TEXT_FORMAT = DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS | align;
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    let mut rect = r.to_win32();
    unsafe {
        SelectObject(dc, style.font.0.into());
        SetTextColor(dc, style.color.colorref());
        SetTextCharacterExtra(dc, style.tracking);
        DrawTextW(dc, &mut wide, &mut rect, format);
        SetTextCharacterExtra(dc, 0);
    }
}

pub fn text_width(dc: HDC, text: &str, font: &Font) -> i32 {
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut size = SIZE::default();
    unsafe {
        SelectObject(dc, font.0.into());
        let _ = GetTextExtentPoint32W(dc, &wide, &mut size);
    }
    size.cx
}
