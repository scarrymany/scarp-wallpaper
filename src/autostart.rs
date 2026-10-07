use std::env;
use std::error::Error;

use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{PCWSTR, w};

/// Passed by the Run entry so a login start stays in the tray.
pub const BACKGROUND_FLAG: &str = "--background";

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE_NAME: PCWSTR = w!("ScarpWallpaper");

pub fn is_enabled() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE_NAME, RRF_RT_REG_SZ, None, None, None).is_ok() }
}

pub fn set_enabled(enabled: bool) -> Result<(), Box<dyn Error>> {
    if !enabled {
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE_NAME).ok()? };
        return Ok(());
    }

    let exe = env::current_exe()?;
    let command: Vec<u16> = format!("\"{}\" {BACKGROUND_FLAG}", exe.display()).encode_utf16().chain([0]).collect();
    let bytes = u32::try_from(command.len() * size_of::<u16>())?;
    unsafe {
        RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE_NAME, REG_SZ.0, Some(command.as_ptr().cast()), bytes).ok()?;
    }
    Ok(())
}
