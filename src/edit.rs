//! 改别名的小输入框：单独一个线程，普通 GDI 窗口加系统 EDIT 控件。
//! 主线程关了输入法（输入法一加载私有内存 +34 MB），别名要能打中文，所以输入框放在另一个线程里，只有这个线程用输入法。
//! 回车或点到外面保存，Esc 取消；清空（或改回原名）保存 = 删掉别名。结果投回主窗口（WM_APP_ALIAS，lParam 是 Box<Done>）。

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::EM_SETSEL;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

use crate::config::tr;
use crate::theme::*;

const CLASS: PCWSTR = w!("RamuletEdit");
const W: f32 = 300.0;
const H: f32 = 104.0;
const PAD: f32 = 14.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    Repo(String),
    /// (仓库, 分支)
    Branch(String, String),
}

pub struct Done {
    pub key: Key,
    pub text: String,
}

struct State {
    main: isize,
    key: Option<Key>,
    edit: HWND,
    title: String,
    s: f32,
    fonts: [HFONT; 3],
    /// 面板底色、输入区、边框、透明色键、硬阴影
    brushes: [HBRUSH; 5],
    text: COLORREF,
    muted: COLORREF,
    field: COLORREF,
}

fn rgb(v: u32) -> COLORREF {
    COLORREF((v & 0xFF) << 16 | (v & 0xFF00) | (v >> 16 & 0xFF))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

unsafe fn state(hwnd: HWND) -> Option<&'static mut State> {
    unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State).as_mut() }
}

/// 别名里不能有 Tab 和换行（配置文件的分隔符）
pub fn clean(s: &str) -> String {
    s.replace(['\t', '\r', '\n'], " ").trim().to_string()
}

/// 保存或取消，然后关掉输入框
unsafe fn finish(hwnd: HWND, save: bool) {
    unsafe {
        let Some(st) = state(hwnd) else { return };
        let Some(key) = st.key.take() else { return };
        if save {
            let mut buf = vec![0u16; GetWindowTextLengthW(st.edit) as usize + 1];
            let n = GetWindowTextW(st.edit, &mut buf) as usize;
            let done = Box::new(Done { key, text: clean(&String::from_utf16_lossy(&buf[..n])) });
            let _ = PostMessageW(Some(HWND(st.main as _)), crate::WM_APP_ALIAS, WPARAM(0), LPARAM(Box::into_raw(done) as isize));
        }
        let _ = DestroyWindow(hwnd);
    }
}

unsafe extern "system" fn proc_(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            // 整个窗口都是客户区，WS_THICKFRAME 只为了要系统阴影
            WM_NCCALCSIZE if wp.0 != 0 => LRESULT(0),
            WM_NCHITTEST => LRESULT(HTCLIENT as isize),
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
                if let Some(st) = state(hwnd) {
                    let s = st.s;
                    let mut rc = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rc);
                    let px = |v: f32| (v * s) as i32;
                    // 窗口比面板大 SHADOW_POP：先铺透明色键，再画硬阴影、面板、2 px 边框
                    let (pw, ph) = (rc.right - px(SHADOW_POP), rc.bottom - px(SHADOW_POP));
                    FillRect(dc, &rc, st.brushes[3]);
                    let (sh, panel) = (px(SHADOW_POP), RECT { left: 0, top: 0, right: pw, bottom: ph });
                    FillRect(dc, &RECT { left: sh, top: sh, right: pw + sh, bottom: ph + sh }, st.brushes[4]);
                    let frame = |r: &RECT, bw: i32| {
                        for e in [RECT { bottom: r.top + bw, ..*r }, RECT { top: r.bottom - bw, ..*r }, RECT { right: r.left + bw, ..*r }, RECT { left: r.right - bw, ..*r }] {
                            FillRect(dc, &e, st.brushes[2]);
                        }
                    };
                    FillRect(dc, &panel, st.brushes[0]);
                    frame(&panel, px(BORDER));
                    // 输入区：底色一块，2 px 边框
                    let field = RECT { left: px(PAD), top: px(38.0), right: pw - px(PAD), bottom: px(70.0) };
                    FillRect(dc, &field, st.brushes[1]);
                    frame(&field, px(BORDER));
                    SetBkMode(dc, TRANSPARENT);
                    let flags = DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX;
                    for (text, font, color, top, bottom) in [(st.title.as_str(), st.fonts[0], st.text, 10.0, 34.0), (tr("回车保存 · Esc 取消 · 清空恢复原名", "Enter to save · Esc to cancel · clear to reset"), st.fonts[1], st.muted, 74.0, 96.0)] {
                        SelectObject(dc, font.into());
                        SetTextColor(dc, color);
                        let mut r = RECT { left: px(PAD), top: px(top), right: pw - px(PAD), bottom: px(bottom) };
                        DrawTextW(dc, &mut wide(text), &mut r, flags);
                    }
                }
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_CTLCOLOREDIT => match state(hwnd) {
                Some(st) => {
                    let dc = HDC(wp.0 as _);
                    SetTextColor(dc, st.text);
                    SetBkColor(dc, st.field);
                    LRESULT(st.brushes[1].0 as isize)
                }
                None => DefWindowProcW(hwnd, msg, wp, lp),
            },
            // 点到外面：和资源管理器重命名一样，保存
            WM_ACTIVATE if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE => {
                finish(hwnd, true);
                LRESULT(0)
            }
            WM_DESTROY => {
                let p = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) as *mut State;
                if !p.is_null() {
                    let st = Box::from_raw(p);
                    for f in st.fonts {
                        let _ = DeleteObject(f.into());
                    }
                    for b in st.brushes {
                        let _ = DeleteObject(b.into());
                    }
                }
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

