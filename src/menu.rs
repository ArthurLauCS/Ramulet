//! 右键菜单：自己画的面板，替代系统菜单（系统菜单不跟深色主题，也放不下滑块）。
//! 透明度滑块可以直接拖；开关、分段选项点了面板不关，方便连着调；动作项点了就关。点到面板外面、按 Esc 关闭。
//! 面板底色不透明，不受磁贴透明度影响。键盘：上下选项、左右调滑块和分段、回车或空格确认。

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

use crate::config::{en, tr};
use crate::editors::Editor;
use crate::gfx::Surface;
use crate::theme::*;
use crate::view::{Align, P, inside, rect};
use crate::{App, app, sys};

pub const MENU_CLASS: PCWSTR = w!("RamuletMenu");
const W: f32 = 272.0;
const ROW: f32 = 34.0;
const EDGE: f32 = 6.0;
/// 内容左右边：文字、开关、分段条、分隔线、滑块（连同圆钮）都在 X 和 W - X 之间
const X: f32 = 14.0;
/// 滑块方块的半宽：中心只走到离两边 THUMB 处，拉满也不出内容边
const THUMB: f32 = 5.0;
/// 弹出层右、下两边留给硬阴影的宽度
const SH: f32 = SHADOW_POP;
pub const OPACITY_MIN: u32 = 20;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Id {
    Opacity,
    Top,
    Auto,
    Lang(bool),
    RepoAlias,
    BranchAlias,
    /// 用这个编辑器打开右键点中的已关闭窗口（还没有默认编辑器时，选中的就成为默认）
    OpenWith(Editor),
    /// 点已关闭的磁贴默认用哪个编辑器打开
    Opener(Editor),
    /// 把右键点中的已关闭窗口从列表里删掉
    Forget,
    Exit,
}

enum Row {
    Slider,
    Switch(Id, &'static str, bool),
    Seg(&'static str, Vec<(Id, &'static str, bool)>),
    Act(Id, &'static str, &'static str),
    Sep,
}

pub struct Menu {
    pub hwnd: HWND,
    surf: Option<Surface>,
    hits: Vec<(D2D_RECT_F, Id)>,
    hover: Option<Id>,
    /// 键盘焦点（hits 里的序号），None 表示没用过键盘
    focus: Option<usize>,
    sliding: bool,
    tracking: bool,
    /// 按下时所在的项，松开时还在同一项上才算点击
    pressed: Option<Id>,
}

fn rows(a: &App) -> Vec<Row> {
    let mut v = Vec::new();
    // 右键点在磁贴上：最上面是这块磁贴的操作。Segoe Fluent Icons：E8A7 在新窗口打开，E8BB 关闭（叉号占满 14，和别的图标一样大），E8AC 重命名
    if let Some(t) = &a.target {
        // 已关闭的窗口（hwnd 为 0）：还没选过默认编辑器时（第一次点它）是一排二选一，选过之后是两条“用 … 打开”
        if t.hwnd == 0 {
            let eds = [Editor::Cursor, Editor::Code];
            if a.cfg.opener.is_none() {
                v.push(Row::Seg(tr("用哪个程序打开？之后默认用它", "Open with? It becomes the default"), eds.iter().map(|&e| (Id::OpenWith(e), e.label(), false)).collect()));
            } else {
                v.push(Row::Act(Id::OpenWith(Editor::Cursor), "\u{E8A7}", tr("用 Cursor 打开", "Open with Cursor")));
                v.push(Row::Act(Id::OpenWith(Editor::Code), "\u{E8A7}", tr("用 VS Code 打开", "Open with VS Code")));
            }
            v.push(Row::Act(Id::Forget, "\u{E8BB}", tr("从列表移除", "Remove from list")));
            v.push(Row::Sep);
        }
        v.push(Row::Act(Id::RepoAlias, "\u{E8AC}", tr("设置仓库别名…", "Set repo alias…")));
        if !t.branch.is_empty() {
            v.push(Row::Act(Id::BranchAlias, "\u{E8AC}", tr("设置分支别名…", "Set branch alias…")));
        }
        v.push(Row::Sep);
    }
    v.extend([
        Row::Slider,
        Row::Sep,
        Row::Switch(Id::Top, tr("置顶显示", "Always on top"), a.cfg.topmost),
        Row::Switch(Id::Auto, tr("开机自启", "Start with Windows"), sys::autostart()),
        Row::Seg(tr("重新打开已关闭的窗口用", "Reopen closed windows with"), [Editor::Cursor, Editor::Code].iter().map(|&e| (Id::Opener(e), e.label(), a.cfg.opener == Some(e))).collect()),
        // 语言入口和选项都带英文，切到看不懂的语言后也能找回来
        Row::Seg(tr("语言/Language", "Language"), vec![(Id::Lang(false), "中文/Chinese", !en()), (Id::Lang(true), "English", en())]),
        Row::Sep,
        // Segoe Fluent Icons：E7E8 电源
        Row::Act(Id::Exit, "\u{E7E8}", tr("退出", "Exit")),
    ]);
    v
}

