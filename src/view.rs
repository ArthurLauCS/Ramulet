//! 磁贴的排版和绘制。尺寸都是 DIP，绘制时由渲染目标按窗口 DPI 缩放。
//! 标题行（名字、置顶按钮）下面是窗口磁贴网格：每块上行仓库名、下行分支名，同一仓库同一个颜色、排在一起；
//! 工作树右上角一个分叉图标；最近用过的那个窗口描一圈边。下面是已关闭的窗口，置灰。列数跟着宽度走，高度跟着磁贴数走。

use std::collections::HashMap;

use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::*;
use windows_numerics::Vector2;

use crate::App;
use crate::config::tr;
use crate::editors::{Editor, Win};

const PAD: f32 = 12.0;
const HEAD: f32 = 40.0;
const GAP: f32 = 8.0;
const TILE_H: f32 = 54.0;
const TILE_MIN: f32 = 170.0;
pub const MIN_W: f32 = 180.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Win(isize),
    /// 已关闭列表里的第几个
    Closed(usize),
    /// 右上角的置顶按钮
    Pin,
}

pub struct Theme {
    pub dark: bool,
    pub tint: D2D1_COLOR_F,
    pub text: D2D1_COLOR_F,
    pub muted: D2D1_COLOR_F,
    pub faint: D2D1_COLOR_F,
    pub track: D2D1_COLOR_F,
    pub hover: D2D1_COLOR_F,
    pub accent: D2D1_COLOR_F,
    pub danger: D2D1_COLOR_F,
    /// 底色太薄时文字压在壁纸上，加阴影（和桌面图标一样）
    pub shadow: bool,
}

pub fn rgba(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}

pub fn with_a(c: D2D1_COLOR_F, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { a: c.a * a, ..c }
}

fn hex(v: u32) -> D2D1_COLOR_F {
    rgba((v >> 16) as f32 / 255.0, (v >> 8 & 0xFF) as f32 / 255.0, (v & 0xFF) as f32 / 255.0, 1.0)
}

impl Theme {
    /// 磁贴的配色；opacity 是底色的不透明度（0.2～1）
    pub fn new(dark: bool, opacity: f32) -> Theme {
        let (bg, fg) = if dark { (0x20 as f32 / 255.0, 0xF3 as f32 / 255.0) } else { (0xF3 as f32 / 255.0, 0x1A as f32 / 255.0) };
        let g = |a: f32| rgba(fg, fg, fg, a);
        Theme {
            dark,
            tint: rgba(bg, bg, bg, opacity * if dark { 0.80 } else { 0.88 }),
            shadow: opacity < 0.5,
            text: g(1.0),
            muted: g(if dark { 0.66 } else { 0.64 }),
            faint: g(if dark { 0.07 } else { 0.06 }),
            track: g(if dark { 0.14 } else { 0.12 }),
            hover: g(0.07),
            accent: if dark { hex(0x60A5FA) } else { hex(0x2563EB) },
            danger: if dark { rgba(0.957, 0.392, 0.341, 1.0) } else { rgba(0.769, 0.169, 0.110, 1.0) },
        }
    }

    /// 右键菜单用的不透明面板：不跟磁贴的透明度走
    pub fn panel(dark: bool) -> Theme {
        let bg = if dark { 0x2C as f32 / 255.0 } else { 0xF9 as f32 / 255.0 };
        Theme { tint: rgba(bg, bg, bg, 1.0), ..Theme::new(dark, 1.0) }
    }
}

/// 仓库配色：深色、浅色背景各一套，色相相同。只留一眼能分开的 6 种（橙和黄、红和粉放一起分不清），超过 6 个仓库才重色
const REPO_DARK: [u32; 6] = [0x60A5FA, 0x34D399, 0xFBBF24, 0xF472B6, 0xA78BFA, 0x22D3EE];
const REPO_LIGHT: [u32; 6] = [0x2563EB, 0x059669, 0xB45309, 0xDB2777, 0x7C3AED, 0x0891B2];