/// 在屏幕坐标 at 处弹出输入框，current 是框里预先填的文字（全选）
pub fn open(main: isize, key: Key, title: String, current: String, at: POINT, dark: bool) {
    std::thread::spawn(move || unsafe {
        let wc = WNDCLASSW { lpfnWndProc: Some(proc_), lpszClassName: CLASS, hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(), ..Default::default() };
        RegisterClassW(&wc);
        let mon = MonitorFromPoint(at, MONITOR_DEFAULTTONEAREST);
        let (mut dx, mut dy) = (96, 96);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let s = dx as f32 / 96.0;
        let (w, h) = (((W + SHADOW_POP) * s) as i32, ((H + SHADOW_POP) * s) as i32);
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let wa = mi.rcWork;
        let (x, y) = (at.x.clamp(wa.left, (wa.right - w).max(wa.left)), at.y.clamp(wa.top, (wa.bottom - h).max(wa.top)));
        let Ok(hwnd) = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_LAYERED, CLASS, w!("Ramulet alias"), WS_POPUP, x, y, w, h, None, None, None, None) else { return };
        let d = BOOL::from(dark);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &d as *const _ as _, 4);
        let corner = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corner as *const _ as _, 4);

        // 洋红是透明色键，面板和阴影里不会出现
        const KEY: u32 = 0xFF00FF;
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(colorref(KEY)), 0, LWA_COLORKEY);
        let font = |size: f32, weight: i32| CreateFontW(-(size * s) as i32, 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, 0, w!("Microsoft YaHei UI"));
        // 面板、输入区（用窗口底色，比面板低一档）、边框、字、淡字、硬阴影（色键窗口没有 alpha，取 20% 黑在底色上的近似）
        let [bg, field, border, text, muted, shadow] = if dark { [SURFACE_DARK, BG_DARK, BORDER_DARK, TEXT_DARK, MUTED_DARK, 0x000000] } else { [SURFACE_LIGHT, BG_LIGHT, BORDER_LIGHT, TEXT_LIGHT, MUTED_LIGHT, 0xB9B8B4] };
        let eh = (18.0 * s) as i32;
        let cur = wide(&current).into_iter().chain([0]).collect::<Vec<u16>>();
        let Ok(edit) = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            PCWSTR(cur.as_ptr()),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            ((PAD + 8.0) * s) as i32,
            ((38.0 + 16.0) * s) as i32 - eh / 2,
            ((W - 2.0 * PAD - 16.0) * s) as i32,
            eh,
            Some(hwnd),
            None,
            None,
            None,
        ) else {
            let _ = DestroyWindow(hwnd);
            return;
        };
        let st = Box::new(State {
            main,
            key: Some(key),
            edit,
            title,
            s,
            fonts: [font(13.0, 700), font(12.0, 400), font(13.0, 400)],
            brushes: [CreateSolidBrush(rgb(bg)), CreateSolidBrush(rgb(field)), CreateSolidBrush(rgb(border)), CreateSolidBrush(rgb(KEY)), CreateSolidBrush(rgb(shadow))],
            text: rgb(text),
            muted: rgb(muted),
            field: rgb(field),
        });
        SendMessageW(edit, WM_SETFONT, Some(WPARAM(st.fonts[2].0 as usize)), Some(LPARAM(1)));
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(st) as isize);
        SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
        let _ = ShowWindow(hwnd, SW_SHOW);
        // 主线程刚收到菜单点击，本进程有权把前台给输入框
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(edit));

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            // 单行 EDIT 自己不处理回车和 Esc；输入法正在组字时按回车是 VK_PROCESSKEY，不会误保存
            if msg.message == WM_KEYDOWN && (msg.wParam.0 == VK_RETURN.0 as usize || msg.wParam.0 == VK_ESCAPE.0 as usize) {
                finish(hwnd, msg.wParam.0 == VK_RETURN.0 as usize);
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn clean() {
        assert_eq!(super::clean("  冠军\t主\r\n "), "冠军 主");
    }
}