fn row_h(r: &Row) -> f32 {
    match r {
        Row::Slider => 58.0,
        Row::Seg(..) => 58.0,
        Row::Sep => 9.0,
        _ => ROW,
    }
}

fn height(a: &App) -> f32 {
    2.0 * EDGE + rows(a).iter().map(row_h).sum::<f32>()
}

/// 在屏幕坐标 pt 处弹出；靠近屏幕右边或下边时往左、往上翻
pub fn show(a: &mut App, pt: POINT) {
    close(a);
    unsafe {
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let (mut dx, mut dy) = (96, 96);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let s = dx as f32 / 96.0;
        let (w, h) = (((W + SH) * s) as i32, ((height(a) + SH) * s) as i32);
        let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let wa = mi.rcWork;
        let x = if pt.x + w > wa.right { pt.x - w } else { pt.x };
        let y = if pt.y + h > wa.bottom { pt.y - h } else { pt.y };
        let (x, y) = (x.clamp(wa.left, (wa.right - w).max(wa.left)), y.clamp(wa.top, (wa.bottom - h).max(wa.top)));
        // 没有系统阴影：硬阴影自己画在窗口右、下多出的 SH 里
        let Ok(hwnd) = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOREDIRECTIONBITMAP, MENU_CLASS, w!("Ramulet menu"), WS_POPUP, x, y, w, h, None, None, None, None) else { return };
        let dark = BOOL::from(a.theme.dark);
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as _, 4);
        let corner = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corner as *const _ as _, 4);
        a.menu = Some(Box::new(Menu { hwnd, surf: None, hits: Vec::new(), hover: None, focus: None, sliding: false, tracking: false, pressed: None }));
        let _ = ShowWindow(hwnd, SW_SHOW);
        // 磁贴刚收到点击，有权把前台给菜单；菜单要在前台才能在点到外面时收到失活消息
        let _ = SetForegroundWindow(hwnd);
    }
}

pub fn close(a: &mut App) {
    if let Some(m) = a.menu.take() {
        unsafe {
            let _ = DestroyWindow(m.hwnd);
        }
    }
}

