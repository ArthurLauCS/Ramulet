#![windows_subsystem = "windows"]
//! Cursor / VS Code 窗口磁贴：桌面上一块小窗，每个打开的编辑器窗口一块磁贴（仓库名 + 分支名），点一下切过去。
//! 关掉的窗口置灰排在下面，点一下用编辑器重新打开那个文件夹。
//! 平时贴在桌面这一层，不挡其他窗口；点托盘图标到最前面，切走后回到桌面层；按下图钉一直置顶。不占任务栏按钮。
//!
//! 参数：--data <目录> 代替 %APPDATA%\Ramulet；--dump <文件> 不开窗口，把扫到的窗口和耗时写成文本；
//! --top 置顶；--light / --dark 强制主题；--lang zh|en 强制语言；--demo 显示一组虚构的仓库，不扫描（后几个是截图走查用的）。

mod config;
mod edit;
mod editors;
mod gfx;
mod json;
mod menu;
mod sys;
mod view;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicIsize, Ordering::Relaxed};

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

use config::Config;
use editors::{Editor, Scanner, Win};
use sys::wide;
use view::{Fonts, Hit, Theme};

const CLASS: PCWSTR = w!("RamuletMain");
/// 系统窗口事件（前台切换、窗口显示 / 隐藏 / 销毁 / 改标题），wParam 是事件号，lParam 是窗口
const WM_APP_EVENT: u32 = WM_APP + 1;
/// 别名输入框保存了，lParam 是 Box<edit::Done>
pub const WM_APP_ALIAS: u32 = WM_APP + 2;
const WM_APP_TRAY: u32 = WM_APP + 3;
const T_SCAN: usize = 1;
const T_POLL: usize = 2;
/// 窗口事件来了多久后重扫（一串事件只扫一次）
const SCAN_DELAY: u32 = 60;
/// 兜底轮询：在终端里切分支不会有窗口事件，靠它跟上
const POLL_MS: u32 = 2000;
const EDGE: f32 = 6.0;
const DEFAULT_W: f32 = 440.0;

pub struct App {
    data: PathBuf,
    pub cfg: Config,
    hwnd: HWND,
    d2d: ID2D1Factory1,
    pub gfx: gfx::Gfx,
    pub dw: IDWriteFactory,
    pub fonts: Fonts,
    pub theme: Theme,
    force_dark: Option<bool>,
    surf: Option<gfx::Surface>,
    scanner: Scanner,
    pub wins: Vec<Win>,
    /// 已关闭的窗口（hwnd 为 0），按 editors::sort 排，画在打开的下面
    pub closed: Vec<Win>,
    tray_icon: HICON,
    taskbar_created: u32,
    scan_pending: bool,
    /// 系统主题是不是深色（菜单跟它走；磁贴在低透明度时会强制深色）
    pub sys_dark: bool,
    pub menu: Option<Box<menu::Menu>>,
    pub hits: Vec<(D2D_RECT_F, Hit)>,
    pub hover: Option<Hit>,
    tracking: bool,
    /// 按下左键：屏幕坐标、窗口位置、是否拖过阈值
    press: Option<(POINT, RECT, bool)>,
    /// 不置顶时被任务栏图标叫到了最前面（切到别的程序就回桌面层）
    front: bool,
    /// 右键点在哪块磁贴上（菜单里的“设置别名”改它）、右键时鼠标的屏幕坐标
    pub target: Option<Win>,
    menu_pt: POINT,
    /// --demo：显示固定的虚构数据，不扫描
    demo: bool,
}

static mut APP: *mut App = std::ptr::null_mut();
/// 窗口事件回调里往主窗口投消息用
static MAIN: AtomicIsize = AtomicIsize::new(0);

/// 单线程 UI，全局状态只在主线程上访问
pub fn app() -> &'static mut App {
    unsafe { &mut *APP }
}

fn arg(args: &[String], k: &str) -> Option<PathBuf> {
    args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).map(PathBuf::from)
}

fn scale(hwnd: HWND) -> f32 {
    unsafe { GetDpiForWindow(hwnd) as f32 / 96.0 }
}

fn lparam_pt(lp: LPARAM) -> POINT {
    POINT { x: lp.0 as i16 as i32, y: (lp.0 >> 16) as i16 as i32 }
}