/// 每个仓库按名字散列到固定的颜色，换窗口、开关窗口都不变；和已分到的撞色时顺延到下一个空色（先到先得，打开的排在已关闭的前面）
pub fn repo_colors<'a>(wins: impl IntoIterator<Item = &'a Win>) -> HashMap<&'a str, usize> {
    let mut m: HashMap<&str, usize> = HashMap::new();
    for w in wins {
        if m.contains_key(w.repo.as_str()) {
            continue;
        }
        let h = w.repo.to_lowercase().bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619)) as usize;
        let slot = (0..REPO_DARK.len()).map(|k| (h + k) % REPO_DARK.len()).find(|s| !m.values().any(|v| v == s)).unwrap_or(h % REPO_DARK.len());
        m.insert(&w.repo, slot);
    }
    m
}

pub struct Fonts {
    pub f11: IDWriteTextFormat,
    pub f12: IDWriteTextFormat,
    pub f12b: IDWriteTextFormat,
    pub f14b: IDWriteTextFormat,
    pub icon: IDWriteTextFormat,
}

impl Fonts {
    pub fn new(dw: &IDWriteFactory) -> Fonts {
        let mk = |size: f32, weight: DWRITE_FONT_WEIGHT| unsafe {
            let f = dw.CreateTextFormat(w!("Segoe UI Variable Text"), None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!("zh-CN")).expect("创建字体失败");
            let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
            let trim = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, delimiter: 0, delimiterCount: 0 };
            if let Ok(sign) = dw.CreateEllipsisTrimmingSign(&f) {
                let _ = f.SetTrimming(&trim, &sign);
            }
            f
        };
        // Win11 的图标字体是 Segoe Fluent Icons，Win10 只有 Segoe MDL2 Assets，两者用到的码位相同
        let icon = unsafe {
            let mut coll = None;
            let _ = dw.GetSystemFontCollection(&mut coll, false);
            let (mut idx, mut found) = (0u32, BOOL(0));
            if let Some(c) = coll {
                let _ = c.FindFamilyName(w!("Segoe Fluent Icons"), &mut idx, &mut found);
            }
            let fam = if found.as_bool() { w!("Segoe Fluent Icons") } else { w!("Segoe MDL2 Assets") };
            let f = dw.CreateTextFormat(fam, None, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, 14.0, w!("en-US")).expect("创建字体失败");
            let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            f
        };
        Fonts { f11: mk(11.0, DWRITE_FONT_WEIGHT_NORMAL), f12: mk(12.0, DWRITE_FONT_WEIGHT_NORMAL), f12b: mk(12.0, DWRITE_FONT_WEIGHT_SEMI_BOLD), f14b: mk(14.0, DWRITE_FONT_WEIGHT_SEMI_BOLD), icon }
    }
}

pub fn rect(l: f32, t: f32, r: f32, b: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: l, top: t, right: r, bottom: b }
}

pub fn inside(r: &D2D_RECT_F, x: f32, y: f32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

pub struct P<'a> {
    pub dc: &'a ID2D1DeviceContext,
    pub br: &'a ID2D1SolidColorBrush,
    pub dw: &'a IDWriteFactory,
    /// 文字加阴影
    pub shadow: bool,
}

#[derive(Clone, Copy)]
pub enum Align {
    L,
    C,
    R,
}

