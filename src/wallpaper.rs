use std::fs;
use std::path::{Path, PathBuf};

use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemFree};
use windows::Win32::UI::Shell::{DESKTOP_WALLPAPER_POSITION, DWPOS_FILL, DesktopWallpaper, IDesktopWallpaper};
use windows::core::{HSTRING, PCWSTR, PWSTR, Result};

/// Two files are alternated: Windows may skip reloading an unchanged path,
/// and the file it currently shows is never overwritten.
const FILE_NAMES: [&str; 2] = ["wallpaper-a.bmp", "wallpaper-b.bmp"];
const VERBATIM_PREFIX: &str = r"\\?\";
const VERBATIM_UNC_PREFIX: &str = r"\\?\UNC\";

pub struct Wallpaper {
    api: IDesktopWallpaper,
    original_position: DESKTOP_WALLPAPER_POSITION,
    /// (monitor device path, wallpaper path) captured at startup.
    original: Vec<(HSTRING, HSTRING)>,
    dir: PathBuf,
    slot: usize,
    applied: bool,
}

impl Wallpaper {
    pub fn new(dir: PathBuf) -> Result<Self> {
        let api: IDesktopWallpaper = unsafe { CoCreateInstance(&DesktopWallpaper, None, CLSCTX_ALL)? };
        let original_position = unsafe { api.GetPosition()? };

        let real_dir = resolve(&dir);
        let mut original = Vec::new();
        for i in 0..unsafe { api.GetMonitorDevicePathCount()? } {
            let monitor = unsafe { take_string(api.GetMonitorDevicePathAt(i)?) };
            let Ok(path) = (unsafe { api.GetWallpaper(&monitor) }) else {
                continue;
            };
            let path = unsafe { take_string(path) };
            // After a crash the desktop still shows our file; restoring it
            // would only bring back a stale gradient.
            if !is_inside(&path, &dir) && !is_inside(&path, &real_dir) {
                original.push((monitor, path));
            }
        }

        Ok(Self { api, original_position, original, dir, slot: 0, applied: false })
    }

    pub fn next_path(&mut self) -> PathBuf {
        self.slot = (self.slot + 1) % FILE_NAMES.len();
        self.dir.join(FILE_NAMES[self.slot])
    }

    pub fn apply(&mut self, path: &Path) -> Result<()> {
        unsafe {
            self.api.SetPosition(DWPOS_FILL)?;
            self.api.SetWallpaper(PCWSTR::null(), &HSTRING::from(resolve(path).as_path()))?;
        }
        self.applied = true;
        Ok(())
    }

    pub fn restore(&self) -> Result<()> {
        if !self.applied {
            return Ok(());
        }
        let mut result = unsafe { self.api.SetPosition(self.original_position) };
        for (monitor, path) in &self.original {
            if let Err(error) = unsafe { self.api.SetWallpaper(monitor, path) } {
                result = Err(error);
            }
        }
        result
    }
}

/// The wallpaper is loaded by Explorer, not by this process. When started
/// from an MSIX-packaged app (a packaged terminal, launcher or IDE), writes to
/// AppData are silently redirected into the package, so the logical path does
/// not exist for Explorer. The canonical (final) path is the real one.
fn resolve(path: &Path) -> PathBuf {
    let Ok(real) = fs::canonicalize(path) else {
        return path.to_path_buf();
    };
    let real = real.to_string_lossy();
    if let Some(share) = real.strip_prefix(VERBATIM_UNC_PREFIX) {
        PathBuf::from(format!(r"\\{share}"))
    } else {
        PathBuf::from(real.strip_prefix(VERBATIM_PREFIX).unwrap_or(&real))
    }
}

unsafe fn take_string(value: PWSTR) -> HSTRING {
    let text = unsafe { HSTRING::from_wide(value.as_wide()) };
    unsafe { CoTaskMemFree(Some(value.0.cast_const().cast())) };
    text
}

fn is_inside(path: &HSTRING, dir: &Path) -> bool {
    let path = path.to_string().to_lowercase();
    let dir = dir.to_string_lossy().to_lowercase();
    !dir.is_empty() && path.starts_with(&dir)
}
