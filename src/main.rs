#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod accent;
mod autostart;
mod media;
mod render;
mod settings;
mod ui;
mod wallpaper;
mod worker;

use std::env;
use std::error::Error;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::thread;

use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows::Win32::System::ProcessStatus::K32EmptyWorkingSet;
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, MB_ICONERROR, MB_OK, MessageBoxW, SM_CXSCREEN, SM_CYSCREEN,
};
use windows::core::{HSTRING, PCWSTR, w};

use settings::Settings;

pub const APP_NAME: &str = "SCARP WALLPAPER";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

const APP_DIR: &str = "ScarpWallpaper";
const SETTINGS_FILE: &str = "settings.ini";
const INSTANCE_MUTEX: PCWSTR = w!(r"Local\ScarpWallpaper.Instance");

#[derive(Clone, Copy, Debug)]
pub enum Event {
    MediaChanged,
    SessionsChanged,
    SettingsChanged,
    Shutdown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusKind {
    Idle,
    Playing,
    Custom,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub kind: StatusKind,
    pub text: String,
}

impl Status {
    pub fn idle(text: impl Into<String>) -> Self {
        Self { kind: StatusKind::Idle, text: text.into() }
    }

    pub fn playing(text: impl Into<String>) -> Self {
        Self { kind: StatusKind::Playing, text: text.into() }
    }

    pub fn custom(text: impl Into<String>) -> Self {
        Self { kind: StatusKind::Custom, text: text.into() }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self { kind: StatusKind::Error, text: text.into() }
    }
}

/// State shared between the UI thread and the worker.
pub struct Shared {
    settings: Mutex<Settings>,
    status: Mutex<Status>,
    cover: Mutex<Option<Arc<[u8]>>>,
    ui_window: AtomicIsize,
    settings_path: PathBuf,
}

impl Shared {
    fn new(settings_path: PathBuf) -> Self {
        Self {
            settings: Mutex::new(Settings::load(&settings_path)),
            status: Mutex::new(Status::idle("Запуск")),
            cover: Mutex::new(None),
            ui_window: AtomicIsize::new(0),
            settings_path,
        }
    }

    pub fn settings(&self) -> Settings {
        *lock(&self.settings)
    }

    pub fn update_settings(&self, settings: Settings) -> io::Result<()> {
        *lock(&self.settings) = settings;
        settings.save(&self.settings_path)
    }

    pub fn status(&self) -> Status {
        lock(&self.status).clone()
    }

    pub fn set_status(&self, status: Status) {
        let changed = {
            let mut current = lock(&self.status);
            let changed = *current != status;
            *current = status;
            changed
        };
        if changed {
            ui::post_refresh(self.ui_window.load(Ordering::Acquire));
        }
    }

    pub fn cover(&self) -> Option<Arc<[u8]>> {
        lock(&self.cover).clone()
    }

    pub fn set_cover(&self, cover: Option<Arc<[u8]>>) {
        *lock(&self.cover) = cover;
        ui::post_refresh(self.ui_window.load(Ordering::Acquire));
    }

    fn set_ui_window(&self, hwnd: isize) {
        self.ui_window.store(hwnd, Ordering::Release);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Primary display size in physical pixels (the process is per-monitor DPI aware).
pub fn screen_size() -> (u32, u32) {
    let (width, height) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    (width.max(1) as u32, height.max(1) as u32)
}

/// Returns idle pages to the OS; the app sleeps between track changes.
pub fn trim_memory() {
    unsafe {
        let _ = K32EmptyWorkingSet(GetCurrentProcess());
    }
}

fn app_dir(env_var: &str) -> PathBuf {
    env::var_os(env_var).map_or_else(env::temp_dir, PathBuf::from).join(APP_DIR)
}

fn main() {
    if let Err(error) = run() {
        let text = HSTRING::from(error.to_string());
        unsafe {
            MessageBoxW(None, &text, &HSTRING::from(APP_NAME), MB_OK | MB_ICONERROR);
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    // The handle lives until the process exits and marks the running instance.
    let _instance = unsafe { CreateMutexW(None, true, INSTANCE_MUTEX)? };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        ui::activate_running_instance();
        return Ok(());
    }

    let background = env::args().any(|arg| arg == autostart::BACKGROUND_FLAG);
    // Keeps the Run entry valid after the executable was moved.
    if autostart::is_enabled() {
        let _ = autostart::set_enabled(true);
    }

    let shared = Arc::new(Shared::new(app_dir("APPDATA").join(SETTINGS_FILE)));
    let (sender, receiver) = mpsc::channel();
    let worker_shared = shared.clone();
    let worker_notify = sender.clone();
    let wallpaper_dir = app_dir("LOCALAPPDATA");

    ui::run(shared, sender, !background, move || {
        thread::Builder::new()
            .name("worker".into())
            .spawn(move || worker::run(worker_shared, receiver, worker_notify, wallpaper_dir))
    })
}
