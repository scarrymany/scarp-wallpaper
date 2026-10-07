//! Settings window, drawn entirely by hand.
//!
//! There are no child controls: one back buffer, a hit-test over a static
//! layout and a handful of mouse handlers. The window and everything it owns
//! is freed when it is closed; the app itself keeps running in the tray.
//!
//! Every visual state change runs on springs (see `anim`), colors included:
//! switching the theme or a new cover blends the whole palette. A frame
//! timer exists only while something moves; at rest the window costs nothing.
//!
//! Until a theme is chosen, the window opens on a theme picker.

use std::cell::{Cell, RefCell};
use std::ops::RangeInclusive;
use std::sync::Arc;
use std::time::Instant;

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_BORDER_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, EndPaint, HDC, InvalidateRect, PAINTSTRUCT, SRCCOPY, ScreenToClient,
};
use windows::Win32::UI::Controls::Dialogs::{CC_ANYCOLOR, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW, ChooseColorW};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HICON, HTCAPTION, HTCLIENT, IDC_ARROW, IDC_HAND, KillTimer,
    LoadCursorW, RegisterClassExW, SPI_GETCLIENTAREAANIMATION, SPI_GETWORKAREA, SW_MINIMIZE, SW_RESTORE, SW_SHOW,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    SetCursor, SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, SystemParametersInfoW, WINDOW_EX_STYLE,
    WM_CAPTURECHANGED, WM_DESTROY, WM_DPICHANGED, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_NCCALCSIZE, WM_NCHITTEST, WM_PAINT, WM_SETCURSOR, WM_SETTINGCHANGE, WM_TIMER, WNDCLASSEXW,
    WS_CAPTION, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU,
};
use windows::core::{BOOL, HSTRING, PCWSTR, Result, w};

use super::anim::{Motion, Params};
use super::canvas::{Align, BackBuffer, Canvas, Font, Rect, Rgb, TextStyle, draw_text, scale, text_width};
use super::theme::{self, Palette};
use super::{apply_settings, with_app};
use crate::render::{Gradient, Look, corner_cover};
use crate::settings::{BLUR_RANGE, BRIGHTNESS_RANGE, Corners, Mode, PRESETS, SATURATION_RANGE, Settings, Theme};
use crate::{APP_NAME, APP_VERSION, Shared, Status, StatusKind, accent, autostart, screen_size, trim_memory};

const CLASS: PCWSTR = w!("ScarpWallpaperSettings");
const FRAME_TIMER: usize = 1;
/// Requested frame interval; Windows rounds it up to the timer resolution.
const FRAME_MS: u32 = 8;

/// Spring tunings, in the spirit of Island: quick and critically damped for
/// feedback, a touch of overshoot for things that physically move.
mod springs {
    use super::Params;