fn draw(a: &App, m: &mut Menu) {
    unsafe {
        let mut rc = RECT::default();
        let _ = GetClientRect(m.hwnd, &mut rc);
        if m.surf.is_none() {
            m.surf = a.gfx.surface(m.hwnd, rc.right as u32, rc.bottom as u32).ok();
        }
        let Some(surf) = m.surf.clone() else { return };
        let dpi = GetDpiForWindow(m.hwnd) as f32;
        if a.gfx.bind(&surf, dpi).is_err() {
            return;
        }
        // 跟磁贴实际用的深浅色走（透明度低于 50% 时磁贴强制深色）
        let th = Theme::panel(a.theme.dark);
        let f = &a.fonts;
        let p = P { dc: &a.gfx.dc, br: &a.gfx.brush, dw: &a.dw, shadow: false };
        // 窗口比面板大 SH：右、下两边给硬阴影
        let w = rc.right as f32 * 96.0 / dpi - SH;
        let h = rc.bottom as f32 * 96.0 / dpi - SH;
        let mut hits = Vec::new();
        let focused = m.focus.and_then(|i| m.hits.get(i)).map(|x| x.1);
        let hot = |id: Id| m.hover == Some(id) || focused == Some(id);
        a.gfx.dc.BeginDraw();
        a.gfx.dc.Clear(Some(&rgba(0.0, 0.0, 0.0, 0.0)));
        p.block(rect(0.0, 0.0, w, h), th.surface, th.border, th.shadow, SH);
        let mut y = EDGE;
        for row in rows(a) {
            let rh = row_h(&row);
            match row {
                Row::Slider => {
                    let v = a.cfg.opacity;
                    p.text(tr("磁贴透明度", "Tile opacity"), rect(X, y + 4.0, w - X, y + 26.0), &f.body13, th.text, Align::L);
                    p.text(&format!("{v}%"), rect(X, y + 4.0, w - X, y + 26.0), &f.mono12, th.text, Align::R);
                    let (l, r, cy) = (X + THUMB, w - X - THUMB, y + 42.0);
                    let x = l + (r - l) * (v - OPACITY_MIN) as f32 / (100 - OPACITY_MIN) as f32;
                    // 轨道：2 px 边框的细槽，已走过的部分是强调色；滑块是 2 px 边框的方块
                    p.block(rect(X, cy - 4.0, w - X, cy + 4.0), th.sunk, th.border, th.shadow, 0.0);
                    p.fill(rect(X + BORDER, cy - 2.0, x, cy + 2.0), th.accent);
                    let on = m.sliding || hot(Id::Opacity);
                    p.block(rect(x - THUMB, cy - 10.0, x + THUMB, cy + 10.0), th.accent, th.border, th.shadow, if on { 2.0 } else { 0.0 });
                    hits.push((rect(X - 4.0, y + 28.0, w - X + 4.0, y + 56.0), Id::Opacity));
                }
                Row::Switch(id, label, on) => {
                    let r = rect(BORDER, y + 1.0, w - BORDER, y + rh - 1.0);
                    if hot(id) {
                        p.fill(r, th.sunk);
                    }
                    p.text(label, rect(X, y, w - 64.0, y + rh), &f.body13, th.text, Align::L);
                    // 复选框：18 见方，选中是强调色填充加对勾
                    let cy = y + rh / 2.0;
                    let bx = rect(w - X - 18.0, cy - 9.0, w - X, cy + 9.0);
                    p.block(bx, if on { th.accent } else { th.surface }, th.border, th.shadow, 0.0);
                    if on {
                        p.tick(bx.left + 4.0, cy - 0.5, th.ink);
                    }
                    hits.push((r, id));
                }
                Row::Seg(label, opts) => {
                    p.text(label, rect(X, y + 2.0, w - X, y + 22.0), &f.body12, th.muted, Align::L);
                    let bar = rect(X, y + 24.0, w - X, y + 52.0);
                    p.block(bar, th.surface, th.border, th.shadow, 0.0);
                    let n = opts.len();
                    let sw = (bar.right - bar.left - 2.0 * BORDER) / n as f32;
                    for (k, (id, text, on)) in opts.into_iter().enumerate() {
                        let b = rect(bar.left + BORDER + k as f32 * sw, bar.top + BORDER, bar.left + BORDER + (k + 1) as f32 * sw, bar.bottom - BORDER);
                        if on {
                            p.fill(b, th.accent);
                        } else if hot(id) {
                            p.fill(b, th.sunk);
                        }
                        if k > 0 {
                            p.fill(rect(b.left - BORDER / 2.0, b.top, b.left + BORDER / 2.0, b.bottom), th.border);
                        }
                        p.text(text, b, if on { &f.body12b } else { &f.body12 }, if on { th.ink } else { th.text }, Align::C);
                        hits.push((b, id));
                    }
                }
                Row::Act(id, glyph, label) => {
                    let r = rect(BORDER, y + 1.0, w - BORDER, y + rh - 1.0);
                    let danger = id == Id::Exit && hot(id);
                    if danger {
                        p.fill(r, th.red);
                    } else if hot(id) {
                        p.fill(r, th.sunk);
                    }
                    let c = if danger { th.ink } else { th.text };
                    // 图标笔画左边和开关行的文字左边对齐
                    p.ink(glyph, rect(X, y, X + 20.0, y + rh), &f.icon, if danger { th.ink } else { th.muted }, Align::L);
                    p.text(label, rect(X + 30.0, y, w - X, y + rh), &f.body13, c, Align::L);
                    hits.push((r, id));
                }
                Row::Sep => p.fill(rect(X, y + 4.0, w - X, y + 5.0), th.rule),
            }
            y += rh;
        }
        let _ = a.gfx.dc.EndDraw(None, None);
        let _ = a.gfx.present(&surf);
        m.hits = hits;
    }
}

fn hit(m: &Menu, lp: LPARAM) -> Option<Id> {
    let s = unsafe { GetDpiForWindow(m.hwnd) } as f32 / 96.0;
    let (x, y) = ((lp.0 as i16) as f32 / s, ((lp.0 >> 16) as i16) as f32 / s);
    m.hits.iter().find(|(r, _)| inside(r, x, y)).map(|(_, id)| *id)
}

/// 滑块按鼠标横坐标取值，5% 一档
fn slide(a: &mut App, m: &Menu, lp: LPARAM) {
    let s = unsafe { GetDpiForWindow(m.hwnd) } as f32 / 96.0;
    let x = (lp.0 as i16) as f32 / s;
    let (l, r) = (X + THUMB, W - X - THUMB);
    let t = ((x - l) / (r - l)).clamp(0.0, 1.0);
    let v = OPACITY_MIN + ((t * (100 - OPACITY_MIN) as f32 / 5.0).round() as u32) * 5;
    if v != a.cfg.opacity {
        a.set_opacity(v);
    }
}

