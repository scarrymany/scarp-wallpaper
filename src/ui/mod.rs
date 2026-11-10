//! Tray icon, message loop and the bridge between the UI and the worker.

mod anim;
mod canvas;
mod theme;
mod window;

use std::cell::{Cell, OnceCell, RefCell};
use std::error::Error;
use std::io;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread::JoinHandle;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW, FindWindowW,
    GetCursorPos, GetMessageW, GetSystemMetrics, HICON, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, MF_GRAYED,
    MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage, RegisterClassExW, RegisterWindowMessageW, SM_CXICON,
    SM_CXSMICON, SM_CYICON, SM_CYSMICON, SYSTEM_METRICS_INDEX, SetForegroundWindow, SetMenuDefaultItem, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_ENDSESSION,
    WM_LBUTTONDBLCLK, WM_NULL, WM_RBUTTONUP, WNDCLASSEXW, WS_OVERLAPPED,
};
use windows::core::{HSTRING, PCSTR, PCWSTR, w};

use crate::settings::Settings;
use crate::{APP_NAME, Event, Shared, Status};

/// Posted by the worker whenever status or cover changes.
const WM_REFRESH: u32 = WM_APP + 1;
const WM_TRAY: u32 = WM_APP + 2;
/// Posted by a second launch to bring up the settings of this instance.
const WM_ACTIVATE_INSTANCE: u32 = WM_APP + 3;

const TRAY_CLASS: PCWSTR = w!("ScarpWallpaperTray");
const TRAY_ICON_ID: u32 = 1;
/// MAKEINTRESOURCE(1): an integer resource id passed as a pointer value.
const ICON_RESOURCE: PCWSTR = PCWSTR(std::ptr::without_provenance(1));
const MENU_SETTINGS: i32 = 1;
const MENU_EXIT: i32 = 2;
const TOOLTIP_CHARS: usize = 127;
/// uxtheme `SetPreferredAppMode`, exported by ordinal only (Windows 10 1903+).
const SET_PREFERRED_APP_MODE_ORDINAL: usize = 135;
const APP_MODE_FORCE_DARK: i32 = 2;

struct App {
    shared: Arc<Shared>,
    events: Sender<Event>,
    worker: RefCell<Option<JoinHandle<()>>>,
    instance: HINSTANCE,
    small_icon: HICON,
    tray: Cell<HWND>,
    taskbar_created: u32,
}

thread_local! {
    static APP: OnceCell<App> = const { OnceCell::new() };
}

fn with_app<R>(f: impl FnOnce(&App) -> R) -> Option<R> {
    APP.with(|app| app.get().map(f))
}

