//! Settings window, drawn entirely by hand in the scarp.cc style.
//!
//! There are no child controls: one back buffer, a hit-test over a static
//! layout and a handful of mouse handlers. The window and everything it owns
//! is freed when it is closed; the app itself keeps running in the tray.

use std::cell::{Cell, RefCell};
use std::ops::RangeInclusive;
use std::sync::Arc;

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, EndPaint, InvalidateRect, PAINTSTRUCT, SRCCOPY, ScreenToClient,
};
use windows::Win32::UI::Controls::Dialogs::{CC_ANYCOLOR, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW, ChooseColorW};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HICON, HTCAPTION, HTCLIENT, IDC_ARROW, IDC_HAND, LoadCursorW,
    RegisterClassExW, SPI_GETWORKAREA, SW_MINIMIZE, SW_RESTORE, SW_SHOW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetCursor, SetForegroundWindow, SetWindowPos,
    ShowWindow, SystemParametersInfoW, WINDOW_EX_STYLE, WM_CAPTURECHANGED, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND,
    WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCALCSIZE, WM_NCHITTEST, WM_PAINT, WM_SETCURSOR,
    WNDCLASSEXW, WS_CAPTION, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
};
use windows::core::{BOOL, HSTRING, PCWSTR, Result, w};

use super::canvas::{Align, BackBuffer, Canvas, Font, Rect, Rgb, TextStyle, draw_text, scale, text_width};
use super::{apply_settings, with_app};
use crate::render::{Gradient, Look, corner_cover};
use crate::settings::{BLUR_RANGE, BRIGHTNESS_RANGE, Corners, Mode, PRESETS, SATURATION_RANGE, Settings};
use crate::{APP_NAME, APP_VERSION, Shared, StatusKind, autostart, screen_size, trim_memory};

const CLASS: PCWSTR = w!("ScarpWallpaperSettings");

/// scarp.cc design tokens.
mod theme {
    use super::Rgb;

    pub const BG: Rgb = Rgb::hex(0x0a0a0a);
    pub const SURFACE: Rgb = Rgb::hex(0x141414);
    pub const SURFACE_2: Rgb = Rgb::hex(0x181818);
    pub const LINE: Rgb = Rgb::hex(0x1e1e1e);
    pub const LINE_2: Rgb = Rgb::hex(0x262626);
    pub const LINE_3: Rgb = Rgb::hex(0x3a3a3a);
    pub const TEXT: Rgb = Rgb::hex(0xd4d4d4);
    pub const TEXT_STRONG: Rgb = Rgb::hex(0xf5f5f5);
    pub const MUTED: Rgb = Rgb::hex(0xa3a3a3);
    pub const DIM: Rgb = Rgb::hex(0x737373);
    pub const OK: Rgb = Rgb::hex(0x4ade80);
    pub const ERR: Rgb = Rgb::hex(0xf87171);
    pub const WHITE: Rgb = Rgb::hex(0xffffff);
    pub const WHITE_HOVER: Rgb = Rgb::hex(0xe5e5e5);
    pub const BLACK: Rgb = Rgb::hex(0x000000);

    pub const RADIUS: f32 = 6.0;
    pub const RADIUS_SM: f32 = 4.0;

    pub const FONT: &str = "Segoe UI";
    pub const FONT_MONO: &str = "Consolas";
    pub const FONT_ICONS: &str = "Segoe MDL2 Assets";
}

// Layout metrics in 96 DPI pixels.
const PAD: i32 = 20;
const GAP: i32 = 20;
const COLUMN: i32 = 330;
const HEADER_H: i32 = 52;
const HEADER_BUTTON: i32 = 32;
const LOGO: i32 = 22;
const ROW_H: i32 = 52;
const ROW_SLIDER_H: i32 = 44;
const ROW_PAD: i32 = 14;
const LABEL_H: i32 = 16;
const LABEL_GAP: i32 = 8;
const SECTION_GAP: i32 = 20;
const STATUS_H: i32 = 20;
const STATUS_DOT: i32 = 8;
const PREVIEW_MIN_H: i32 = 140;
const PREVIEW_MAX_H: i32 = 260;
const MODE_H: i32 = 36;
const SEGMENT_INSET: i32 = 3;
const SWITCH_W: i32 = 36;
const SWITCH_H: i32 = 20;
const SWITCH_INSET: i32 = 3;
const SWATCH: i32 = 26;
const SWATCH_GAP: i32 = 8;
const CHIP_GAP: i32 = 6;
const SLIDER_LABEL_W: i32 = 110;
const SLIDER_VALUE_W: i32 = 48;
const SLIDER_TRACK_H: i32 = 4;
const SLIDER_KNOB: i32 = 14;
const SLIDER_KNOB_ACTIVE: i32 = 16;
const SLIDER_HIT_SLOP: i32 = 8;
const BUTTON_H: i32 = 32;
const DONE_W: i32 = 96;
const DEFAULTS_W: i32 = 132;

const ICON_MINIMIZE: &str = "\u{E921}";
const ICON_CLOSE: &str = "\u{E8BB}";
const MSG_NO_COVER: &str = "Включите трек в Spotify или SoundCloud";
const MSG_HINT: &str = "Окно можно закрыть - приложение останется в трее";
/// Chips show the raw corner colors, without the user's blur and grading.
const CHIP_LOOK: Look = Look { blur_radius: 0, saturation: 1.0, brightness: 1.0 };

const MODES: [Mode; 2] = [Mode::Music, Mode::Custom];

fn mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::Music => "Музыка",
        Mode::Custom => "Свой градиент",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Toggle {
    Spotify,
    Browsers,
    Autostart,
    Restore,
}