pub unsafe extern "system" fn menu_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_NCCALCSIZE if wp.0 != 0 => return LRESULT(0),
            WM_NCHITTEST => return LRESULT(HTCLIENT as isize),
            WM_ERASEBKGND => return LRESULT(1),
            _ => {}
        }
        let a = app();
        // 菜单先从 App 里拿出来，处理完再放回去：动作可能要改 App 的其他部分，甚至关掉菜单
        let Some(mut m) = a.menu.take_if(|m| m.hwnd == hwnd) else {
            return DefWindowProcW(hwnd, msg, wp, lp);
        };
        let mut keep = true;
        let mut redraw = false;
        match msg {
            WM_PAINT => {
                let _ = ValidateRect(Some(hwnd), None);
                draw(a, &mut m);
            }
            WM_ACTIVATE if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE => keep = false,
            WM_MOUSEMOVE => {
                if !m.tracking {
                    let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    let _ = TrackMouseEvent(&mut tme);
                    m.tracking = true;
                }
                if m.sliding {
                    slide(a, &m, lp);
                }
                let h = hit(&m, lp);
                redraw = h != m.hover || m.sliding || m.focus.is_some();
                m.hover = h;
                m.focus = None;
            }
            WM_MOUSELEAVE => {
                m.tracking = false;
                redraw = m.hover.take().is_some();
            }
            WM_LBUTTONDOWN => {
                m.pressed = hit(&m, lp);
                if m.pressed == Some(Id::Opacity) {
                    m.sliding = true;
                    SetCapture(hwnd);
                    slide(a, &m, lp);
                }
                redraw = true;
            }
            WM_LBUTTONUP => {
                if std::mem::take(&mut m.sliding) {
                    let _ = ReleaseCapture();
                    a.save_cfg();
                } else if let Some(id) = hit(&m, lp).filter(|id| m.pressed == Some(*id)) {
                    keep = !a.menu_action(id);
                }
                m.pressed = None;
                redraw = true;
            }
            WM_KEYDOWN => {
                let n = m.hits.len();
                let cur = m.focus.or_else(|| m.hover.and_then(|h| m.hits.iter().position(|x| x.1 == h)));
                match VIRTUAL_KEY(wp.0 as u16) {
                    VK_ESCAPE => keep = false,
                    VK_DOWN | VK_TAB if n > 0 => m.focus = Some(cur.map_or(0, |i| (i + 1) % n)),
                    VK_UP if n > 0 => m.focus = Some(cur.map_or(n - 1, |i| (i + n - 1) % n)),
                    k @ (VK_LEFT | VK_RIGHT) => match cur.map(|i| m.hits[i].1) {
                        Some(Id::Opacity) => {
                            let v = if k == VK_LEFT { a.cfg.opacity.saturating_sub(5).max(OPACITY_MIN) } else { (a.cfg.opacity + 5).min(100) };
                            a.set_opacity(v);
                            a.save_cfg();
                            m.focus = cur;
                        }
                        // 分段选项里左右移动就是换选项
                        Some(Id::Lang(_) | Id::Opener(_)) => {
                            let i = cur.unwrap_or(0);
                            let j = if k == VK_LEFT { i.saturating_sub(1) } else { (i + 1).min(n - 1) };
                            if std::mem::discriminant(&m.hits[i].1) == std::mem::discriminant(&m.hits[j].1) {
                                keep = !a.menu_action(m.hits[j].1);
                                m.focus = Some(j);
                            }
                        }
                        _ => {}
                    },
                    VK_RETURN | VK_SPACE => {
                        if let Some(i) = cur {
                            keep = !a.menu_action(m.hits[i].1);
                            m.focus = Some(i);
                        }
                    }
                    _ => {}
                }
                redraw = true;
            }
            WM_DPICHANGED => {
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                m.surf = None;
            }
            _ => {
                a.menu = Some(m);
                return DefWindowProcW(hwnd, msg, wp, lp);
            }
        }
        if keep {
            // 动作可能改了行数（如“隐藏磁贴”变“显示磁贴”）不会改高度；语言切换会改文字，整块重画
            if redraw {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            if a.menu.is_none() {
                a.menu = Some(m);
            }
        } else {
            let _ = DestroyWindow(m.hwnd);
        }
        LRESULT(0)
    }
}