impl P<'_> {
    pub fn color(&self, c: D2D1_COLOR_F) {
        unsafe { self.br.SetColor(&c) };
    }

    pub fn fill(&self, r: D2D_RECT_F, radius: f32, c: D2D1_COLOR_F) {
        self.color(c);
        unsafe { self.dc.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius }, self.br) };
    }

    pub fn dot(&self, x: f32, y: f32, r: f32, c: D2D1_COLOR_F) {
        self.color(c);
        unsafe { self.dc.FillEllipse(&D2D1_ELLIPSE { point: Vector2 { X: x, Y: y }, radiusX: r, radiusY: r }, self.br) };
    }

    fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: D2D1_COLOR_F) {
        self.color(c);
        unsafe { self.dc.DrawLine(Vector2 { X: x0, Y: y0 }, Vector2 { X: x1, Y: y1 }, self.br, 1.3, None) };
    }

    /// 在 r 里画一行字，返回文字实际宽度
    pub fn text(&self, s: &str, r: D2D_RECT_F, f: &IDWriteTextFormat, c: D2D1_COLOR_F, al: Align) -> f32 {
        let w: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let Ok(l) = self.dw.CreateTextLayout(&w, f, (r.right - r.left).max(1.0), (r.bottom - r.top).max(1.0)) else { return 0.0 };
            let _ = l.SetTextAlignment(match al {
                Align::L => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::C => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::R => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            if self.shadow {
                self.color(rgba(0.0, 0.0, 0.0, 0.55 * c.a));
                self.dc.DrawTextLayout(Vector2 { X: r.left + 1.0, Y: r.top + 1.0 }, &l, self.br, D2D1_DRAW_TEXT_OPTIONS_NONE);
            }
            self.color(c);
            self.dc.DrawTextLayout(Vector2 { X: r.left, Y: r.top }, &l, self.br, D2D1_DRAW_TEXT_OPTIONS_NONE);
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = l.GetMetrics(&mut m);
            m.width
        }
    }

    /// 工作树标记：git 分叉图标，(x, y) 是 10×12 的左上角
    fn fork(&self, x: f32, y: f32, c: D2D1_COLOR_F) {
        self.line(x + 2.0, y + 2.0, x + 2.0, y + 10.0, c);
        self.line(x + 8.0, y + 3.5, x + 2.5, y + 8.5, c);
        for (dx, dy) in [(2.0, 1.6), (2.0, 10.4), (8.0, 1.6)] {
            self.dot(x + dx, y + dy, 1.7, c);
        }
    }
}

/// 一帧的排版结果
pub struct Frame {
    pub h: f32,
    pub hits: Vec<(D2D_RECT_F, Hit)>,
}

pub fn columns(w: f32) -> usize {
    (((w - 2.0 * PAD + GAP) / (TILE_MIN + GAP)).floor() as usize).max(1)
}