impl Toggle {
    fn texts(self) -> (&'static str, &'static str) {
        match self {
            Self::Spotify => ("Spotify", "Десктопное приложение"),
            Self::Browsers => ("Браузер", "SoundCloud, YouTube Music и другие"),
            Self::Autostart => ("Автозапуск", "Запускать вместе с Windows"),
            Self::Restore => ("Вернуть обои", "Исходные при выходе, режим музыки"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slider {
    Blur,
    Saturation,
    Brightness,
}

impl Slider {
    fn label(self) -> &'static str {
        match self {
            Self::Blur => "Размытие",
            Self::Saturation => "Насыщенность",
            Self::Brightness => "Яркость",
        }
    }

    fn range(self) -> RangeInclusive<u32> {
        match self {
            Self::Blur => BLUR_RANGE,
            Self::Saturation => SATURATION_RANGE,
            Self::Brightness => BRIGHTNESS_RANGE,
        }
    }

    fn unit(self) -> &'static str {
        match self {
            Self::Blur => "",
            Self::Saturation | Self::Brightness => "%",
        }
    }

    fn get(self, settings: &Settings) -> u32 {
        match self {
            Self::Blur => settings.blur,
            Self::Saturation => settings.saturation,
            Self::Brightness => settings.brightness,
        }
    }

    fn set(self, settings: &mut Settings, value: u32) {
        match self {
            Self::Blur => settings.blur = value,
            Self::Saturation => settings.saturation = value,
            Self::Brightness => settings.brightness = value,
        }
    }

    fn fraction(self, settings: &Settings) -> f32 {
        let range = self.range();
        (self.get(settings) - range.start()) as f32 / (range.end() - range.start()) as f32
    }
}

const SOURCE_TOGGLES: [Toggle; 2] = [Toggle::Spotify, Toggle::Browsers];
const SYSTEM_TOGGLES: [Toggle; 2] = [Toggle::Autostart, Toggle::Restore];
const SLIDERS: [Slider; 3] = [Slider::Blur, Slider::Saturation, Slider::Brightness];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    Minimize,
    Close,
    Mode(Mode),
    Toggle(Toggle),
    Swatch(usize),
    Preset(usize),
    Slider(Slider),
    Defaults,
    Done,
}

/// Physical-pixel layout for one DPI.
struct Layout {
    dpi: u32,
    size: Rect,
    header: Rect,
    logo: Rect,
    title: Rect,
    minimize: Rect,
    close: Rect,
    preview: Rect,
    status: Rect,
    /// Left card: music sources or custom colors, depending on the mode.
    source_label: Rect,
    source_card: Rect,
    mode_label: Rect,
    mode: Rect,
    look_label: Rect,
    look_card: Rect,
    system_label: Rect,
    system_card: Rect,
    footer_line: Rect,
    hint: Rect,
    defaults: Rect,
    done: Rect,
}

impl Layout {
    fn new(dpi: u32, (screen_w, screen_h): (u32, u32)) -> Self {
        let width = PAD + COLUMN + GAP + COLUMN + PAD;
        let (left, right, top) = (PAD, PAD + COLUMN + GAP, HEADER_H + PAD);

        let preview_h = (COLUMN as u64 * screen_h as u64 / screen_w as u64) as i32;
        let preview = Rect::new(left, top, COLUMN, preview_h.clamp(PREVIEW_MIN_H, PREVIEW_MAX_H));
        let status = Rect::new(left, preview.bottom() + 12, COLUMN, STATUS_H);
        let source_label = Rect::new(left, status.bottom() + SECTION_GAP, COLUMN, LABEL_H);
        let source_card = Rect::new(left, source_label.bottom() + LABEL_GAP, COLUMN, 2 * ROW_H);

        let mode_label = Rect::new(right, top, COLUMN, LABEL_H);
        let mode = Rect::new(right, mode_label.bottom() + LABEL_GAP, COLUMN, MODE_H);
        let look_label = Rect::new(right, mode.bottom() + SECTION_GAP, COLUMN, LABEL_H);
        let look_card = Rect::new(right, look_label.bottom() + LABEL_GAP, COLUMN, 3 * ROW_SLIDER_H);
        let system_label = Rect::new(right, look_card.bottom() + SECTION_GAP, COLUMN, LABEL_H);
        let system_card = Rect::new(right, system_label.bottom() + LABEL_GAP, COLUMN, 2 * ROW_H);

        let footer_y = source_card.bottom().max(system_card.bottom()) + SECTION_GAP;
        let buttons_y = footer_y + 16;
        let done = Rect::new(width - PAD - DONE_W, buttons_y, DONE_W, BUTTON_H);
        let defaults = Rect::new(done.x - 8 - DEFAULTS_W, buttons_y, DEFAULTS_W, BUTTON_H);
        let height = buttons_y + BUTTON_H + 16;
        let header_button_y = (HEADER_H - HEADER_BUTTON) / 2;
        let close = Rect::new(width - 10 - HEADER_BUTTON, header_button_y, HEADER_BUTTON, HEADER_BUTTON);

        Self {
            dpi,
            size: Rect::new(0, 0, width, height).scale(dpi),
            header: Rect::new(0, 0, width, HEADER_H).scale(dpi),
            logo: Rect::new(PAD, (HEADER_H - LOGO) / 2, LOGO, LOGO).scale(dpi),
            title: Rect::new(PAD + LOGO + 10, 0, COLUMN, HEADER_H).scale(dpi),
            minimize: Rect::new(close.x - 4 - HEADER_BUTTON, header_button_y, HEADER_BUTTON, HEADER_BUTTON).scale(dpi),
            close: close.scale(dpi),
            preview: preview.scale(dpi),
            status: status.scale(dpi),
            source_label: source_label.scale(dpi),
            source_card: source_card.scale(dpi),
            mode_label: mode_label.scale(dpi),
            mode: mode.scale(dpi),
            look_label: look_label.scale(dpi),
            look_card: look_card.scale(dpi),
            system_label: system_label.scale(dpi),
            system_card: system_card.scale(dpi),
            footer_line: Rect::new(0, footer_y, width, 1).scale(dpi),
            hint: Rect::new(PAD, buttons_y, defaults.x - PAD - 12, BUTTON_H).scale(dpi),
            defaults: defaults.scale(dpi),
            done: done.scale(dpi),
        }
    }