fn work_area(mon: HMONITOR) -> RECT {
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let _ = GetMonitorInfoW(mon, &mut mi);
    }
    mi.rcWork
}

/// 矩形整个挪进工作区，返回新的左上角；比工作区还大时贴左上
fn fit(r: &RECT, wa: RECT) -> (i32, i32) {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    (r.left.min(wa.right - w).max(wa.left), r.top.min(wa.bottom - h).max(wa.top))
}

/// 桌面窗口：在桌面层时磁贴以它为所有者，始终在桌面之上、其他窗口之下
fn desktop_owner() -> isize {
    unsafe { FindWindowW(w!("Progman"), PCWSTR::null()).map_or(0, |h| h.0 as isize) }
}

fn copy_wide(dst: &mut [u16], s: &str) {
    let n = dst.len() - 1;
    for (d, c) in dst.iter_mut().take(n).zip(s.encode_utf16().chain([0])) {
        *d = c;
    }
}

fn keep_on_screen(hwnd: HWND) {
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(hwnd, &mut r);
        let (x, y) = fit(&r, work_area(MonitorFromRect(&r, MONITOR_DEFAULTTONEAREST)));
        if (x, y) != (r.left, r.top) {
            let _ = SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }
}

impl App {
    fn save_cfg(&self) {
        let _ = std::fs::create_dir_all(&self.data);
        let _ = std::fs::write(self.data.join("config.txt"), self.cfg.to_text());
    }

    fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn load_theme(&mut self) {
        self.sys_dark = self.force_dark.unwrap_or_else(|| sys::reg_dword(w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"), w!("AppsUseLightTheme")) != Some(1));
        // 透明度低于 50% 时底色太薄，深色字在深色壁纸上看不清：不管系统主题，都用桌面图标那样的白字加阴影；同时关掉模糊，透出壁纸
        let o = self.cfg.opacity.clamp(menu::OPACITY_MIN, 100) as f32 / 100.0;
        let dark = self.sys_dark || o < 0.5;
        self.theme = Theme::new(dark, o);
        unsafe {
            let d = BOOL::from(dark);
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &d as *const _ as _, 4);
            let corner = DWMWCP_ROUND;
            let _ = DwmSetWindowAttribute(self.hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corner as *const _ as _, 4);
            // 系统亚克力（DWMSBT_TRANSIENTWINDOW）在窗口未激活时退成纯色，磁贴几乎总是未激活，
            // 所以用 SetWindowCompositionAttribute 的亚克力模糊：未公开，但从 Win10 起一直可用（和 ClaudeTile 一样）
            #[repr(C)]
            struct Accent {
                state: u32,
                flags: u32,
                tint: u32,
                anim: u32,
            }
            #[repr(C)]
            struct Data {
                attr: u32,
                data: *mut Accent,
                size: usize,
            }
            type Swca = unsafe extern "system" fn(HWND, *mut Data) -> BOOL;
            if let Some(f) = GetProcAddress(GetModuleHandleW(w!("user32.dll")).unwrap_or_default(), s!("SetWindowCompositionAttribute")) {
                let f: Swca = std::mem::transmute(f);
                // 4 = ACCENT_ENABLE_ACRYLICBLURBEHIND；染色交给 D2D 画的底色
                let mut accent = Accent { state: if o >= 0.5 { 4 } else { 0 }, flags: 0, tint: 0x01000000, anim: 0 };
                let mut d = Data { attr: 19, data: &mut accent, size: size_of::<Accent>() };
                let _ = f(self.hwnd, &mut d);
            }
        }
        self.invalidate();
    }

    fn paint(&mut self) {
        unsafe {
            let _ = ValidateRect(Some(self.hwnd), None);
            let mut rc = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rc);
            if self.surf.is_none() {
                self.surf = self.gfx.surface(self.hwnd, rc.right as u32, rc.bottom as u32).ok();
            }
            let Some(surf) = self.surf.clone() else { return };
            let s = scale(self.hwnd);
            if self.gfx.bind(&surf, s * 96.0).is_err() {
                return self.reset_gfx();
            }
            self.gfx.dc.BeginDraw();
            let fr = view::draw(self, rc.right as f32 / s, rc.bottom as f32 / s);
            if self.gfx.dc.EndDraw(None, None).is_err() {
                return self.reset_gfx();
            }
            let _ = self.gfx.present(&surf);
            self.hits = fr.hits;
            // 高度跟着磁贴数走。顶边以用户摆的位置为准，放不下才临时往上挪，变矮后回到原处；拖动窗口途中不动位置
            let want = (fr.h * s).ceil() as i32;
            if want != rc.bottom {
                let mut wr = RECT::default();
                let _ = GetWindowRect(self.hwnd, &mut wr);
                let wa = work_area(MonitorFromRect(&wr, MONITOR_DEFAULTTONEAREST));
                let (x, y) = fit(&RECT { left: wr.left, top: self.cfg.pos[1], right: wr.right, bottom: self.cfg.pos[1] + want }, wa);
                let flags = if self.press.is_some() { SWP_NOMOVE } else { SET_WINDOW_POS_FLAGS(0) };
                let _ = SetWindowPos(self.hwnd, None, x, y, rc.right, want, flags | SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
    }

    /// 设备丢失（驱动更新、显卡重置）后重建
    fn reset_gfx(&mut self) {
        if let Ok(g) = gfx::Gfx::new(&self.d2d) {
            self.gfx = g;
            self.surf = None;
            self.invalidate();
        }
    }

    fn hit_at(&self, pt: POINT) -> Option<Hit> {
        let s = scale(self.hwnd);
        let (x, y) = (pt.x as f32 / s, pt.y as f32 / s);
        self.hits.iter().find(|(r, _)| view::inside(r, x, y)).map(|(_, h)| *h)
    }

    fn click(&mut self, h: Hit) {
        match h {
            Hit::Win(w) => {
                if !sys::focus(HWND(w as _)) {
                    unsafe { MessageBeep(MB_ICONWARNING).ok() };
                }
            }
            // 已关闭的：选过默认编辑器就直接打开，没选过弹菜单让用户选（菜单最上面是“用 Cursor / VS Code 打开”）
            Hit::Closed(i) => {
                if let Some(t) = self.closed.get(i).cloned() {
                    match self.cfg.opener {
                        Some(e) => self.reopen(&t, e),
                        None => self.menu_at_cursor(Some(t)),
                    }
                }
            }
            Hit::Pin => self.set_topmost(!self.cfg.topmost),
        }
        self.invalidate();
    }

    /// 用编辑器打开已关闭窗口的文件夹；编辑器已在运行时它会转给运行中的那个，开一个新窗口
    fn reopen(&mut self, t: &Win, e: Editor) {
        let exe = self.cfg.exes.get(&e).cloned().into_iter().chain(e.default_exes()).find(|p| p.exists());
        let ok = match (exe, &t.path) {
            (Some(exe), Some(p)) => unsafe {
                // 新窗口由编辑器进程打开，让它能拿到前台
                let _ = AllowSetForegroundWindow(ASFW_ANY);
                std::process::Command::new(exe).arg(p).spawn().is_ok()
            },
            _ => false,
        };
        if !ok {
            unsafe { MessageBeep(MB_ICONWARNING).ok() };
        }
    }

    /// 在鼠标处弹出右键菜单；target 是点中的磁贴（菜单最上面是它的操作）
    fn menu_at_cursor(&mut self, target: Option<Win>) {
        self.target = target;
        unsafe {
            let _ = GetCursorPos(&mut self.menu_pt);
        }
        menu::show(self, self.menu_pt);
    }

    fn rescan(&mut self) {
        if self.demo {
            return;
        }
        let wins = self.scanner.scan();
        let mut dirty = false;
        for (e, p) in &self.scanner.exes {
            if self.cfg.exes.get(e) != Some(p) {
                self.cfg.exes.insert(*e, p.clone());
                dirty = true;
            }
        }
        if editors::track_closed(&mut self.cfg.closed, &self.wins, &wins, config::MAX_CLOSED) || dirty {
            self.save_cfg();
        }
        let closed = self.closed_wins();
        if wins != self.wins || closed != self.closed {
            self.wins = wins;
            self.closed = closed;
            self.invalidate();
        }
    }

    fn closed_wins(&mut self) -> Vec<Win> {
        let mut v: Vec<Win> = self
            .cfg
            .closed
            .clone()
            .into_iter()
            .map(|(editor, p)| {
                let name = p.file_name().map_or_else(|| p.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned());
                let (repo, branch, worktree) = self.scanner.describe(Some(&p), &name);
                Win { hwnd: 0, editor, repo, branch, worktree, active: false, path: Some(p) }
            })
            .collect();
        editors::sort(&mut v);
        v
    }

    fn tray(&self, msg: NOTIFY_ICON_MESSAGE) {
        let mut nid = NOTIFYICONDATAW { cbSize: size_of::<NOTIFYICONDATAW>() as u32, hWnd: self.hwnd, uID: 1, uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP, uCallbackMessage: WM_APP_TRAY, hIcon: self.tray_icon, ..Default::default() };
        copy_wide(&mut nid.szTip, "Ramulet");
        unsafe {
            let _ = Shell_NotifyIconW(msg, &nid);
        }
    }

    /// 点了托盘图标：不管置不置顶都到最前面（激活后 WM_ACTIVATE 里离开桌面层）
    fn show_front(&mut self) {
        menu::close(self);
        unsafe {
            let _ = SetForegroundWindow(self.hwnd);
        }
    }

    /// 窗口事件：只在和编辑器窗口有关时重扫；别的程序到了前台，被叫到最前面的磁贴回桌面层
    fn on_event(&mut self, ev: u32, w: HWND) {
        let relevant = match ev {
            EVENT_SYSTEM_FOREGROUND => {
                // 事件是异步投过来的：处理时前台可能已经回到磁贴或菜单（点任务栏图标时先激活任务栏，再激活磁贴）
                let fg = unsafe { GetForegroundWindow() };
                if fg != self.hwnd && self.menu.as_ref().is_none_or(|m| m.hwnd != fg) {
                    self.raise(false);
                }
                true
            }
            EVENT_OBJECT_DESTROY | EVENT_OBJECT_HIDE => self.wins.iter().any(|x| x.hwnd == w.0 as isize),
            _ => editors::class_name(w) == editors::ELECTRON_CLASS,
        };
        if relevant && !self.scan_pending {
            self.scan_pending = true;
            unsafe { SetTimer(Some(self.hwnd), T_SCAN, SCAN_DELAY, None) };
        }
    }

    /// 不置顶时：on = 到最前面（点了任务栏图标），否则回到桌面层
    fn raise(&mut self, on: bool) {
        if self.cfg.topmost || self.front == on {
            return;
        }
        self.front = on;
        // 以桌面（Progman）为所有者的窗口压不到其他窗口上面：到前面时先去掉所有者，回去时再挂上
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_HWNDPARENT, if on { 0 } else { desktop_owner() });
            let _ = SetWindowPos(self.hwnd, Some(if on { HWND_TOP } else { HWND_BOTTOM }), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }

    fn set_topmost(&mut self, on: bool) {
        self.cfg.topmost = on;
        self.front = false;
        self.save_cfg();
        // 以桌面为所有者的窗口设不成置顶：置顶时先去掉所有者，取消置顶时再挂回桌面、回到桌面这一层
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_HWNDPARENT, if on { 0 } else { desktop_owner() });
            let _ = SetWindowPos(self.hwnd, Some(if on { HWND_TOPMOST } else { HWND_NOTOPMOST }), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            if !on {
                let _ = SetWindowPos(self.hwnd, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }

    fn save_pos(&mut self) {
        let mut r = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.hwnd, &mut r);
        }
        self.cfg.pos = [r.left, r.top, r.right - r.left];
        self.save_cfg();
    }

    /// 菜单里点了某项；返回 true 表示菜单要关
    pub fn menu_action(&mut self, id: menu::Id) -> bool {
        use menu::Id;
        match id {
            Id::Opacity => return false,
            Id::Top => self.set_topmost(!self.cfg.topmost),
            Id::Auto => sys::set_autostart(!sys::autostart()),
            Id::Lang(en) => {
                self.cfg.en = Some(en);
                config::set_lang(self.cfg.en);
                self.save_cfg();
            }
            Id::RepoAlias | Id::BranchAlias => {
                let Some(t) = self.target.clone() else { return true };
                let (key, label, orig, cur) = if id == Id::RepoAlias {
                    (edit::Key::Repo(t.repo.clone()), config::tr("仓库别名", "Repo alias"), t.repo.clone(), self.cfg.repo_alias.get(&t.repo))
                } else {
                    let k = (t.repo.clone(), t.branch.clone());
                    let cur = self.cfg.branch_alias.get(&k);
                    (edit::Key::Branch(k.0, k.1), config::tr("分支别名", "Branch alias"), t.branch.clone(), cur)
                };
                let cur = cur.cloned().unwrap_or_else(|| orig.clone());
                edit::open(self.hwnd.0 as isize, key, format!("{label} · {orig}"), cur, self.menu_pt, self.sys_dark);
                return true;
            }
            // 第一次选的编辑器记成默认，之后点已关闭的磁贴直接用它
            Id::OpenWith(e) => {
                if self.cfg.opener.is_none() {
                    self.cfg.opener = Some(e);
                    self.save_cfg();
                }
                if let Some(t) = self.target.clone() {
                    self.reopen(&t, e);
                }
                return true;
            }
            Id::Opener(e) => {
                self.cfg.opener = Some(e);
                self.save_cfg();
            }
            Id::Forget => {
                if let Some(p) = self.target.as_ref().and_then(|t| t.path.clone()) {
                    self.cfg.closed.retain(|c| c.1 != p);
                    self.save_cfg();
                    self.closed = self.closed_wins();
                    self.invalidate();
                }
                return true;
            }
            Id::Exit => {
                unsafe {
                    let _ = DestroyWindow(self.hwnd);
                }
                return true;
            }
        }
        self.invalidate();
        false
    }

    /// 别名输入框保存：清空或改回原名就删掉别名
    fn set_alias(&mut self, d: edit::Done) {
        match d.key {
            edit::Key::Repo(r) => {
                if d.text.is_empty() || d.text == r {
                    self.cfg.repo_alias.remove(&r);
                } else {
                    self.cfg.repo_alias.insert(r, d.text);
                }
            }
            edit::Key::Branch(r, b) => {
                if d.text.is_empty() || d.text == b {
                    self.cfg.branch_alias.remove(&(r, b));
                } else {
                    self.cfg.branch_alias.insert((r, b), d.text);
                }
            }
        }
        self.save_cfg();
        self.invalidate();
    }

    /// 改透明度（拖滑块时实时生效，松手再存盘）
    pub fn set_opacity(&mut self, v: u32) {
        self.cfg.opacity = v;
        self.load_theme();
    }
}

/// 系统窗口事件回调（本线程的消息循环里调用）：转成消息交给窗口过程，不在回调里碰 App
unsafe extern "system" fn win_event(_: HWINEVENTHOOK, ev: u32, w: HWND, obj: i32, child: i32, _: u32, _: u32) {
    // 只要窗口本身的事件（OBJID_WINDOW、CHILDID_SELF），不要窗口里控件的
    if obj == 0 && child == 0 {
        unsafe {
            let _ = PostMessageW(Some(HWND(MAIN.load(Relaxed) as _)), WM_APP_EVENT, WPARAM(ev as usize), LPARAM(w.0 as isize));
        }
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            // 整个窗口都是客户区，没有系统边框
            WM_NCCALCSIZE if wp.0 != 0 => return LRESULT(0),
            WM_ERASEBKGND => return LRESULT(1),
            _ => {}
        }
        if APP.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let a = app();
        match msg {
            WM_PAINT => a.paint(),
            WM_SIZE => {
                if let Some(s) = &a.surf {
                    a.gfx.resize(s, (lp.0 & 0xFFFF) as u32, ((lp.0 >> 16) & 0xFFFF) as u32);
                }
                a.invalidate();
            }
            WM_DPICHANGED => {
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
                a.surf = None;
                a.save_pos();
            }
            // 只能左右拖边改宽度，高度跟着内容
            WM_NCHITTEST => {
                let p = lparam_pt(lp);
                let mut r = RECT::default();
                let _ = GetWindowRect(hwnd, &mut r);
                let e = (EDGE * scale(hwnd)) as i32;
                let code = if p.x < r.left + e {
                    HTLEFT
                } else if p.x >= r.right - e {
                    HTRIGHT
                } else {
                    HTCLIENT
                };
                return LRESULT(code as isize);
            }
            WM_GETMINMAXINFO => {
                let mm = &mut *(lp.0 as *mut MINMAXINFO);
                mm.ptMinTrackSize.x = (view::MIN_W * scale(hwnd)) as i32;
            }
            WM_EXITSIZEMOVE => a.save_pos(),
            WM_WINDOWPOSCHANGING => {
                // 在桌面层时不盖住其他程序
                let pos = &mut *(lp.0 as *mut WINDOWPOS);
                if pos.flags & SWP_NOZORDER == SET_WINDOW_POS_FLAGS(0) && !a.cfg.topmost && !a.front {
                    pos.hwndInsertAfter = HWND_BOTTOM;
                }
            }
            // 点托盘图标激活：到最前面
            WM_ACTIVATE if (wp.0 & 0xFFFF) as u32 != WA_INACTIVE => a.raise(true),
            // 点磁贴不抢焦点，直接切到编辑器窗口
            WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
            WM_MOUSEMOVE => {
                if let Some((p0, r0, moved)) = a.press.as_mut() {
                    let mut p = POINT::default();
                    let _ = GetCursorPos(&mut p);
                    let (dx, dy) = (p.x - p0.x, p.y - p0.y);
                    if !*moved && (dx.abs() > GetSystemMetrics(SM_CXDRAG) || dy.abs() > GetSystemMetrics(SM_CYDRAG)) {
                        *moved = true;
                    }
                    if *moved {
                        // 整个窗口留在鼠标所在显示器的工作区里
                        let r = RECT { left: r0.left + dx, top: r0.top + dy, right: r0.right + dx, bottom: r0.bottom + dy };
                        let (x, y) = fit(&r, work_area(MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST)));
                        let _ = SetWindowPos(hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                    }
                    return LRESULT(0);
                }
                if !a.tracking {
                    let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                    let _ = TrackMouseEvent(&mut tme);
                    a.tracking = true;
                }
                let h = a.hit_at(lparam_pt(lp));
                if h != a.hover {
                    a.hover = h;
                    a.invalidate();
                }
            }
            WM_MOUSELEAVE => {
                a.tracking = false;
                if a.hover.take().is_some() {
                    a.invalidate();
                }
            }
            // 菜单开着时，点磁贴只关菜单，不触发磁贴上的操作（和系统菜单一样）
            WM_LBUTTONDOWN if a.menu.is_some() => menu::close(a),
            WM_LBUTTONDOWN => {
                let (mut p, mut r) = (POINT::default(), RECT::default());
                let _ = GetCursorPos(&mut p);
                let _ = GetWindowRect(hwnd, &mut r);
                a.press = Some((p, r, false));
                SetCapture(hwnd);
            }
            WM_LBUTTONUP => {
                let pr = a.press.take();
                let _ = ReleaseCapture();
                match pr {
                    Some((_, _, true)) => a.save_pos(),
                    Some(_) => {
                        if let Some(h) = a.hit_at(lparam_pt(lp)) {
                            a.click(h);
                        }
                    }
                    None => {}
                }
            }
            // 拖动中鼠标被别的窗口抢走（弹窗、Alt+Tab）：当作松开
            WM_CAPTURECHANGED if a.press.is_some() && lp.0 != hwnd.0 as isize => {
                if a.press.take().is_some_and(|p| p.2) {
                    a.save_pos();
                }
            }
            WM_RBUTTONUP => {
                let t = match a.hit_at(lparam_pt(lp)) {
                    Some(Hit::Win(h)) => a.wins.iter().find(|w| w.hwnd == h).cloned(),
                    Some(Hit::Closed(i)) => a.closed.get(i).cloned(),
                    _ => None,
                };
                a.menu_at_cursor(t);
            }
            WM_APP_TRAY => match (lp.0 & 0xFFFF) as u32 {
                WM_LBUTTONUP => a.show_front(),
                WM_RBUTTONUP => a.menu_at_cursor(None),
                _ => {}
            },
            WM_SETTINGCHANGE => a.load_theme(),
            // 分辨率或显示器变了，跑到屏幕外就拉回来
            WM_DISPLAYCHANGE => {
                keep_on_screen(hwnd);
                a.save_pos();
            }
            WM_APP_EVENT => a.on_event(wp.0 as u32, HWND(lp.0 as _)),
            WM_APP_ALIAS => a.set_alias(*Box::from_raw(lp.0 as *mut edit::Done)),
            WM_TIMER => {
                if wp.0 == T_SCAN {
                    let _ = KillTimer(Some(hwnd), T_SCAN);
                    a.scan_pending = false;
                }
                a.rescan();
            }
            WM_DESTROY => {
                menu::close(a);
                a.tray(NIM_DELETE);
                PostQuitMessage(0);
            }
            // 资源管理器重启后托盘图标要重新加
            m if m == a.taskbar_created => a.tray(NIM_ADD),
            _ => return DefWindowProcW(hwnd, msg, wp, lp),
        }
        LRESULT(0)
    }
}

/// --demo 的数据：(打开的, 已关闭的)，README 截图用，不露真实仓库名
fn demo() -> (Vec<Win>, Vec<Win>) {
    let w = |hwnd: isize, editor, repo: &str, branch: &str, worktree, active| Win { hwnd, editor, repo: repo.into(), branch: branch.into(), worktree, active, path: None };
    let open = vec![
        w(1, Editor::Cursor, "payments-api", "release/2.4", false, false),
        w(2, Editor::Code, "mobile-app", "fix/login-crash", false, false),
        w(3, Editor::Cursor, "storefront-web", "main", false, true),
        w(4, Editor::Cursor, "storefront-web", "feat/checkout-v2", true, false),
    ];
    let closed = vec![w(0, Editor::Cursor, "docs-site", "main", false, false), w(0, Editor::Cursor, "infra", "chore/terraform-1.9", false, false)];
    (open, closed)
}

/// --dump <文件>：不开窗口，把扫到的窗口和耗时写成文本（测试和排查用）
fn dump(out: &Path) {
    let mut sc = Scanner::default();
    let t0 = std::time::Instant::now();
    let wins = sc.scan();
    let first = t0.elapsed().as_micros();
    let t1 = std::time::Instant::now();
    const N: u32 = 100;
    for _ in 0..N {
        sc.scan();
    }
    let rescan = t1.elapsed().as_micros() / N as u128;
    let t2 = std::time::Instant::now();
    for _ in 0..N {
        editors::electron_windows();
    }
    let mut s = format!("first scan {first} us, rescan avg {rescan} us (enum windows {} us)\nexes {:?}\n", t2.elapsed().as_micros() / N as u128, sc.exes);
    for w in wins {
        s += &format!("{w:?}\n");
    }
    let _ = std::fs::write(out, s);
}

fn main() {
    // 没有控制台，崩溃信息写到 %TEMP%\ramulet-crash.txt
    std::panic::set_hook(Box::new(|info| {
        let _ = std::fs::write(std::env::temp_dir().join("ramulet-crash.txt"), format!("{info}\n{}", std::backtrace::Backtrace::force_capture()));
    }));
    let args: Vec<String> = std::env::args().collect();
    if let Some(out) = arg(&args, "--dump") {
        return dump(&out);
    }
    let data = arg(&args, "--data").unwrap_or_else(|| PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("Ramulet"));
    unsafe {
        let mutex = wide(&format!("Ramulet-{}", data.to_string_lossy().replace('\\', "/").to_lowercase()));
        let _guard = CreateMutexW(None, true, PCWSTR(mutex.as_ptr()));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // 主线程没有输入框：关掉本线程的输入法。否则右键菜单一拿到焦点，系统就把输入法（如搜狗拼音）加载进本进程，私有内存 +34 MB。
        // 别名输入框在自己的线程里，那里输入法照常可用
        let _ = windows::Win32::UI::Input::Ime::ImmDisableIME(windows::Win32::System::Threading::GetCurrentThreadId());
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        let mut cfg = Config::parse(&std::fs::read_to_string(data.join("config.txt")).unwrap_or_default());
        if let Some(l) = args.iter().position(|a| a == "--lang").and_then(|i| args.get(i + 1)) {
            cfg.en = Some(l == "en");
        }
        config::set_lang(cfg.en);

        let d2d: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).expect("D2D 初始化失败");
        let gfx = gfx::Gfx::new(&d2d).expect("D3D 初始化失败");
        let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).expect("DirectWrite 初始化失败");
        let fonts = Fonts::new(&dw);

        let module = GetModuleHandleW(None).unwrap_or_default();
        let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        // exe 里的 1 号图标；托盘用系统小图标尺寸的那一张
        let icon = LoadIconW(Some(module.into()), PCWSTR(1 as _)).or_else(|_| LoadIconW(None, IDI_APPLICATION)).unwrap_or_default();
        let (sx, sy) = (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON));
        let tray_icon = LoadImageW(Some(module.into()), PCWSTR(1 as _), IMAGE_ICON, sx, sy, LR_DEFAULTCOLOR).map_or(icon, |h| HICON(h.0));
        type Proc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;
        let classes: [(Proc, PCWSTR); 2] = [(wnd_proc, CLASS), (menu::menu_proc, menu::MENU_CLASS)];
        for (proc_, class) in classes {
            let wc = WNDCLASSW { lpfnWndProc: Some(proc_), lpszClassName: class, hCursor: cursor, hIcon: icon, ..Default::default() };
            RegisterClassW(&wc);
        }
        let topmost = cfg.topmost || args.iter().any(|a| a == "--top");
        cfg.topmost = topmost;
        // 默认放在主屏工作区右上角
        let s = GetDpiForSystem() as f32 / 96.0;
        let mut wa = RECT::default();
        let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut wa as *mut _ as _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
        if cfg.pos[2] == 0 {
            let w = (DEFAULT_W * s) as i32;
            cfg.pos = [wa.right - w - (24.0 * s) as i32, wa.top + (24.0 * s) as i32, w];
        }
        // 不置顶时用桌面（Progman）做所有者：始终在桌面之上、其他窗口之下；WS_EX_TOOLWINDOW 不占任务栏按钮和 Alt+Tab，入口是托盘图标
        let owner = if topmost { 0 } else { desktop_owner() };
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP | if topmost { WS_EX_TOPMOST } else { WINDOW_EX_STYLE(0) },
            CLASS,
            w!("Ramulet"),
            WS_POPUP | WS_THICKFRAME,
            cfg.pos[0],
            cfg.pos[1],
            cfg.pos[2],
            (160.0 * s) as i32,
            (owner != 0).then_some(HWND(owner as _)),
            None,
            None,
            None,
        )
        .expect("创建窗口失败");
        MAIN.store(hwnd.0 as isize, Relaxed);

        let mut a = Box::new(App {
            data,
            cfg,
            hwnd,
            d2d,
            gfx,
            dw,
            fonts,
            theme: Theme::new(true, 1.0),
            force_dark: if args.iter().any(|a| a == "--light") { Some(false) } else if args.iter().any(|a| a == "--dark") { Some(true) } else { None },
            surf: None,
            scanner: Scanner::default(),
            wins: Vec::new(),
            closed: Vec::new(),
            tray_icon,
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            scan_pending: false,
            sys_dark: true,
            menu: None,
            hits: Vec::new(),
            hover: None,
            tracking: false,
            press: None,
            front: false,
            target: None,
            menu_pt: POINT::default(),
            demo: args.iter().any(|a| a == "--demo"),
        });
        if a.demo {
            (a.wins, a.closed) = demo();
        }
        APP = &mut *a;
        a.load_theme();
        a.tray(NIM_ADD);
        a.rescan();
        // 编辑器窗口开关、改标题、前台切换都有事件，磁贴立刻跟上；不在本进程里注入（OUTOFCONTEXT）
        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        for (lo, hi) in [(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND), (EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE), (EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_NAMECHANGE)] {
            SetWinEventHook(lo, hi, None, Some(win_event), 0, 0, flags);
        }
        SetTimer(Some(hwnd), T_POLL, POLL_MS, None);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        // 新建的窗口在 z 序最前面，那时窗口过程还管不到层级：显示出来后再压回桌面层
        if !topmost {
            let _ = SetWindowPos(hwnd, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
