// Thin Win32 helpers: taskbar theme, tray icon size, fullscreen/game
// detection and toast app-id registration.

use std::{ffi::OsStr, os::windows::ffi::OsStrExt, path::Path, ptr, time::Duration};

use winapi::{
    shared::{minwindef::DWORD, winerror::ERROR_SUCCESS},
    um::{
        shellapi::{
            SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE,
            QUNS_RUNNING_D3D_FULL_SCREEN,
        },
        sysinfoapi::GetTickCount,
        winnt::REG_SZ,
        winreg::{RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD},
        winuser::{GetLastInputInfo, GetSystemMetrics, LASTINPUTINFO, SM_CXSMICON},
    },
};

fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// True when the Windows taskbar uses the light theme. Defaults to dark,
/// which is the Windows 11 default.
pub fn taskbar_is_light() -> bool {
    let subkey = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let value = wide("SystemUsesLightTheme");
    let mut data: DWORD = 0;
    let mut size = std::mem::size_of::<DWORD>() as DWORD;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            ptr::null_mut(),
            &mut data as *mut DWORD as *mut _,
            &mut size,
        )
    };
    status == ERROR_SUCCESS as i32 && data != 0
}

/// Tray icon size in physical pixels (16 at 100% scaling, 20 at 125%, ...).
pub fn tray_icon_size() -> u32 {
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) };
    size.clamp(16, 64) as u32
}

/// Time since the last keyboard or mouse input in this session.
pub fn idle_time() -> Option<Duration> {
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    if unsafe { GetLastInputInfo(&mut info) } == 0 {
        return None;
    }
    // Both are 32-bit tick counts; wrapping_sub survives the 49-day rollover.
    let now = unsafe { GetTickCount() };
    Some(Duration::from_millis(now.wrapping_sub(info.dwTime) as u64))
}

/// True while a fullscreen game (exclusive or borderless), a fullscreen app
/// or presentation mode is active.
pub fn fullscreen_app_running() -> bool {
    let mut state = 0;
    let hr = unsafe { SHQueryUserNotificationState(&mut state) };
    hr >= 0
        && matches!(
            state,
            QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE
        )
}

/// Registers an AppUserModelID for this unpackaged app so toasts show our
/// name and icon instead of "Windows PowerShell".
pub fn register_app_id(app_id: &str, display_name: &str, icon: &Path) -> Result<(), String> {
    let key = wide(&format!(r"Software\Classes\AppUserModelId\{app_id}"));
    let set = |name: &str, value: &str| -> Result<(), String> {
        let name_w = wide(name);
        let value_w = wide(value);
        let status = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                name_w.as_ptr(),
                REG_SZ,
                value_w.as_ptr() as *const _,
                (value_w.len() * 2) as DWORD,
            )
        };
        if status == ERROR_SUCCESS as i32 {
            Ok(())
        } else {
            Err(format!("RegSetKeyValueW({name}) failed: {status}"))
        }
    };
    set("DisplayName", display_name)?;
    set("IconUri", &icon.to_string_lossy())
}