    fn s(&self, value: i32) -> i32 {
        scale(value, self.dpi)
    }

    fn radius(&self, value: f32) -> f32 {
        value * self.dpi as f32 / super::canvas::BASE_DPI as f32
    }

    fn row(&self, card: Rect, index: usize, row_h: i32) -> Rect {
        let h = self.s(row_h);
        Rect::new(card.x, card.y + h * index as i32, card.w, h)
    }

    fn segment(&self, mode: Mode) -> Rect {
        let inset = self.s(SEGMENT_INSET);
        let half = (self.mode.w - 2 * inset) / 2;
        let index = MODES.iter().position(|m| *m == mode).unwrap_or_default() as i32;
        Rect::new(self.mode.x + inset + index * half, self.mode.y + inset, half, self.mode.h - 2 * inset)
    }

    fn toggle_row(&self, toggle: Toggle) -> Rect {
        match SOURCE_TOGGLES.iter().position(|t| *t == toggle) {
            Some(i) => self.row(self.source_card, i, ROW_H),
            None => {
                let i = SYSTEM_TOGGLES.iter().position(|t| *t == toggle).unwrap_or_default();
                self.row(self.system_card, i, ROW_H)
            }
        }
    }

    fn switch(&self, toggle: Toggle) -> Rect {
        let row = self.toggle_row(toggle);
        let (w, h) = (self.s(SWITCH_W), self.s(SWITCH_H));
        Rect::new(row.right() - self.s(ROW_PAD) - w, row.center_y() - h / 2, w, h)
    }

    /// Square `index` of `count`, right-aligned in `row`.
    fn square(&self, row: Rect, index: usize, count: usize, gap: i32) -> Rect {
        let (size, gap) = (self.s(SWATCH), self.s(gap));
        let count = count as i32;
        let start = row.right() - self.s(ROW_PAD) - count * size - (count - 1) * gap;
        Rect::new(start + index as i32 * (size + gap), row.center_y() - size / 2, size, size)
    }

    fn swatch(&self, index: usize) -> Rect {
        self.square(self.row(self.source_card, 0, ROW_H), index, Corners::default().len(), SWATCH_GAP)
    }

    fn preset(&self, index: usize) -> Rect {
        self.square(self.row(self.source_card, 1, ROW_H), index, PRESETS.len(), CHIP_GAP)
    }

    fn slider_row(&self, slider: Slider) -> Rect {
        let i = SLIDERS.iter().position(|s| *s == slider).unwrap_or_default();
        self.row(self.look_card, i, ROW_SLIDER_H)
    }

    fn slider_track(&self, slider: Slider) -> Rect {
        let row = self.slider_row(slider);
        let x = row.x + self.s(ROW_PAD + SLIDER_LABEL_W);
        let right = row.right() - self.s(ROW_PAD + SLIDER_VALUE_W + 12);
        let h = self.s(SLIDER_TRACK_H).max(2);
        Rect::new(x, row.center_y() - h / 2, right - x, h)
    }

    fn hit(&self, x: i32, y: i32, mode: Mode) -> Option<Control> {
        let fixed = [
            (self.minimize, Control::Minimize),
            (self.close, Control::Close),
            (self.defaults, Control::Defaults),
            (self.done, Control::Done),
        ];
        if let Some((_, control)) = fixed.iter().find(|(r, _)| r.contains(x, y)) {
            return Some(*control);
        }
        if let Some(mode) = MODES.into_iter().find(|m| self.segment(*m).contains(x, y)) {
            return Some(Control::Mode(mode));
        }

        match mode {
            Mode::Music => {
                if let Some(toggle) = SOURCE_TOGGLES.into_iter().find(|t| self.toggle_row(*t).contains(x, y)) {
                    return Some(Control::Toggle(toggle));
                }
            }
            Mode::Custom => {
                if let Some(i) = (0..Corners::default().len()).find(|i| self.swatch(*i).contains(x, y)) {
                    return Some(Control::Swatch(i));
                }
                if let Some(i) = (0..PRESETS.len()).find(|i| self.preset(*i).contains(x, y)) {
                    return Some(Control::Preset(i));
                }
            }
        }
        if let Some(toggle) = SYSTEM_TOGGLES.into_iter().find(|t| self.toggle_row(*t).contains(x, y)) {
            return Some(Control::Toggle(toggle));
        }

        let slop = self.s(SLIDER_HIT_SLOP);
        SLIDERS
            .into_iter()
            .find(|s| {
                let (row, track) = (self.slider_row(*s), self.slider_track(*s));
                Rect::new(track.x - slop, row.y, track.w + 2 * slop, row.h).contains(x, y)
            })
            .map(Control::Slider)
    }
}

struct Fonts {
    body: Font,
    strong: Font,
    small: Font,
    eyebrow: Font,
    mono: Font,
    icons: Font,
    logo: Font,
}

impl Fonts {
    fn new(dpi: u32) -> Self {
        Self {
            body: Font::new(theme::FONT, 13, 400, dpi),
            strong: Font::new(theme::FONT, 13, 600, dpi),
            small: Font::new(theme::FONT, 12, 400, dpi),
            eyebrow: Font::new(theme::FONT, 11, 600, dpi),
            mono: Font::new(theme::FONT_MONO, 12, 400, dpi),
            icons: Font::new(theme::FONT_ICONS, 10, 400, dpi),
            logo: Font::new(theme::FONT, 14, 700, dpi),
        }
    }
}

/// What the preview was rendered from, to skip redundant re-renders.
#[derive(Clone)]
enum PreviewSource {
    Cover(Arc<[u8]>),
    Colors(Corners),
}

impl PartialEq for PreviewSource {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Cover(a), Self::Cover(b)) => Arc::ptr_eq(a, b),
            (Self::Colors(a), Self::Colors(b)) => a == b,
            _ => false,
        }
    }
}