    pub const HOVER: Params = Params::new(0.18, 1.0);
    pub const PRESS: Params = Params::new(0.12, 1.0);
    pub const SWITCH: Params = Params::new(0.3, 0.78);
    pub const SEGMENT: Params = Params::new(0.34, 0.86);
    pub const SLIDER: Params = Params::new(0.32, 0.9);
    pub const KNOB: Params = Params::new(0.24, 0.72);
    pub const RING: Params = Params::new(0.24, 0.72);
    pub const FADE: Params = Params::new(0.38, 1.0);
    /// Palette blends: slow enough to read as a change of mood.
    pub const PALETTE: Params = Params::new(0.6, 1.0);
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
const SWITCH_W: i32 = 38;
const SWITCH_H: i32 = 22;
const SWITCH_INSET: i32 = 3;
const SWATCH: i32 = 26;
const SWATCH_GAP: i32 = 8;
const CHIP_GAP: i32 = 6;
const SLIDER_LABEL_W: i32 = 110;
const SLIDER_VALUE_W: i32 = 48;
const SLIDER_TRACK_H: i32 = 6;
const SLIDER_KNOB: i32 = 16;
const SLIDER_KNOB_ACTIVE: i32 = 18;
const SLIDER_HIT_SLOP: i32 = 8;
const BUTTON_H: i32 = 34;
const DONE_W: i32 = 100;
const DEFAULTS_W: i32 = 136;
const RADIUS: f32 = 14.0;
const RADIUS_SM: f32 = 8.0;

// Theme picker, 96 DPI pixels.
const PICKER_PAD: i32 = 40;
const PICKER_CARD_GAP: i32 = 16;
const PICKER_CARDS_Y: i32 = 172;
const PICKER_CARD_H: i32 = 210;
const PICKER_CARD_PAD: i32 = 8;
const PICKER_THUMB_H: i32 = 130;
const PICKER_RADIO: i32 = 12;
const PICKER_BUTTON_W: i32 = 140;
const PICKER_BUTTON_H: i32 = 36;

const ICON_MINIMIZE: &str = "\u{E921}";
const ICON_CLOSE: &str = "\u{E8BB}";
const MSG_NO_COVER: &str = "Включите трек в Spotify или SoundCloud";
const MSG_HINT: &str = "Окно можно закрыть - приложение останется в трее";
const MSG_PICKER_EYEBROW: &str = "ПЕРВЫЙ ЗАПУСК";
const MSG_PICKER_TITLE: &str = "Выберите оформление";
const MSG_PICKER_SUBTITLE: &str = "Потом его можно сменить в настройках, раздел «Система».";
const MSG_PICKER_HINT: &str = "Акцент и Пастель берут цвет из обложки";
/// Chips show the raw corner colors, without the user's blur and grading.
const CHIP_LOOK: Look = Look { blur_radius: 0, saturation: 1.0, brightness: 1.0 };

const MODES: [Mode; 2] = [Mode::Music, Mode::Custom];

fn mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::Music => "Музыка",
        Mode::Custom => "Свой градиент",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
/// Row of the system card holding the theme chips.
const THEME_ROW: usize = SYSTEM_TOGGLES.len();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Screen {
    Picker,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Control {
    Minimize,
    Close,
    Mode(Mode),
    Toggle(Toggle),
    Swatch(usize),
    Preset(usize),
    Slider(Slider),
    /// Theme chip in the settings, or a card in the picker.
    Theme(usize),
    Defaults,
    Done,
    /// Picker: confirm the theme.
    Continue,
}

impl Control {
    fn all() -> impl Iterator<Item = Self> {
        [Self::Minimize, Self::Close, Self::Defaults, Self::Done, Self::Continue]
            .into_iter()
            .chain(MODES.map(Self::Mode))
            .chain(SOURCE_TOGGLES.into_iter().chain(SYSTEM_TOGGLES).map(Self::Toggle))
            .chain((0..Corners::default().len()).map(Self::Swatch))
            .chain((0..PRESETS.len()).map(Self::Preset))
            .chain(SLIDERS.map(Self::Slider))
            .chain((0..Theme::ALL.len()).map(Self::Theme))
    }
}

/// Animated values. Each one is a spring, mostly between 0 and 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Key {
    Hover(Control),
    Press(Control),
    Switch(Toggle),
    /// Position of the mode pill: index of the selected mode.
    Segment,
    /// Slider fill, as a fraction of the track.
    Slider(Slider),
    /// Ring around the preset matching the current colors.
    Selected(usize),
    /// Ring around the current theme.
    ThemeSelected(usize),
    /// Swatch colors, from `swatch_from` to `swatch_to`.
    Swatches,
    /// Preview crossfade from `previous` to `preview`.
    Preview,
    /// Status crossfade from `status_old` to `status`.
    Status,
    /// Left card: below 0.5 the old mode fades out, above it the new one in.
    Card,
    /// Palette blend from `palette_from` to `palette_to`.
    Palette,
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
    hint: Rect,
    defaults: Rect,
    done: Rect,
    picker_eyebrow: Rect,
    picker_title: Rect,
    picker_subtitle: Rect,
    picker_hint: Rect,
    picker_continue: Rect,
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
        let system_rows = (THEME_ROW + 1) as i32;
        let system_card = Rect::new(right, system_label.bottom() + LABEL_GAP, COLUMN, system_rows * ROW_H);

        let buttons_y = source_card.bottom().max(system_card.bottom()) + SECTION_GAP;
        let done = Rect::new(width - PAD - DONE_W, buttons_y, DONE_W, BUTTON_H);
        let defaults = Rect::new(done.x - 8 - DEFAULTS_W, buttons_y, DEFAULTS_W, BUTTON_H);
        let height = buttons_y + BUTTON_H + 20;
        let header_button_y = (HEADER_H - HEADER_BUTTON) / 2;
        let close = Rect::new(width - 10 - HEADER_BUTTON, header_button_y, HEADER_BUTTON, HEADER_BUTTON);

        let picker_w = width - 2 * PICKER_PAD;
        let picker_button_y = height - 28 - PICKER_BUTTON_H;
        let picker_continue =
            Rect::new(width - PICKER_PAD - PICKER_BUTTON_W, picker_button_y, PICKER_BUTTON_W, PICKER_BUTTON_H);
        let picker_hint = Rect::new(PICKER_PAD, picker_button_y, picker_continue.x - PICKER_PAD - 12, PICKER_BUTTON_H);

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
            hint: Rect::new(PAD, buttons_y, defaults.x - PAD - 12, BUTTON_H).scale(dpi),
            defaults: defaults.scale(dpi),
            done: done.scale(dpi),
            picker_eyebrow: Rect::new(PICKER_PAD, HEADER_H + 24, picker_w, 16).scale(dpi),
            picker_title: Rect::new(PICKER_PAD, HEADER_H + 44, picker_w, 36).scale(dpi),
            picker_subtitle: Rect::new(PICKER_PAD, HEADER_H + 82, picker_w, 18).scale(dpi),
            picker_hint: picker_hint.scale(dpi),
            picker_continue: picker_continue.scale(dpi),
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

    fn theme_row(&self) -> Rect {
        self.row(self.system_card, THEME_ROW, ROW_H)
    }

    fn theme_chip(&self, index: usize) -> Rect {
        self.square(self.theme_row(), index, Theme::ALL.len(), SWATCH_GAP)
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

    fn picker_card(&self, index: usize) -> Rect {
        let count = Theme::ALL.len() as i32;
        let (gap, pad) = (self.s(PICKER_CARD_GAP), self.s(PICKER_PAD));
        let w = (self.size.w - 2 * pad - (count - 1) * gap) / count;
        Rect::new(pad + index as i32 * (w + gap), self.s(PICKER_CARDS_Y), w, self.s(PICKER_CARD_H))
    }

    fn picker_thumb(&self, index: usize) -> Rect {
        let card = self.picker_card(index);
        let pad = self.s(PICKER_CARD_PAD);
        Rect::new(card.x + pad, card.y + pad, card.w - 2 * pad, self.s(PICKER_THUMB_H))
    }

    /// Size of the cover block inside a picker thumbnail.
    fn picker_cover(&self) -> (i32, i32) {
        let inner = self.picker_thumb(0).w - 2 * self.s(8);
        ((inner - self.s(6)) / 2, self.s(44))
    }

    fn hit(&self, x: i32, y: i32, mode: Mode, screen: Screen) -> Option<Control> {
        let header = [(self.minimize, Control::Minimize), (self.close, Control::Close)];
        if let Some((_, control)) = header.iter().find(|(r, _)| r.contains(x, y)) {
            return Some(*control);
        }
        if screen == Screen::Picker {
            if self.picker_continue.contains(x, y) {
                return Some(Control::Continue);
            }
            return (0..Theme::ALL.len()).find(|i| self.picker_card(*i).contains(x, y)).map(Control::Theme);
        }

        let footer = [(self.defaults, Control::Defaults), (self.done, Control::Done)];
        if let Some((_, control)) = footer.iter().find(|(r, _)| r.contains(x, y)) {
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
        if let Some(i) = (0..Theme::ALL.len()).find(|i| self.theme_chip(*i).contains(x, y)) {
            return Some(Control::Theme(i));
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
    title: Font,
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
            logo: Font::new(theme::FONT, 13, 700, dpi),
            title: Font::new(theme::FONT, 26, 600, dpi),
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

/// What the preview area shows.
enum Frame {
    /// Placeholder: no cover yet.
    Empty,
    Image(Preview),
}

impl Frame {
    fn image(&self) -> Option<&Preview> {
        match self {
            Self::Empty => None,
            Self::Image(preview) => Some(preview),
        }
    }
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
    screen: Screen,
    /// Theme highlighted in the picker.
    picked: Theme,
    hover: Option<Control>,
    pressed: Option<Control>,
    dragging: Option<Slider>,
    tracking_mouse: bool,
    preview: Frame,
    /// The frame being faded out; dropped as soon as the fade ends, so at
    /// most two frames are ever held.
    previous: Option<Frame>,
    /// Dominant color of the current picture, for the tinted themes.
    cover_color: Option<u32>,
    palette_from: Palette,
    palette_to: Palette,
    /// Status as displayed, and the one fading out.
    status: Status,
    status_old: Option<Status>,
    card_from: Mode,
    card_to: Mode,
    swatch_from: Corners,
    swatch_to: Corners,
    motion: Motion<Key>,
    /// Time of the last frame while the frame timer runs.
    last_frame: Option<Instant>,
    /// Preset thumbnails at the current DPI, BGRA.
    chips: Vec<Vec<u8>>,
    /// Cover block of the picker thumbnails, BGRA.
    picker_cover: Vec<u8>,
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

    let draft = shared.settings();
    let picked = draft.theme.unwrap_or(Theme::Graphite);
    let palette = Palette::new(picked, None);
    style_frame(hwnd, &palette, picked);
    let mut state = State {
        hwnd,
        draft,
        status: shared.status(),
        shared,
        fonts: Fonts::new(dpi),
        chips: preset_chips(&layout),
        picker_cover: picker_cover(&layout),
        layout,
        autostart: autostart::is_enabled(),
        screen: if draft.theme.is_some() { Screen::Settings } else { Screen::Picker },
        picked,
        hover: None,
        pressed: None,
        dragging: None,
        tracking_mouse: false,
        preview: Frame::Empty,
        previous: None,
        cover_color: None,
        palette_from: palette,
        palette_to: palette,
        status_old: None,
        card_from: draft.mode,
        card_to: draft.mode,
        swatch_from: draft.colors,
        swatch_to: draft.colors,
        // Starts at rest: the first frame shows everything in place, already
        // tinted with the current cover.
        motion: Motion::new(false),
        last_frame: None,
        buffer: None,
    };
    state.update_preview();
    state.sync();
    state.motion.enabled = animations_enabled();
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
        state.update_status();
        state.changed();
    });
}

/// The "Animate controls and elements inside windows" system setting.
fn animations_enabled() -> bool {
    let mut enabled = BOOL::from(true);
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&raw mut enabled).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    enabled.as_bool()
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

fn picker_cover(layout: &Layout) -> Vec<u8> {
    let (w, h) = layout.picker_cover();
    let (w, h) = (w.max(1) as u32, h.max(1) as u32);
    Gradient::new(&corner_cover(PRESETS[0]), &CHIP_LOOK, w, h).to_bgra(w, h)
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
        state.changed();
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

/// Light or dark frame, rounded corners and a border in the theme's line
/// color on Windows 11. Older systems ignore the attributes.
fn style_frame(hwnd: HWND, palette: &Palette, theme: Theme) {
    unsafe {
        let dark = BOOL::from(theme != Theme::Pastel);
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
        let border = palette.line.colorref();
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

/// 1.0 for `true`, 0.0 for `false`: spring targets.
fn unit(on: bool) -> f32 {
    if on { 1.0 } else { 0.0 }
}

fn status_dot(kind: StatusKind, p: &Palette) -> Rgb {
    match kind {
        StatusKind::Playing | StatusKind::Custom => p.ok,
        StatusKind::Idle => p.dim,
        StatusKind::Error => p.err,
    }
}

fn status_text(kind: StatusKind, p: &Palette) -> Rgb {
    match kind {
        StatusKind::Playing | StatusKind::Custom => p.strong,
        StatusKind::Idle | StatusKind::Error => p.muted,
    }
}

/// Shrinks a button slightly while it is held.
fn pressed_rect(r: Rect, press: f32) -> Rect {
    let d = (r.h as f32 * 0.04 * press.clamp(0.0, 1.0)).round() as i32;
    Rect::new(r.x + d, r.y + d, r.w - 2 * d, r.h - 2 * d)
}

fn offset_y(r: Rect, dy: i32) -> Rect {
    Rect::new(r.x, r.y + dy, r.w, r.h)
}

/// Corner radius that makes a fully rounded pill.
fn pill(r: Rect) -> f32 {
    r.h as f32 / 2.0
}

impl State {
    fn handle(&mut self, message: u32, wparam: WPARAM, lparam: LPARAM) -> Option<(LRESULT, Effect)> {
        let handled = Some((LRESULT(0), Effect::None));
        match message {
            WM_PAINT => {
                self.paint();
                handled
            }
            WM_TIMER if wparam.0 == FRAME_TIMER => {
                self.tick();
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
                    self.changed();
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
            WM_SETTINGCHANGE => {
                self.motion.enabled = animations_enabled();
                if !self.motion.enabled {
                    self.settle();
                }
                handled
            }
            _ => None,
        }
    }

    fn hit(&self, x: i32, y: i32) -> Option<Control> {
        self.layout.hit(x, y, self.draft.mode, self.screen)
    }

    fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// Something changed: retarget the springs and repaint.
    fn changed(&mut self) {
        self.sync();
        self.invalidate();
    }

    /// The theme on screen: the highlighted one while picking.
    fn theme(&self) -> Theme {
        match self.screen {
            Screen::Picker => self.picked,
            Screen::Settings => self.draft.theme.unwrap_or(Theme::Graphite),
        }
    }

    fn palette(&self) -> Palette {
        self.palette_from.lerp(&self.palette_to, self.motion.get(Key::Palette))
    }

    /// Points every spring at the value the current state asks for. Targets
    /// that did not change are left alone, so running animations continue.
    fn sync(&mut self) {
        for control in Control::all() {
            let (hovered, params) = match control {
                Control::Slider(slider) => {
                    (self.hover == Some(control) || self.dragging == Some(slider), springs::KNOB)
                }
                _ => (self.hover == Some(control), springs::HOVER),
            };
            self.motion.to(Key::Hover(control), unit(hovered), params);
            self.motion.to(Key::Press(control), unit(self.pressed == Some(control)), springs::PRESS);
        }
        for toggle in SOURCE_TOGGLES.into_iter().chain(SYSTEM_TOGGLES) {
            self.motion.to(Key::Switch(toggle), unit(self.toggle_on(toggle)), springs::SWITCH);
        }
        let segment = MODES.iter().position(|m| *m == self.draft.mode).unwrap_or_default();
        self.motion.to(Key::Segment, segment as f32, springs::SEGMENT);
        for slider in SLIDERS {
            let fraction = slider.fraction(&self.draft);
            // Direct manipulation follows the cursor exactly.
            if self.dragging == Some(slider) {
                self.motion.snap(Key::Slider(slider), fraction, springs::SLIDER);
            } else {
                self.motion.to(Key::Slider(slider), fraction, springs::SLIDER);
            }
        }
        for (i, preset) in PRESETS.iter().enumerate() {
            self.motion.to(Key::Selected(i), unit(*preset == self.draft.colors), springs::RING);
        }
        let theme = self.theme();
        for (i, candidate) in Theme::ALL.into_iter().enumerate() {
            self.motion.to(Key::ThemeSelected(i), unit(candidate == theme), springs::RING);
        }

        if self.draft.mode != self.card_to {
            // Continues from whatever is visible, even mid-fade.
            let (visible, visibility) = self.card();
            let start = if visible == self.draft.mode { 0.5 + visibility / 2.0 } else { (1.0 - visibility) / 2.0 };
            self.card_from = visible;
            self.card_to = self.draft.mode;
            self.motion.restart(Key::Card, start, 1.0, springs::FADE);
        }
        if self.draft.colors != self.swatch_to {
            self.swatch_from = self.swatch_colors().map(Rgb::to_hex);
            self.swatch_to = self.draft.colors;
            self.motion.restart(Key::Swatches, 0.0, 1.0, springs::FADE);
        }
        let palette = Palette::new(theme, self.cover_color);
        if palette != self.palette_to {
            self.palette_from = self.palette();
            self.palette_to = palette;
            self.motion.restart(Key::Palette, 0.0, 1.0, springs::PALETTE);
            style_frame(self.hwnd, &palette, theme);
        }

        self.start_frames();
    }

    /// Runs the frame timer while anything moves.
    fn start_frames(&mut self) {
        if self.last_frame.is_none() && self.motion.is_moving() {
            unsafe {
                SetTimer(Some(self.hwnd), FRAME_TIMER, FRAME_MS, None);
            }
            self.last_frame = Some(Instant::now());
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let dt = self.last_frame.map_or(0.0, |last| (now - last).as_secs_f32());
        self.last_frame = Some(now);
        let moving = self.motion.step(dt);
        self.drop_finished_fades();
        self.invalidate();
        if !moving {
            self.last_frame = None;
            unsafe {
                let _ = KillTimer(Some(self.hwnd), FRAME_TIMER);
            }
        }
    }

    /// Jumps every animation to its end, for the reduced-motion setting.
    fn settle(&mut self) {
        self.motion.finish();
        self.drop_finished_fades();
        self.invalidate();
    }

    fn drop_finished_fades(&mut self) {
        if self.motion.get(Key::Preview) >= 1.0 {
            self.previous = None;
        }
        if self.motion.get(Key::Status) >= 1.0 {
            self.status_old = None;
        }
    }

    /// The mode the left card shows right now and how opaque it is.
    fn card(&self) -> (Mode, f32) {
        let phase = self.motion.get(Key::Card).clamp(0.0, 1.0);
        if phase < 0.5 { (self.card_from, 1.0 - 2.0 * phase) } else { (self.card_to, 2.0 * phase - 1.0) }
    }

    fn swatch_colors(&self) -> [Rgb; 4] {
        let t = self.motion.get(Key::Swatches);
        std::array::from_fn(|i| Rgb::hex(self.swatch_from[i]).lerp(Rgb::hex(self.swatch_to[i]), t))
    }

    /// Progress of a crossfade; 1 when nothing is fading.
    fn fade(&self, key: Key, fading: bool) -> f32 {
        if fading { self.motion.get(key).clamp(0.0, 1.0) } else { 1.0 }
    }

    fn hover(&self, control: Control) -> f32 {
        self.motion.get(Key::Hover(control)).clamp(0.0, 1.0)
    }

    fn press(&self, control: Control) -> f32 {
        self.motion.get(Key::Press(control)).clamp(0.0, 1.0)
    }

    /// Hover or press, whichever is stronger.
    fn active(&self, control: Control) -> f32 {
        self.hover(control).max(self.press(control))
    }

    fn set_dpi(&mut self, dpi: u32) {
        self.layout = Layout::new(dpi, screen_size());
        self.fonts = Fonts::new(dpi);
        self.chips = preset_chips(&self.layout);
        self.picker_cover = picker_cover(&self.layout);
        self.buffer = None;
        // The old frame has the old size; the new one is the same picture.
        self.previous = None;
        self.update_preview();
    }

    fn set_hover(&mut self, hover: Option<Control>) {
        if self.hover != hover {
            self.hover = hover;
            self.changed();
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
            Some(Control::Theme(index)) => {
                let theme = Theme::ALL[index];
                match self.screen {
                    Screen::Picker => {
                        self.picked = theme;
                        self.changed();
                    }
                    Screen::Settings => self.edit(|settings| settings.theme = Some(theme)),
                }
            }
            Some(Control::Slider(slider)) => {
                self.dragging = Some(slider);
                unsafe {
                    SetCapture(self.hwnd);
                }
                self.drag_slider(slider, x);
                self.changed();
            }
            Some(control) => {
                self.pressed = Some(control);
                self.changed();
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
            self.changed();
            return Effect::None;
        }

        let pressed = self.pressed.take();
        self.changed();
        if pressed.is_none() || pressed != self.hit(x, y) {
            return Effect::None;
        }
        match pressed {
            Some(Control::Minimize) => Effect::Minimize,
            Some(Control::Close | Control::Done) => Effect::Close,
            Some(Control::Swatch(index)) => Effect::PickColor(index),
            Some(Control::Continue) => {
                let picked = self.picked;
                self.screen = Screen::Settings;
                self.hover = self.hit(x, y);
                self.edit(|settings| settings.theme = Some(picked));
                Effect::None
            }
            Some(Control::Defaults) => {
                // Keeps the mode, the picked colors and the theme: "defaults"
                // resets the look of the wallpaper and the behavior.
                let current = self.draft;
                self.edit(|settings| {
                    *settings = Settings {
                        mode: current.mode,
                        colors: current.colors,
                        theme: current.theme,
                        ..Settings::default()
                    }
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
        self.changed();
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
                    Err(error) => self.shared.set_status(Status::error(format!("Автозапуск: {error}"))),
                }
                self.changed();
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
            self.changed();
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
            Mode::Custom => Some(PreviewSource::Colors(self.draft.colors)),
            Mode::Music => self.shared.cover().map(PreviewSource::Cover),
        };
        let look = self.draft.look();
        let (width, height) = (self.layout.preview.w, self.layout.preview.h);
        let fresh = match (self.preview.image(), &source) {
            (Some(p), Some(source)) => p.source == *source && p.look == look && p.width == width && p.height == height,
            (None, None) => true,
            _ => false,
        };
        if fresh {
            return;
        }

        let next = match source {
            None => Frame::Empty,
            Some(source) => {
                let custom_cover;
                let cover: &[u8] = match &source {
                    PreviewSource::Cover(cover) => cover,
                    PreviewSource::Colors(colors) => {
                        custom_cover = corner_cover(*colors);
                        &custom_cover
                    }
                };
                // Without a clear color the tint stays as it was.
                if let Some(color) = accent::dominant(cover) {
                    self.cover_color = Some(color);
                }
                let (screen_w, screen_h) = screen_size();
                let pixels = Gradient::new(cover, &look, screen_w, screen_h).to_bgra(width as u32, height as u32);
                Frame::Image(Preview { source, look, width, height, pixels })
            }
        };
        // Look and size changes (slider drags, DPI) swap the frame in place;
        // a new picture crossfades.
        let same_picture = matches!((self.preview.image(), next.image()), (Some(a), Some(b)) if a.source == b.source);
        let old = std::mem::replace(&mut self.preview, next);
        if same_picture || !self.motion.enabled {
            return;
        }
        // Interrupted mid-fade: keep whichever old frame is more visible.
        if self.previous.is_none() || self.motion.get(Key::Preview) >= 0.5 {
            self.previous = Some(old);
        }
        self.motion.restart(Key::Preview, 0.0, 1.0, springs::FADE);
        self.start_frames();
    }

    fn update_status(&mut self) {
        let status = self.shared.status();
        if status == self.status {
            return;
        }
        let old = std::mem::replace(&mut self.status, status);
        if !self.motion.enabled {
            return;
        }
        if self.status_old.is_none() || self.motion.get(Key::Status) >= 0.5 {
            self.status_old = Some(old);
        }
        self.motion.restart(Key::Status, 0.0, 1.0, springs::FADE);
        self.start_frames();
    }

    fn paint(&mut self) {
        let size = self.layout.size;
        let mut buffer = match self.buffer.take() {
            Some(buffer) if buffer.width == size.w && buffer.height == size.h => Some(buffer),
            _ => BackBuffer::new(size.w, size.h),
        };

        let p = self.palette();
        let mut ps = PAINTSTRUCT::default();
        unsafe {
            let dc = BeginPaint(self.hwnd, &mut ps);
            if let Some(buffer) = buffer.as_mut() {
                let mut canvas = buffer.canvas();
                self.draw_header_shapes(&mut canvas, &p);
                match self.screen {
                    Screen::Picker => self.draw_picker_shapes(&mut canvas, &p),
                    Screen::Settings => self.draw_shapes(&mut canvas, &p),
                }
                let text_dc = buffer.dc();
                self.draw_header_text(text_dc, &p);
                match self.screen {
                    Screen::Picker => self.draw_picker_text(text_dc, &p),
                    Screen::Settings => self.draw_text(text_dc, &p),
                }
                let _ = BitBlt(dc, 0, 0, size.w, size.h, Some(text_dc), 0, 0, SRCCOPY);
            }
            let _ = EndPaint(self.hwnd, &ps);
        }
        self.buffer = buffer;
    }

    /// The preview frames, bottom first, with their opacity.
    fn preview_layers(&self) -> impl Iterator<Item = (&Frame, f32)> {
        let fade = self.fade(Key::Preview, self.previous.is_some());
        self.previous.iter().map(|frame| (frame, 1.0)).chain([(&self.preview, fade)])
    }

    /// How much of the visible preview is a picture rather than the placeholder.
    fn preview_image(&self) -> f32 {
        self.preview_layers().fold(0.0, |image, (frame, alpha)| image + (unit(frame.image().is_some()) - image) * alpha)
    }

    fn draw_header_shapes(&self, c: &mut Canvas, p: &Palette) {
        let l = &self.layout;
        let line = l.s(1).max(1);
        c.fill(l.size, p.bg);
        for (control, r) in [(Control::Minimize, l.minimize), (Control::Close, l.close)] {
            let active = self.active(control);
            if active > 0.0 {
                c.fill_round(r, pill(r), p.bg.lerp(p.hover, active));
            }
        }
        c.fill_round(l.logo, pill(l.logo), p.accent);
        c.fill(Rect::new(0, l.header.bottom() - line, l.size.w, line), p.line);
    }

    fn draw_header_text(&self, dc: HDC, p: &Palette) {
        let l = &self.layout;
        let f = &self.fonts;
        let style = |font, color, align| TextStyle { font, color, align, tracking: 0 };

        draw_text(dc, l.logo, "S", &style(&f.logo, p.on_accent, Align::Center));
        draw_text(dc, l.title, APP_NAME, &style(&f.strong, p.strong, Align::Left));
        let version_x = l.title.x + text_width(dc, APP_NAME, &f.strong) + l.s(8);
        let version = Rect::new(version_x, l.title.y, l.s(80), l.title.h);
        draw_text(dc, version, APP_VERSION, &style(&f.mono, p.dim, Align::Left));

        let icon_color = |control: Control, hover: Rgb| p.muted.lerp(hover, self.active(control));
        let minimize_color = icon_color(Control::Minimize, p.strong);
        draw_text(dc, l.minimize, ICON_MINIMIZE, &style(&f.icons, minimize_color, Align::Center));
        draw_text(dc, l.close, ICON_CLOSE, &style(&f.icons, icon_color(Control::Close, p.err), Align::Center));
    }

    fn draw_shapes(&self, c: &mut Canvas, p: &Palette) {
        let l = &self.layout;
        let radius = l.radius(RADIUS);

        for (frame, alpha) in self.preview_layers() {
            match frame {
                Frame::Empty => c.fill_round_alpha(l.preview, radius, p.surface, alpha),
                Frame::Image(preview) => c.blit_alpha(l.preview, &preview.pixels, alpha),
            }
        }
        c.round_corners(l.preview, radius, p.bg);

        let status_fade = self.fade(Key::Status, self.status_old.is_some());
        let dot_color = match &self.status_old {
            Some(old) => status_dot(old.kind, p).lerp(status_dot(self.status.kind, p), status_fade),
            None => status_dot(self.status.kind, p),
        };
        let dot = l.s(STATUS_DOT);
        c.fill_round(Rect::new(l.status.x, l.status.center_y() - dot / 2, dot, dot), dot as f32, dot_color);

        c.fill_round(l.mode, pill(l.mode), p.segment);
        for mode in MODES {
            let hover = self.active(Control::Mode(mode));
            if hover > 0.0 {
                let r = l.segment(mode);
                c.fill_round(r, pill(r), p.segment.lerp(p.pill, hover * 0.5));
            }
        }
        // The pill slides between the segments, with a little overshoot
        // that stays inside the control.
        let (first, last) = (l.segment(MODES[0]), l.segment(MODES[MODES.len() - 1]));
        let travel = (last.x - first.x) as f32 / (MODES.len() - 1) as f32;
        let inset = l.s(SEGMENT_INSET) - l.s(1).max(1);
        let x = (first.x as f32 + travel * self.motion.get(Key::Segment)).round() as i32;
        let x = x.clamp(first.x - inset, last.x + inset);
        let selected = Rect::new(x, first.y, first.w, first.h);
        c.fill_round(selected, pill(selected), p.pill);

        for card in [l.source_card, l.look_card, l.system_card] {
            c.fill_round(card, radius, p.surface);
        }

        let (card_mode, visibility) = self.card();
        match card_mode {
            Mode::Music => SOURCE_TOGGLES.into_iter().for_each(|t| self.draw_switch(c, p, t, visibility)),
            Mode::Custom => self.draw_colors(c, p, visibility),
        }
        SYSTEM_TOGGLES.into_iter().for_each(|t| self.draw_switch(c, p, t, 1.0));
        self.draw_theme_chips(c, p);
        SLIDERS.into_iter().for_each(|s| self.draw_slider(c, p, s));

        let defaults = pressed_rect(l.defaults, self.press(Control::Defaults));
        c.fill_round(defaults, pill(defaults), p.secondary.lerp(p.pill, self.active(Control::Defaults)));
        let done_color =
            p.accent.lerp(p.bg, self.hover(Control::Done) * 0.12).lerp(p.bg, self.press(Control::Done) * 0.25);
        let done = pressed_rect(l.done, self.press(Control::Done));
        c.fill_round(done, pill(done), done_color);
    }

    /// Corner swatches and presets; `visibility` fades them into the card.
    fn draw_colors(&self, c: &mut Canvas, p: &Palette, visibility: f32) {
        let l = &self.layout;
        let radius = l.radius(RADIUS_SM);
        let ring = l.s(3);
        let fade_in = |color: Rgb| p.surface.lerp(color, visibility);
        let outline = p.surface.lerp(p.dim, 0.5);

        for (i, color) in self.swatch_colors().into_iter().enumerate() {
            let control = Control::Swatch(i);
            let r = pressed_rect(l.swatch(i), self.press(control));
            let hover = self.active(control) * visibility;
            if hover > 0.0 {
                c.stroke_round(r.outset(ring), radius + ring as f32, p.surface.lerp(p.dim, hover));
            }
            c.fill_round(r, radius, fade_in(color));
            c.stroke_round(r, radius, fade_in(outline));
        }

        for (i, chip) in self.chips.iter().enumerate() {
            let r = l.preset(i);
            let selected = self.motion.get(Key::Selected(i));
            self.draw_ring(c, p, r, selected, self.active(Control::Preset(i)), visibility);
            c.blit_alpha(r, chip, visibility);
            c.round_corners(r, radius, p.surface);
        }
    }

    /// Selection and hover ring around a chip; it springs outwards as it appears.
    fn draw_ring(&self, c: &mut Canvas, p: &Palette, r: Rect, selected: f32, hover: f32, visibility: f32) {
        let l = &self.layout;
        let strength = hover.max(selected).clamp(0.0, 1.0) * visibility;
        if strength <= 0.0 {
            return;
        }
        let grow = l.s(2) + (l.s(1) as f32 * selected.max(hover)).round() as i32;
        let color = p.dim.lerp(p.accent, selected);
        c.stroke_round(r.outset(grow), l.radius(RADIUS_SM) + grow as f32, p.surface.lerp(color, strength));
    }

    /// Each chip is a tiny swatch of its theme: background with an accent dot.
    fn draw_theme_chips(&self, c: &mut Canvas, p: &Palette) {
        let l = &self.layout;
        let radius = l.radius(RADIUS_SM);
        for (i, theme) in Theme::ALL.into_iter().enumerate() {
            let r = pressed_rect(l.theme_chip(i), self.press(Control::Theme(i)));
            let selected = self.motion.get(Key::ThemeSelected(i));
            self.draw_ring(c, p, r, selected, self.active(Control::Theme(i)), 1.0);
            let look = Palette::new(theme, self.cover_color);
            c.fill_round(r, radius, look.bg);
            c.stroke_round(r, radius, p.surface.lerp(p.dim, 0.5));
            let dot = r.w * 2 / 5;
            let dot_rect = Rect::new(r.x + (r.w - dot) / 2, r.y + (r.h - dot) / 2, dot, dot);
            c.fill_round(dot_rect, dot as f32, look.accent);
        }
    }

    fn draw_switch(&self, c: &mut Canvas, p: &Palette, toggle: Toggle, visibility: f32) {
        let l = &self.layout;
        let r = l.switch(toggle);
        let position = self.motion.get(Key::Switch(toggle));
        let on = position.clamp(0.0, 1.0);
        let hover = self.hover(Control::Toggle(toggle));
        let track_off = p.switch_off.lerp(p.dim, hover * 0.25);
        let track_on = p.accent.lerp(p.bg, hover * 0.12);
        let fade_in = |color: Rgb| p.surface.lerp(color, visibility);
        c.fill_round(r, pill(r), fade_in(track_off.lerp(track_on, on)));

        let inset = l.s(SWITCH_INSET);
        let knob = r.h - 2 * inset;
        let travel = (r.w - 2 * inset - knob) as f32;
        let x = (r.x as f32 + inset as f32 + travel * position).round() as i32;
        let x = x.clamp(r.x + inset / 2, r.right() - inset / 2 - knob);
        let knob_color = p.knob_off.lerp(p.knob_on, on);
        c.fill_round(Rect::new(x, r.y + inset, knob, knob), knob as f32, fade_in(knob_color));
    }

    fn draw_slider(&self, c: &mut Canvas, p: &Palette, slider: Slider) {
        let l = &self.layout;
        let track = l.slider_track(slider);
        let fraction = self.motion.get(Key::Slider(slider)).clamp(0.0, 1.0);
        let filled = (track.w as f32 * fraction).round() as i32;
        let round = track.h as f32 / 2.0;
        let active = self.motion.get(Key::Hover(Control::Slider(slider)));
        c.fill_round(track, round, p.slider_track);
        c.fill_round(Rect::new(track.x, track.y, filled.max(track.h), track.h), round, p.accent);

        let (small, large) = (l.s(SLIDER_KNOB) as f32, l.s(SLIDER_KNOB_ACTIVE) as f32);
        let knob = (small + (large - small) * active).round().max(small - 1.0) as i32;
        let knob_rect = Rect::new(track.x + filled - knob / 2, track.center_y() - knob / 2, knob, knob);
        c.fill_round(knob_rect, knob as f32, p.slider_knob);
        c.stroke_round(knob_rect, knob as f32, p.knob_ring);
    }

    fn draw_row_text(&self, dc: HDC, p: &Palette, row: Rect, right: i32, texts: (&str, &str), visibility: f32) {
        let l = &self.layout;
        let f = &self.fonts;
        let x = row.x + l.s(ROW_PAD);
        let w = right - x - l.s(ROW_PAD);
        let fade_in = |color: Rgb| p.surface.lerp(color, visibility);
        let title_style = TextStyle { font: &f.body, color: fade_in(p.strong), align: Align::Left, tracking: 0 };
        let subtitle_style = TextStyle { font: &f.small, color: fade_in(p.dim), align: Align::Left, tracking: 0 };
        draw_text(dc, Rect::new(x, row.y + l.s(8), w, l.s(18)), texts.0, &title_style);
        draw_text(dc, Rect::new(x, row.y + l.s(27), w, l.s(16)), texts.1, &subtitle_style);
    }

    fn draw_text(&self, dc: HDC, p: &Palette) {
        let l = &self.layout;
        let f = &self.fonts;
        let style = |font, color, align| TextStyle { font, color, align, tracking: 0 };

        let placeholder = 1.0 - self.preview_image();
        if placeholder > 0.0 {
            let color = p.surface.lerp(p.dim, placeholder);
            draw_text(dc, l.preview, MSG_NO_COVER, &style(&f.small, color, Align::Center));
        }

        // The old status slides up and out while the new one rises in.
        let status_text_x = l.s(STATUS_DOT + 8);
        let status_rect = Rect::new(l.status.x + status_text_x, l.status.y, l.status.w - status_text_x, l.status.h);
        let fade = self.fade(Key::Status, self.status_old.is_some());
        let shift = l.s(6) as f32;
        if let Some(old) = &self.status_old {
            let rect = offset_y(status_rect, -(shift * fade).round() as i32);
            let color = status_text(old.kind, p).lerp(p.bg, fade);
            draw_text(dc, rect, &old.text, &style(&f.body, color, Align::Left));
        }
        let rect = offset_y(status_rect, (shift * (1.0 - fade)).round() as i32);
        let color = p.bg.lerp(status_text(self.status.kind, p), fade);
        draw_text(dc, rect, &self.status.text, &style(&f.body, color, Align::Left));

        let eyebrow = |color| TextStyle { font: &f.eyebrow, color, align: Align::Left, tracking: l.s(1) };
        let (card_mode, visibility) = self.card();
        let source_title = match card_mode {
            Mode::Music => "ИСТОЧНИКИ",
            Mode::Custom => "ЦВЕТА",
        };
        draw_text(dc, l.source_label, source_title, &eyebrow(p.bg.lerp(p.dim, visibility)));
        draw_text(dc, l.mode_label, "РЕЖИМ", &eyebrow(p.dim));
        draw_text(dc, l.look_label, "ВИД", &eyebrow(p.dim));
        draw_text(dc, l.system_label, "СИСТЕМА", &eyebrow(p.dim));

        let segment = self.motion.get(Key::Segment);
        for (i, mode) in MODES.into_iter().enumerate() {
            let selected = (1.0 - (segment - i as f32).abs()).clamp(0.0, 1.0);
            let color = p.dim.lerp(p.text, self.active(Control::Mode(mode))).lerp(p.pill_text, selected);
            draw_text(dc, l.segment(mode), mode_label(mode), &style(&f.strong, color, Align::Center));
        }

        match card_mode {
            Mode::Music => {
                for toggle in SOURCE_TOGGLES {
                    self.draw_row_text(dc, p, l.toggle_row(toggle), l.switch(toggle).x, toggle.texts(), visibility);
                }
            }
            Mode::Custom => {
                let colors_row = l.row(l.source_card, 0, ROW_H);
                self.draw_row_text(dc, p, colors_row, l.swatch(0).x, ("Углы", "Нажмите на цвет"), visibility);
                let presets_row = l.row(l.source_card, 1, ROW_H);
                self.draw_row_text(dc, p, presets_row, l.preset(0).x, ("Пресеты", "Готовые наборы"), visibility);
            }
        }
        for toggle in SYSTEM_TOGGLES {
            self.draw_row_text(dc, p, l.toggle_row(toggle), l.switch(toggle).x, toggle.texts(), 1.0);
        }
        let theme_name = theme::name(self.theme());
        self.draw_row_text(dc, p, l.theme_row(), l.theme_chip(0).x, ("Оформление", theme_name), 1.0);

        for slider in SLIDERS {
            let row = l.slider_row(slider);
            let label = Rect::new(row.x + l.s(ROW_PAD), row.y, l.s(SLIDER_LABEL_W), row.h);
            draw_text(dc, label, slider.label(), &style(&f.body, p.text, Align::Left));
            let value = Rect::new(row.right() - l.s(ROW_PAD + SLIDER_VALUE_W), row.y, l.s(SLIDER_VALUE_W), row.h);
            // The number counts along with the animated fill.
            let range = slider.range();
            let fraction = self.motion.get(Key::Slider(slider)).clamp(0.0, 1.0);
            let shown = range.start() + ((range.end() - range.start()) as f32 * fraction).round() as u32;
            let text = format!("{shown}{}", slider.unit());
            draw_text(dc, value, &text, &style(&f.mono, p.muted, Align::Right));
        }

        draw_text(dc, l.hint, MSG_HINT, &style(&f.small, p.dim, Align::Left));
        draw_text(dc, l.defaults, "По умолчанию", &style(&f.strong, p.strong, Align::Center));
        draw_text(dc, l.done, "Готово", &style(&f.strong, p.on_accent, Align::Center));
    }

    fn draw_picker_shapes(&self, c: &mut Canvas, p: &Palette) {
        let l = &self.layout;
        let (radius, radius_sm) = (l.radius(RADIUS), l.radius(RADIUS_SM));
        let (cover_w, cover_h) = l.picker_cover();

        for (i, theme) in Theme::ALL.into_iter().enumerate() {
            let card = l.picker_card(i);
            let selected = self.motion.get(Key::ThemeSelected(i)).clamp(0.0, 1.0);
            let hover = self.active(Control::Theme(i));
            let card_fill = p.bg.lerp(p.surface, selected.max(hover * 0.6));
            c.fill_round(card, radius, card_fill);
            c.stroke_round(card, radius, p.line.lerp(p.dim, hover * 0.5).lerp(p.strong, selected));

            // A miniature of the window in that theme.
            let look = Palette::new(theme, self.cover_color);
            let thumb = l.picker_thumb(i);
            c.fill_round(thumb, radius_sm, look.bg);
            c.stroke_round(thumb, radius_sm, p.line);
            let pad = l.s(8);
            let (x, y) = (thumb.x + pad, thumb.y + pad);
            let bar = Rect::new(x, y, (thumb.w - 2 * pad) * 2 / 5, l.s(6));
            c.fill_round(bar, pill(bar), look.accent);
            let (top, gap, block_h) = (bar.bottom() + l.s(8), l.s(6), l.s(14));
            let cover = Rect::new(x, top, cover_w, cover_h);
            c.blit_alpha(cover, &self.picker_cover, 1.0);
            c.round_corners(cover, l.radius(4.0), look.bg);
            let column = x + cover_w + gap;
            let blocks = [
                (Rect::new(x, cover.bottom() + gap, cover_w, block_h), look.surface),
                (Rect::new(column, top, cover_w * 3 / 5, block_h), look.accent),
                (Rect::new(column, top + block_h + gap, cover_w, block_h), look.surface),
                (Rect::new(column, top + 2 * (block_h + gap), cover_w, block_h), look.surface),
            ];
            for (r, color) in blocks {
                c.fill_round(r, pill(r), color);
            }

            // Radio: a ring that fills in as the card gets selected.
            let size = l.s(PICKER_RADIO);
            let radio = Rect::new(thumb.x + l.s(2), thumb.bottom() + l.s(16), size, size);
            c.fill_round(radio, size as f32, p.dim.lerp(p.strong, selected));
            let hole = (size as f32 * (0.8 - 0.45 * selected)).round() as i32;
            let hole_rect = Rect::new(radio.x + (size - hole) / 2, radio.y + (size - hole) / 2, hole, hole);
            c.fill_round(hole_rect, hole as f32, card_fill);
        }

        let button = pressed_rect(l.picker_continue, self.press(Control::Continue));
        c.fill_round(button, pill(button), p.accent.lerp(p.bg, self.hover(Control::Continue) * 0.12));
    }

    fn draw_picker_text(&self, dc: HDC, p: &Palette) {
        let l = &self.layout;
        let f = &self.fonts;
        let style = |font, color, align| TextStyle { font, color, align, tracking: 0 };

        let eyebrow = TextStyle { font: &f.eyebrow, color: p.dim, align: Align::Left, tracking: l.s(1) };
        draw_text(dc, l.picker_eyebrow, MSG_PICKER_EYEBROW, &eyebrow);
        draw_text(dc, l.picker_title, MSG_PICKER_TITLE, &style(&f.title, p.strong, Align::Left));
        draw_text(dc, l.picker_subtitle, MSG_PICKER_SUBTITLE, &style(&f.body, p.muted, Align::Left));

        for (i, theme) in Theme::ALL.into_iter().enumerate() {
            let thumb = l.picker_thumb(i);
            let x = thumb.x + l.s(2 + PICKER_RADIO + 8);
            let name = Rect::new(x, thumb.bottom() + l.s(12), thumb.right() - x, l.s(20));
            draw_text(dc, name, theme::name(theme), &style(&f.strong, p.strong, Align::Left));
            let description = Rect::new(thumb.x + l.s(2), name.bottom() + l.s(6), thumb.w - l.s(2), l.s(18));
            draw_text(dc, description, theme::description(theme), &style(&f.small, p.muted, Align::Left));
        }

        draw_text(dc, l.picker_hint, MSG_PICKER_HINT, &style(&f.small, p.dim, Align::Left));
        draw_text(dc, l.picker_continue, "Продолжить", &style(&f.strong, p.on_accent, Align::Center));
    }
}