pub fn post_refresh(hwnd: isize) {
    if hwnd != 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_REFRESH, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn activate_running_instance() {
    unsafe {
        if let Ok(hwnd) = FindWindowW(TRAY_CLASS, None) {
            let _ = PostMessageW(Some(hwnd), WM_ACTIVATE_INSTANCE, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn run(
    shared: Arc<Shared>,
    events: Sender<Event>,
    show_settings: bool,
    spawn_worker: impl FnOnce() -> io::Result<JoinHandle<()>>,
) -> Result<(), Box<dyn Error>> {
    enable_dark_menus();

    let instance: HINSTANCE = unsafe { GetModuleHandleW(None)? }.into();
    let small_icon = load_icon(instance, SM_CXSMICON, SM_CYSMICON)?;
    let large_icon = load_icon(instance, SM_CXICON, SM_CYICON)?;

    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(tray_proc),
        hInstance: instance,
        lpszClassName: TRAY_CLASS,
        ..Default::default()
    };
    if unsafe { RegisterClassExW(&class) } == 0 {
        return Err(windows::core::Error::from_thread().into());
    }
    window::register_class(instance, large_icon, small_icon)?;

    let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };
    APP.with(|app| {
        app.set(App {
            shared: shared.clone(),
            events,
            worker: RefCell::new(None),
            instance,
            small_icon,
            tray: Cell::new(HWND::default()),
            taskbar_created,
        })
    })
    .map_err(|_| "UI is already running")?;

    // A regular hidden top-level window: message-only windows do not receive
    // the TaskbarCreated broadcast needed to survive an Explorer restart.
    let tray = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            TRAY_CLASS,
            &HSTRING::from(APP_NAME),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )?
    };
    with_app(|app| app.tray.set(tray));
    shared.set_ui_window(tray.0 as isize);
    update_tray_icon(NIM_ADD);

    let worker = spawn_worker()?;
    with_app(|app| *app.worker.borrow_mut() = Some(worker));
    if show_settings {
        window::open();
    }

    let mut message = MSG::default();
    unsafe {
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    shutdown();
    Ok(())
}

fn load_icon(
    instance: HINSTANCE,
    width: SYSTEM_METRICS_INDEX,
    height: SYSTEM_METRICS_INDEX,
) -> windows::core::Result<HICON> {
    unsafe {
        let handle = LoadImageW(
            Some(instance),
            ICON_RESOURCE,
            IMAGE_ICON,
            GetSystemMetrics(width),
            GetSystemMetrics(height),
            LR_DEFAULTCOLOR,
        )?;
        Ok(HICON(handle.0))
    }
}

/// Makes the tray context menu follow the dark scarp.cc look. The export is
/// undocumented, so its absence is silently tolerated.
fn enable_dark_menus() {
    unsafe {
        let Ok(uxtheme) = LoadLibraryW(w!("uxtheme.dll")) else {
            return;
        };
        if let Some(proc) = GetProcAddress(uxtheme, PCSTR(SET_PREFERRED_APP_MODE_ORDINAL as *const u8)) {
            let set_preferred_app_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(proc);
            set_preferred_app_mode(APP_MODE_FORCE_DARK);
        }
    }
}

/// Stops the worker (which restores the original wallpaper) and removes the
/// tray icon. Safe to call more than once.
fn shutdown() {
    window::close();
    let worker = with_app(|app| {
        let _ = app.events.send(Event::Shutdown);
        app.worker.borrow_mut().take()
    })
    .flatten();
    if let Some(worker) = worker {
        let _ = worker.join();
    }
    update_tray_icon(NIM_DELETE);
}

fn update_tray_icon(action: windows::Win32::UI::Shell::NOTIFY_ICON_MESSAGE) {
    let Some((tray, icon, status)) = with_app(|app| (app.tray.get(), app.small_icon, app.shared.status())) else {
        return;
    };
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: tray,
        uID: TRAY_ICON_ID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY,
        hIcon: icon,
        ..Default::default()
    };
    let tooltip = format!("{APP_NAME}\n{}", status.text);
    for (dst, src) in data.szTip.iter_mut().zip(tooltip.encode_utf16().take(TOOLTIP_CHARS)) {
        *dst = src;
    }
    unsafe {
        let _ = Shell_NotifyIconW(action, &data);
    }
}

fn show_tray_menu(hwnd: HWND) {
    let Some(status) = with_app(|app| app.shared.status()) else {
        return;
    };
    unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        // A single '&' would turn the next character into a mnemonic.
        let status = HSTRING::from(status.text.replace('&', "&&"));
        let _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &status);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
        let _ = AppendMenuW(menu, MF_STRING, MENU_SETTINGS as usize, w!("Настройки"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_EXIT as usize, w!("Выход"));
        let _ = SetMenuDefaultItem(menu, MENU_SETTINGS as u32, 0);

        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);
        // Required for the menu to close when the user clicks elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let command =
            TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, cursor.x, cursor.y, None, hwnd, None);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);

        match command.0 {
            MENU_SETTINGS => window::open(),
            MENU_EXIT => PostQuitMessage(0),
            _ => {}
        }
    }
}

/// Saves settings changed in the window and lets the worker re-render.
fn apply_settings(settings: Settings) {
    with_app(|app| {
        let previous = app.shared.settings();
        if previous == settings {
            return;
        }
        if let Err(error) = app.shared.update_settings(settings) {
            app.shared.set_status(Status::error(format!("Не удалось сохранить настройки: {error}")));
        }
        // The theme only restyles the window; the wallpaper stays as it is.
        if (Settings { theme: previous.theme, ..settings }) != previous {
            let _ = app.events.send(Event::SettingsChanged);
        }
    });
}

unsafe extern "system" fn tray_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_TRAY => match lparam.0 as u32 {
            WM_LBUTTONDBLCLK => window::open(),
            WM_RBUTTONUP => show_tray_menu(hwnd),
            _ => {}
        },
        WM_REFRESH => {
            update_tray_icon(NIM_MODIFY);
            window::refresh();
        }
        WM_ACTIVATE_INSTANCE => window::open(),
        // `taskkill` without /F and similar tools ask the app to close.
        WM_CLOSE => unsafe { PostQuitMessage(0) },
        // The session ends right after this returns, so the wallpaper must be
        // restored synchronously.
        WM_ENDSESSION if wparam.0 != 0 => shutdown(),
        _ if with_app(|app| app.taskbar_created) == Some(message) => update_tray_icon(NIM_ADD),
        _ => return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
    LRESULT(0)
}