struct Preview {
    source: PreviewSource,
    look: Look,
    width: i32,
    height: i32,
    pixels: Vec<u8>,
}

enum Effect {
    None,
    Close,
    Minimize,
    Resize(Rect),
    PickColor(usize),
}

impl Effect {
    /// Runs window calls that send messages synchronously or run a modal
    /// loop; they must happen after the state borrow is released.
    fn run(self, hwnd: HWND) {
        unsafe {
            match self {
                Self::None => {}
                Self::Close => {
                    let _ = DestroyWindow(hwnd);
                }
                Self::Minimize => {
                    let _ = ShowWindow(hwnd, SW_MINIMIZE);
                }
                Self::Resize(r) => {
                    let _ = SetWindowPos(hwnd, None, r.x, r.y, r.w, r.h, SWP_NOZORDER | SWP_NOACTIVATE);
                }
                Self::PickColor(index) => pick_color(hwnd, index),
            }
        }
    }
}

struct State {
    hwnd: HWND,
    shared: Arc<Shared>,
    layout: Layout,
    fonts: Fonts,
    draft: Settings,
    autostart: bool,
    hover: Option<Control>,
    pressed: Option<Control>,
    dragging: Option<Slider>,
    tracking_mouse: bool,
    preview: Option<Preview>,
    /// Preset thumbnails at the current DPI, BGRA.
    chips: Vec<Vec<u8>>,
    buffer: Option<BackBuffer>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    /// The "custom colors" row of the color dialog, kept for the session.
    static CUSTOM_COLORS: Cell<[COLORREF; 16]> = const { Cell::new([COLORREF(0); 16]) };
}

pub fn register_class(instance: HINSTANCE, large_icon: HICON, small_icon: HICON) -> Result<()> {
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        hIcon: large_icon,
        hIconSm: small_icon,
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW)? },
        lpszClassName: CLASS,
        ..Default::default()
    };
    if unsafe { RegisterClassExW(&class) } == 0 {
        return Err(windows::core::Error::from_thread());
    }
    Ok(())
}

pub fn open() {
    if let Some(hwnd) = current_window() {
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
        return;
    }
    let Some((instance, shared)) = with_app(|app| (app.instance, app.shared.clone())) else {
        return;
    };

    let dpi = unsafe { GetDpiForSystem() };
    let layout = Layout::new(dpi, screen_size());
    let (x, y) = centered(layout.size);
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            &HSTRING::from(APP_NAME),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            x,
            y,
            layout.size.w,
            layout.size.h,
            None,
            None,
            Some(instance),
            None,
        )
    };
    let Ok(hwnd) = hwnd else {
        return;
    };
    style_frame(hwnd);

    let mut state = State {
        hwnd,
        draft: shared.settings(),
        shared,
        fonts: Fonts::new(dpi),
        chips: preset_chips(&layout),
        layout,
        autostart: autostart::is_enabled(),
        hover: None,
        pressed: None,
        dragging: None,
        tracking_mouse: false,
        preview: None,
        buffer: None,
    };
    state.update_preview();
    STATE.with(|cell| *cell.borrow_mut() = Some(state));

    unsafe {
        // Re-runs WM_NCCALCSIZE so the frame is removed before first paint.
        let _ = SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
    }
}

pub fn close() {
    if let Some(hwnd) = current_window() {
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }
}

/// Picks up a new cover or status from the worker.
pub fn refresh() {
    with_state(|state| {
        state.update_preview();
        state.invalidate();
    });
}

fn with_state(f: impl FnOnce(&mut State)) {
    STATE.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut()
            && let Some(state) = guard.as_mut()
        {
            f(state);
        }
    });
}

fn current_window() -> Option<HWND> {
    STATE.with(|cell| cell.try_borrow().ok()?.as_ref().map(|state| state.hwnd))
}

fn preset_chips(layout: &Layout) -> Vec<Vec<u8>> {
    let size = layout.s(SWATCH) as u32;
    PRESETS
        .iter()
        .map(|preset| Gradient::new(&corner_cover(*preset), &CHIP_LOOK, size, size).to_bgra(size, size))
        .collect()
}

/// Opens the system color dialog for one corner. It runs its own modal
/// loop, so it is called without holding the window state.
fn pick_color(hwnd: HWND, index: usize) {
    let Some(current) = STATE.with(|cell| cell.try_borrow().ok()?.as_ref().map(|s| s.draft.colors[index])) else {
        return;
    };
    let mut custom = CUSTOM_COLORS.get();
    let mut dialog = CHOOSECOLORW {
        lStructSize: size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: hwnd,
        rgbResult: Rgb::hex(current).colorref(),
        lpCustColors: custom.as_mut_ptr(),
        Flags: CC_FULLOPEN | CC_RGBINIT | CC_ANYCOLOR,
        ..Default::default()
    };
    if !unsafe { ChooseColorW(&mut dialog) }.as_bool() {
        return;
    }
    CUSTOM_COLORS.set(custom);
    with_state(|state| {
        state.draft.colors[index] = Rgb::from_colorref(dialog.rgbResult).to_hex();
        state.update_preview();
        state.commit();
        state.invalidate();
    });
}