/// 画整块磁贴
pub fn draw(a: &App, w: f32, h: f32) -> Frame {
    let p = P { dc: &a.gfx.dc, br: &a.gfx.brush, dw: &a.dw, shadow: a.theme.shadow };
    let (th, f) = (&a.theme, &a.fonts);
    let mut hits = Vec::new();
    let (l, r) = (PAD, w - PAD);
    unsafe {
        p.dc.Clear(Some(&rgba(0.0, 0.0, 0.0, 0.0)));
        p.fill(rect(0.0, 0.0, w, h), 0.0, th.tint);
    }

    // 标题行：名字、窗口数、右上角的置顶按钮
    let tw = p.text("Ramulet", rect(l + 2.0, 8.0, r, 32.0), &f.f14b, th.text, Align::L);
    if !a.wins.is_empty() {
        p.text(&a.wins.len().to_string(), rect(l + tw + 10.0, 9.0, r, 32.0), &f.f11, th.muted, Align::L);
    }
    let pin = rect(r - 26.0, 8.0, r + 2.0, 32.0);
    if a.hover == Some(Hit::Pin) {
        p.fill(pin, 5.0, th.hover);
    }
    // Segoe Fluent Icons：E718 图钉，E840 按下的图钉
    let (glyph, gc) = if a.cfg.topmost { ("\u{E840}", th.accent) } else { ("\u{E718}", th.muted) };
    p.text(glyph, pin, &f.icon, gc, Align::C);
    hits.push((pin, Hit::Pin));

    let mut y = HEAD;
    let cols = columns(w);
    let tw = (r - l - GAP * (cols - 1) as f32) / cols as f32;
    let colors = repo_colors(a.wins.iter().chain(&a.closed));
    let pal = if th.dark { REPO_DARK } else { REPO_LIGHT };
    // 两种编辑器都开着时才标出 VS Code 窗口
    let mixed = a.wins.iter().any(|x| x.editor == Editor::Code) && a.wins.iter().any(|x| x.editor == Editor::Cursor);
    // 先画打开的，再画已关闭的（置灰，上面一行小标题）
    for (list, closed) in [(&a.wins, false), (&a.closed, true)] {
        if closed {
            if list.is_empty() {
                break;
            }
            y += 4.0;
            p.text(tr("已关闭 · 点击重新打开", "Closed · click to reopen"), rect(l + 2.0, y, r, y + 20.0), &f.f11, th.muted, Align::L);
            y += 24.0;
        } else if list.is_empty() {
            p.text(tr("没有打开的 Cursor / VS Code 窗口", "No Cursor / VS Code windows open"), rect(l, y, r, y + TILE_H), &f.f12, th.muted, Align::C);
            y += TILE_H + GAP;
            continue;
        }
        let k = if closed { 0.5 } else { 1.0 };
        for (i, win) in list.iter().enumerate() {
            let (x, ty) = (l + (i % cols) as f32 * (tw + GAP), y + (i / cols) as f32 * (TILE_H + GAP));
            let t = rect(x, ty, x + tw, ty + TILE_H);
            let hit = if closed { Hit::Closed(i) } else { Hit::Win(win.hwnd) };
            p.fill(t, 6.0, if a.hover == Some(hit) { th.track } else { th.faint });
            if win.active {
                p.color(with_a(th.accent, 0.9));
                unsafe { p.dc.DrawRoundedRectangle(&D2D1_ROUNDED_RECT { rect: rect(x + 0.75, ty + 0.75, x + tw - 0.75, ty + TILE_H - 0.75), radiusX: 5.5, radiusY: 5.5 }, p.br, 1.5, None) };
            }
            let c = with_a(hex(pal[colors[win.repo.as_str()]]), k);
            p.fill(rect(x + 7.0, ty + 10.0, x + 10.0, ty + TILE_H - 10.0), 1.5, c);
            let mut right = x + tw - 10.0;
            if win.worktree {
                p.fork(right - 10.0, ty + 9.0, with_a(th.muted, k));
                right -= 16.0;
            }
            if mixed && !closed && win.editor == Editor::Code {
                right -= p.text("VS Code", rect(x + 18.0, ty + 6.0, right, ty + 24.0), &f.f11, th.muted, Align::R) + 6.0;
            }
            // 有别名用别名；颜色和排序仍按原名，改别名不会换色、换位置
            let repo = a.cfg.repo_alias.get(&win.repo).unwrap_or(&win.repo);
            p.text(repo, rect(x + 18.0, ty + 6.0, right, ty + 24.0), &f.f11, c, Align::L);
            let alias = a.cfg.branch_alias.get(&(win.repo.clone(), win.branch.clone()));
            let branch = alias.map_or(if win.branch.is_empty() { "—" } else { &win.branch }, String::as_str);
            p.text(branch, rect(x + 18.0, ty + 23.0, x + tw - 10.0, ty + TILE_H - 7.0), &f.f14b, if closed { th.muted } else { th.text }, Align::L);
            hits.push((t, hit));
        }
        y += list.len().div_ceil(cols) as f32 * (TILE_H + GAP);
    }
    Frame { h: y - GAP + PAD, hits }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(repo: &str) -> Win {
        Win { hwnd: 0, editor: Editor::Cursor, repo: repo.into(), branch: String::new(), worktree: false, active: false, path: None }
    }

    #[test]
    fn colors_distinct_and_stable() {
        let names = ["Champions_062_code", "rokemon_061", "robloxtemple", "Ramulet", "PushRight", "MDView"];
        let wins: Vec<Win> = names.iter().map(|n| win(n)).collect();
        let m = repo_colors(&wins);
        let mut slots: Vec<usize> = m.values().copied().collect();
        slots.sort();
        slots.dedup();
        assert_eq!(slots.len(), 6);
        // 只开一个仓库时颜色就是它散列到的那个，和开了哪些别的仓库无关
        assert_eq!(repo_colors(&wins[..1])["Champions_062_code"], m["Champions_062_code"]);
        assert_eq!(columns(440.0), 2);
        assert_eq!(columns(100.0), 1);
    }
}
