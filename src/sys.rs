//! 系统相关的小工具：切到窗口、开机自启、读注册表。

use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_NAME: PCWSTR = w!("Ramulet");

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

/// 切到窗口，最小化了先还原。调用时本进程刚收到鼠标点击，系统允许它把前台交出去
pub fn focus(w: HWND) -> bool {
    unsafe {
        if IsIconic(w).as_bool() {
            let _ = ShowWindow(w, SW_RESTORE);
        }
        SetForegroundWindow(w).as_bool()
    }
}

pub fn exe_path() -> String {
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(None, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

pub fn autostart() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_NAME, RRF_RT_REG_SZ, None, None, None) == ERROR_SUCCESS }
}

pub fn set_autostart(on: bool) {
    unsafe {
        if on {
            let cmd = wide(&format!("\"{}\"", exe_path()));
            let _ = RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_NAME, REG_SZ.0, Some(cmd.as_ptr() as _), (cmd.len() * 2) as u32);
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_NAME);
        }
    }
}

pub fn reg_dword(key: PCWSTR, name: PCWSTR) -> Option<u32> {
    let (mut v, mut n) = (0u32, 4u32);
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, key, name, RRF_RT_REG_DWORD, None, Some(&mut v as *mut _ as _), Some(&mut n)) };
    (r == ERROR_SUCCESS).then_some(v)
}