fn centered(size: Rect) -> (i32, i32) {
    let mut area = RECT::default();
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some((&raw mut area).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    let x = area.left + (area.right - area.left - size.w) / 2;
    let y = area.top + (area.bottom - area.top - size.h) / 2;
    (x.max(area.left), y.max(area.top))
}

/// Dark frame, rounded corners and a scarp.cc border on Windows 11.
/// Older systems ignore the attributes.
fn style_frame(hwnd: HWND) {
    unsafe {
        let dark = BOOL::from(true);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&raw const dark).cast(),
            size_of::<BOOL>() as u32,
        );
        let corners = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const corners).cast(),
            size_of_val(&corners) as u32,
        );
        let border = theme::LINE_2.colorref();
        let _ =
            DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, (&raw const border).cast(), size_of_val(&border) as u32);
    }
}

fn point_from(lparam: LPARAM) -> (i32, i32) {
    let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
    (x, y)
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        // The whole window is client area; the header acts as the caption.
        WM_NCCALCSIZE if wparam.0 != 0 => return LRESULT(0),
        WM_ERASEBKGND => return LRESULT(1),
        WM_DESTROY => {
            let state = STATE.with(|cell| cell.try_borrow_mut().ok().and_then(|mut guard| guard.take()));
            drop(state);
            trim_memory();
            return LRESULT(0);
        }
        _ => {}
    }

    // Re-entrant messages (sent while a handler runs) fall through to the
    // default procedure instead of aliasing the state.
    let outcome = STATE.with(|cell| {
        let mut guard = cell.try_borrow_mut().ok()?;
        let state = guard.as_mut().filter(|state| state.hwnd == hwnd)?;
        state.handle(message, wparam, lparam)
    });
    match outcome {
        Some((result, effect)) => {
            effect.run(hwnd);
            result
        }
        None => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

impl State {
    fn handle(&mut self, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<(LRESULT, Effect)> {
        let handled = Some((LRESULT(0), Effect::None));
        match message {
            WM_PAINT => {
                self.paint();
                handled
            }
            WM_NCHITTEST => {
                let (x, y) = point_from(lparam);
                let mut point = POINT { x, y };
                unsafe {
                    let _ = ScreenToClient(self.hwnd, &mut point);
                }
                let on_button = matches!(self.hit(point.x, point.y), Some(Control::Minimize | Control::Close));
                let caption = self.layout.header.contains(point.x, point.y) && !on_button;
                Some((LRESULT(if caption { HTCAPTION } else { HTCLIENT } as isize), Effect::None))
            }
            WM_SETCURSOR if (lparam.0 & 0xFFFF) as u32 == HTCLIENT && self.hover.is_some() => {
                unsafe {
                    if let Ok(cursor) = LoadCursorW(None, IDC_HAND) {
                        SetCursor(Some(cursor));
                    }
                }
                Some((LRESULT(1), Effect::None))
            }
            WM_MOUSEMOVE => {
                let (x, y) = point_from(lparam);
                self.on_mouse_move(x, y);
                handled
            }
            WM_MOUSELEAVE => {
                self.tracking_mouse = false;
                self.set_hover(None);
                handled
            }
            WM_LBUTTONDOWN => {
                let (x, y) = point_from(lparam);
                self.on_button_down(x, y);
                handled
            }
            WM_LBUTTONUP => {
                let (x, y) = point_from(lparam);
                Some((LRESULT(0), self.on_button_up(x, y)))
            }
            WM_CAPTURECHANGED => {
                if self.dragging.take().is_some() {
                    self.commit();
                    self.invalidate();
                }
                handled
            }
            WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize => Some((LRESULT(0), Effect::Close)),
            WM_DPICHANGED => {
                let dpi = ((wparam.0 >> 16) & 0xFFFF) as u32;
                let suggested = unsafe { *(lparam.0 as *const RECT) };
                self.set_dpi(dpi);
                let size = self.layout.size;
                Some((LRESULT(0), Effect::Resize(Rect::new(suggested.left, suggested.top, size.w, size.h))))
            }
            _ => None,
        }
    }

    fn hit(&self, x: i32, y: i32) -> Option<Control> {
        self.layout.hit(x, y, self.draft.mode)
    }

    fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn set_dpi(&mut self, dpi: u32) {
        self.layout = Layout::new(dpi, screen_size());
        self.fonts = Fonts::new(dpi);
        self.chips = preset_chips(&self.layout);
        self.buffer = None;
        self.update_preview();
    }

    fn set_hover(&mut self, hover: Option<Control>) {
        if self.hover != hover {
            self.hover = hover;
            self.invalidate();
        }
    }

    fn on_mouse_move(&mut self, x: i32, y: i32) {
        if let Some(slider) = self.dragging {
            self.drag_slider(slider, x);
            return;
        }
        if !self.tracking_mouse {
            let mut track = TRACKMOUSEEVENT {
                cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE,
                hwndTrack: self.hwnd,
                dwHoverTime: 0,
            };
            self.tracking_mouse = unsafe { TrackMouseEvent(&mut track).is_ok() };
        }
        self.set_hover(self.hit(x, y));
    }

    fn on_button_down(&mut self, x: i32, y: i32) {
        match self.hit(x, y) {
            Some(Control::Toggle(toggle)) => self.flip(toggle),
            Some(Control::Mode(mode)) => self.edit(|settings| settings.mode = mode),
            Some(Control::Preset(index)) => self.edit(|settings| settings.colors = PRESETS[index]),
            Some(Control::Slider(slider)) => {
                self.dragging = Some(slider);
                unsafe {
                    SetCapture(self.hwnd);
                }
                self.drag_slider(slider, x);
            }
            Some(control) => {
                self.pressed = Some(control);
                self.invalidate();
            }
            None => {}
        }
    }

    fn on_button_up(&mut self, x: i32, y: i32) -> Effect {
        if self.dragging.take().is_some() {
            unsafe {
                let _ = ReleaseCapture();
            }
            self.commit();
            self.invalidate();
            return Effect::None;
        }

        let pressed = self.pressed.take();
        self.invalidate();
        if pressed.is_none() || pressed != self.hit(x, y) {
            return Effect::None;
        }
        match pressed {
            Some(Control::Minimize) => Effect::Minimize,
            Some(Control::Close | Control::Done) => Effect::Close,
            Some(Control::Swatch(index)) => Effect::PickColor(index),
            Some(Control::Defaults) => {
                // Keeps the mode and the picked colors: "defaults" resets the
                // look and behavior, not the user's own gradient.
                let current = self.draft;
                self.edit(|settings| {
                    *settings = Settings { mode: current.mode, colors: current.colors, ..Settings::default() }
                });
                Effect::None
            }
            _ => Effect::None,
        }
    }

    /// Applies a settings change immediately: preview, file and wallpaper.
    fn edit(&mut self, change: impl FnOnce(&mut Settings)) {
        change(&mut self.draft);
        self.update_preview();
        self.commit();
        self.invalidate();
    }

    fn flip(&mut self, toggle: Toggle) {
        match toggle {
            Toggle::Spotify => self.edit(|s| s.spotify = !s.spotify),
            Toggle::Browsers => self.edit(|s| s.browsers = !s.browsers),
            Toggle::Restore => self.edit(|s| s.restore_on_exit = !s.restore_on_exit),
            Toggle::Autostart => {
                let enabled = !self.autostart;
                match autostart::set_enabled(enabled) {
                    Ok(()) => self.autostart = enabled,
                    Err(error) => self.shared.set_status(crate::Status::error(format!("Автозапуск: {error}"))),
                }
                self.invalidate();
            }
        }
    }

    /// Updates the value and the preview live; the wallpaper itself is only
    /// rendered on release, since writing it is the expensive part.
    fn drag_slider(&mut self, slider: Slider, x: i32) {
        let track = self.layout.slider_track(slider);
        let t = ((x - track.x) as f32 / track.w.max(1) as f32).clamp(0.0, 1.0);
        let range = slider.range();
        let value = range.start() + ((range.end() - range.start()) as f32 * t).round() as u32;
        if slider.get(&self.draft) != value {
            slider.set(&mut self.draft, value);
            self.update_preview();
            self.invalidate();
        }
    }

    fn commit(&self) {
        apply_settings(self.draft);
    }

    fn toggle_on(&self, toggle: Toggle) -> bool {
        match toggle {
            Toggle::Spotify => self.draft.spotify,
            Toggle::Browsers => self.draft.browsers,
            Toggle::Autostart => self.autostart,
            Toggle::Restore => self.draft.restore_on_exit,
        }
    }

    fn update_preview(&mut self) {
        let source = match self.draft.mode {
            Mode::Custom => PreviewSource::Colors(self.draft.colors),
            Mode::Music => match self.shared.cover() {
                Some(cover) => PreviewSource::Cover(cover),
                None => {
                    self.preview = None;
                    return;
                }
            },
        };
        let look = self.draft.look();
        let (width, height) = (self.layout.preview.w, self.layout.preview.h);
        let fresh = self
            .preview
            .as_ref()
            .is_some_and(|p| p.source == source && p.look == look && p.width == width && p.height == height);
        if fresh {
            return;
        }

        let custom_cover;
        let cover: &[u8] = match &source {
            PreviewSource::Cover(cover) => cover,
            PreviewSource::Colors(colors) => {
                custom_cover = corner_cover(*colors);
                &custom_cover
            }
        };
        let (screen_w, screen_h) = screen_size();
        let pixels = Gradient::new(cover, &look, screen_w, screen_h).to_bgra(width as u32, height as u32);
        self.preview = Some(Preview { source, look, width, height, pixels });
    }

    fn paint(&mut self) {
        let size = self.layout.size;
        let mut buffer = match self.buffer.take() {
            Some(buffer) if buffer.width == size.w && buffer.height == size.h => Some(buffer),
            _ => BackBuffer::new(size.w, size.h),
        };

        let mut ps = PAINTSTRUCT::default();
        unsafe {
            let dc = BeginPaint(self.hwnd, &mut ps);
            if let Some(buffer) = buffer.as_mut() {
                self.draw_shapes(&mut buffer.canvas());
                self.draw_text(buffer);
                let _ = BitBlt(dc, 0, 0, size.w, size.h, Some(buffer.dc()), 0, 0, SRCCOPY);
            }
            let _ = EndPaint(self.hwnd, &ps);
        }
        self.buffer = buffer;
    }

    fn is_active(&self, control: Control) -> bool {
        self.hover == Some(control) || self.pressed == Some(control)
    }

    fn draw_shapes(&self, c: &mut Canvas) {
        let l = &self.layout;
        let (radius, radius_sm) = (l.radius(theme::RADIUS), l.radius(theme::RADIUS_SM));
        let line = l.s(1).max(1);

        c.fill(l.size, theme::BG);

        for control in [Control::Minimize, Control::Close] {
            if self.is_active(control) {
                let r = if control == Control::Close { l.close } else { l.minimize };
                c.fill_round(r, radius_sm, theme::SURFACE_2);
            }
        }
        c.fill_round(l.logo, radius_sm, theme::WHITE);
        c.fill(Rect::new(0, l.header.bottom() - line, l.size.w, line), theme::LINE);

        match &self.preview {
            Some(preview) => {
                c.blit(l.preview, &preview.pixels);
                c.round_corners(l.preview, radius, theme::BG);
                c.stroke_round(l.preview, radius, theme::LINE_2);
            }
            None => {
                c.fill_round(l.preview, radius, theme::SURFACE);
                c.stroke_round(l.preview, radius, theme::LINE);
            }
        }

        let dot_color = match self.shared.status().kind {
            StatusKind::Playing | StatusKind::Custom => theme::OK,
            StatusKind::Idle => theme::DIM,
            StatusKind::Error => theme::ERR,
        };
        let dot = l.s(STATUS_DOT);
        c.fill_round(Rect::new(l.status.x, l.status.center_y() - dot / 2, dot, dot), dot as f32, dot_color);

        c.fill_round(l.mode, radius, theme::SURFACE);
        c.stroke_round(l.mode, radius, theme::LINE);
        let segment = l.segment(self.draft.mode);
        c.fill_round(segment, radius_sm, theme::LINE_2);

        for card in [l.source_card, l.look_card, l.system_card] {
            c.fill_round(card, radius, theme::SURFACE);
            c.stroke_round(card, radius, theme::LINE);
        }
        let separator =
            |c: &mut Canvas, row: Rect| c.fill(Rect::new(row.x + line, row.y, row.w - 2 * line, line), theme::LINE);

        separator(c, l.row(l.source_card, 1, ROW_H));
        match self.draft.mode {
            Mode::Music => SOURCE_TOGGLES.into_iter().for_each(|t| self.draw_switch(c, t)),
            Mode::Custom => self.draw_colors(c),
        }

        separator(c, l.row(l.system_card, 1, ROW_H));
        SYSTEM_TOGGLES.into_iter().for_each(|t| self.draw_switch(c, t));

        for (i, slider) in SLIDERS.into_iter().enumerate() {
            if i > 0 {
                separator(c, l.slider_row(slider));
            }
            self.draw_slider(c, slider);
        }

        c.fill(l.footer_line, theme::LINE);
        if self.is_active(Control::Defaults) {
            c.fill_round(l.defaults, radius_sm, theme::SURFACE);
        }
        c.stroke_round(l.defaults, radius_sm, theme::LINE_3);
        let done = if self.is_active(Control::Done) { theme::WHITE_HOVER } else { theme::WHITE };
        c.fill_round(l.done, radius_sm, done);
    }

    fn draw_colors(&self, c: &mut Canvas) {
        let l = &self.layout;
        let radius = l.radius(theme::RADIUS_SM);
        let ring = l.s(3);

        for (i, color) in self.draft.colors.iter().enumerate() {
            let r = l.swatch(i);
            if self.is_active(Control::Swatch(i)) {
                c.stroke_round(r.outset(ring), radius + ring as f32, theme::LINE_3);
            }
            c.fill_round(r, radius, Rgb::hex(*color));
            c.stroke_round(r, radius, theme::LINE_3);
        }

        for (i, (chip, preset)) in self.chips.iter().zip(PRESETS).enumerate() {
            let r = l.preset(i);
            let ring_color = if preset == self.draft.colors {
                Some(theme::WHITE)
            } else {
                self.is_active(Control::Preset(i)).then_some(theme::LINE_3)
            };
            if let Some(ring_color) = ring_color {
                c.stroke_round(r.outset(ring), radius + ring as f32, ring_color);
            }
            c.blit(r, chip);
            c.round_corners(r, radius, theme::SURFACE);
        }
    }

    fn draw_switch(&self, c: &mut Canvas, toggle: Toggle) {
        let l = &self.layout;
        let r = l.switch(toggle);
        let on = self.toggle_on(toggle);
        let hovered = self.hover == Some(Control::Toggle(toggle));
        let track = match (on, hovered) {
            (true, false) => theme::WHITE,
            (true, true) => theme::WHITE_HOVER,
            (false, false) => theme::LINE_2,
            (false, true) => theme::LINE_3,
        };
        c.fill_round(r, r.h as f32 / 2.0, track);

        let inset = l.s(SWITCH_INSET);
        let knob = r.h - 2 * inset;
        let x = if on { r.right() - inset - knob } else { r.x + inset };
        c.fill_round(Rect::new(x, r.y + inset, knob, knob), knob as f32, if on { theme::BG } else { theme::DIM });
    }

    fn draw_slider(&self, c: &mut Canvas, slider: Slider) {
        let l = &self.layout;
        let track = l.slider_track(slider);
        let filled = (track.w as f32 * slider.fraction(&self.draft)).round() as i32;
        let round = track.h as f32 / 2.0;
        c.fill_round(track, round, theme::LINE_2);
        c.fill_round(Rect::new(track.x, track.y, filled.max(track.h), track.h), round, theme::TEXT_STRONG);

        let active = self.dragging == Some(slider) || self.hover == Some(Control::Slider(slider));
        let knob = l.s(if active { SLIDER_KNOB_ACTIVE } else { SLIDER_KNOB });
        let knob_rect = Rect::new(track.x + filled - knob / 2, track.center_y() - knob / 2, knob, knob);
        c.fill_round(knob_rect, knob as f32, theme::WHITE);
    }

    fn draw_row_text(
        &self,
        dc: windows::Win32::Graphics::Gdi::HDC,
        row: Rect,
        right: i32,
        title: &str,
        subtitle: &str,
    ) {
        let l = &self.layout;
        let f = &self.fonts;
        let x = row.x + l.s(ROW_PAD);
        let w = right - x - l.s(ROW_PAD);
        let title_style = TextStyle { font: &f.body, color: theme::TEXT_STRONG, align: Align::Left, tracking: 0 };
        let subtitle_style = TextStyle { font: &f.small, color: theme::DIM, align: Align::Left, tracking: 0 };
        draw_text(dc, Rect::new(x, row.y + l.s(8), w, l.s(18)), title, &title_style);
        draw_text(dc, Rect::new(x, row.y + l.s(27), w, l.s(16)), subtitle, &subtitle_style);
    }

    fn draw_text(&self, buffer: &BackBuffer) {
        let l = &self.layout;
        let f = &self.fonts;
        let dc = buffer.dc();
        let style = |font, color, align| TextStyle { font, color, align, tracking: 0 };

        draw_text(dc, l.logo, "S", &style(&f.logo, theme::BLACK, Align::Center));
        draw_text(dc, l.title, APP_NAME, &style(&f.strong, theme::TEXT_STRONG, Align::Left));
        let version_x = l.title.x + text_width(dc, APP_NAME, &f.strong) + l.s(8);
        let version = Rect::new(version_x, l.title.y, l.s(80), l.title.h);
        draw_text(dc, version, APP_VERSION, &style(&f.mono, theme::DIM, Align::Left));

        let icon_color = |control: Control, hover: Rgb| if self.is_active(control) { hover } else { theme::MUTED };
        let minimize_color = icon_color(Control::Minimize, theme::TEXT_STRONG);
        draw_text(dc, l.minimize, ICON_MINIMIZE, &style(&f.icons, minimize_color, Align::Center));
        draw_text(dc, l.close, ICON_CLOSE, &style(&f.icons, icon_color(Control::Close, theme::ERR), Align::Center));

        if self.preview.is_none() {
            draw_text(dc, l.preview, MSG_NO_COVER, &style(&f.small, theme::DIM, Align::Center));
        }
        let status = self.shared.status();
        let status_text_x = l.s(STATUS_DOT + 8);
        let status_rect = Rect::new(l.status.x + status_text_x, l.status.y, l.status.w - status_text_x, l.status.h);
        let status_color = match status.kind {
            StatusKind::Playing | StatusKind::Custom => theme::TEXT_STRONG,
            StatusKind::Idle | StatusKind::Error => theme::MUTED,
        };
        draw_text(dc, status_rect, &status.text, &style(&f.body, status_color, Align::Left));

        let eyebrow = TextStyle { font: &f.eyebrow, color: theme::DIM, align: Align::Left, tracking: l.s(1) };
        let source_title = match self.draft.mode {
            Mode::Music => "ИСТОЧНИКИ",
            Mode::Custom => "ЦВЕТА",
        };
        draw_text(dc, l.source_label, source_title, &eyebrow);
        draw_text(dc, l.mode_label, "РЕЖИМ", &eyebrow);
        draw_text(dc, l.look_label, "ВИД", &eyebrow);
        draw_text(dc, l.system_label, "СИСТЕМА", &eyebrow);

        for mode in MODES {
            let color = if mode == self.draft.mode {
                theme::TEXT_STRONG
            } else if self.is_active(Control::Mode(mode)) {
                theme::TEXT
            } else {
                theme::DIM
            };
            draw_text(dc, l.segment(mode), mode_label(mode), &style(&f.strong, color, Align::Center));
        }

        match self.draft.mode {
            Mode::Music => {
                for toggle in SOURCE_TOGGLES {
                    let (title, subtitle) = toggle.texts();
                    self.draw_row_text(dc, l.toggle_row(toggle), l.switch(toggle).x, title, subtitle);
                }
            }
            Mode::Custom => {
                let colors_row = l.row(l.source_card, 0, ROW_H);
                self.draw_row_text(dc, colors_row, l.swatch(0).x, "Углы", "Нажмите на цвет");
                let presets_row = l.row(l.source_card, 1, ROW_H);
                self.draw_row_text(dc, presets_row, l.preset(0).x, "Пресеты", "Готовые наборы");
            }
        }
        for toggle in SYSTEM_TOGGLES {
            let (title, subtitle) = toggle.texts();
            self.draw_row_text(dc, l.toggle_row(toggle), l.switch(toggle).x, title, subtitle);
        }

        for slider in SLIDERS {
            let row = l.slider_row(slider);
            let label = Rect::new(row.x + l.s(ROW_PAD), row.y, l.s(SLIDER_LABEL_W), row.h);
            draw_text(dc, label, slider.label(), &style(&f.body, theme::TEXT, Align::Left));
            let value = Rect::new(row.right() - l.s(ROW_PAD + SLIDER_VALUE_W), row.y, l.s(SLIDER_VALUE_W), row.h);
            let text = format!("{}{}", slider.get(&self.draft), slider.unit());
            draw_text(dc, value, &text, &style(&f.mono, theme::MUTED, Align::Right));
        }

        draw_text(dc, l.hint, MSG_HINT, &style(&f.small, theme::DIM, Align::Left));
        draw_text(dc, l.defaults, "По умолчанию", &style(&f.strong, theme::TEXT_STRONG, Align::Center));
        draw_text(dc, l.done, "Готово", &style(&f.strong, theme::BLACK, Align::Center));
    }
}
