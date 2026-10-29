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
use crate::fontload;
use crate::theme::*;

const PAD: f32 = 14.0;
const HEAD: f32 = 44.0;
const GAP: f32 = 12.0;
const TILE_H: f32 = 56.0;
const TILE_MIN: f32 = 170.0;
pub const MIN_W: f32 = 188.0;
/// 磁贴里文字离磁贴左右边（含 2 px 边框）多远
const INSET: f32 = 14.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Win(isize),
    /// 已关闭列表里的第几个
    Closed(usize),
    /// 右上角的置顶按钮
    Pin,
}

/// 每个仓库按名字散列到固定的颜色，换窗口、开关窗口都不变；和已分到的撞色时顺延到下一个空色（先到先得，打开的排在已关闭的前面）
pub fn repo_colors<'a>(wins: impl IntoIterator<Item = &'a Win>) -> HashMap<&'a str, usize> {
    let mut m: HashMap<&str, usize> = HashMap::new();
    for w in wins {
        if m.contains_key(w.repo.as_str()) {
            continue;
        }
        let h = w.repo.to_lowercase().bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619)) as usize;
        let slot = (0..REPO.len()).map(|k| (h + k) % REPO.len()).find(|s| !m.values().any(|v| v == s)).unwrap_or(h % REPO.len());
        m.insert(&w.repo, slot);
    }
    m
}

pub struct Fonts {
    /// MONO（IBM Plex Mono 12）：分支名、数字
    pub mono12: IDWriteTextFormat,
    pub body12: IDWriteTextFormat,
    pub body12b: IDWriteTextFormat,
    pub body13: IDWriteTextFormat,
    /// DISPLAY（得意黑，只有一个字重）：仓库名 14、窗口标题 16
    pub repo14: IDWriteTextFormat,
    pub head16: IDWriteTextFormat,
    pub icon: IDWriteTextFormat,
}

impl Fonts {
    pub fn new(dw: &IDWriteFactory) -> Fonts {
        let bundled = fontload::Bundled::load(dw);
        // coll 为 None 用系统字体；带文件的字体用自带集合，文件缺了就退回系统字体，不报错
        let mk = |name: &str, coll: Option<&IDWriteFontCollection>, size: f32, weight: DWRITE_FONT_WEIGHT| unsafe {
            let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
            let f = dw.CreateTextFormat(PCWSTR(name.as_ptr()), coll, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!("zh-CN")).expect("创建字体失败");
            if let (Some(_), Some(fb)) = (coll, &bundled.fallback) {
                if let Ok(f1) = f.cast::<IDWriteTextFormat1>() {
                    let _ = f1.SetFontFallback(fb);
                }
            }
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
            let f = dw.CreateTextFormat(fam, None, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, 16.0, w!("en-US")).expect("创建字体失败");
            let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            f
        };
        let (n, b) = (DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_BOLD);
        let (disp, mono) = (bundled.display.as_ref(), bundled.mono.as_ref());
        Fonts {
            mono12: mk(if mono.is_some() { FONT_MONO } else { FONT_MONO_FALLBACK }, mono, 12.0, n),
            body12: mk(FONT_BODY, None, 12.0, n),
            body12b: mk(FONT_BODY, None, 12.0, b),
            body13: mk(FONT_BODY, None, 13.0, n),
            repo14: mk(if disp.is_some() { FONT_DISPLAY } else { FONT_BODY }, disp, 14.0, if disp.is_some() { n } else { b }),
            head16: mk(if disp.is_some() { FONT_DISPLAY } else { FONT_BODY }, disp, 16.0, if disp.is_some() { n } else { b }),
            icon,
        }
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

    /// 平色块，直角
    pub fn fill(&self, r: D2D_RECT_F, c: D2D1_COLOR_F) {
        self.color(c);
        unsafe { self.dc.FillRectangle(&r, self.br) };
    }

    /// 向里的边框（宽 bw，画在 r 以内）
    pub fn frame(&self, r: D2D_RECT_F, bw: f32, c: D2D1_COLOR_F) {
        self.fill(rect(r.left, r.top, r.right, r.top + bw), c);
        self.fill(rect(r.left, r.bottom - bw, r.right, r.bottom), c);
        self.fill(rect(r.left, r.top + bw, r.left + bw, r.bottom - bw), c);
        self.fill(rect(r.right - bw, r.top + bw, r.right, r.bottom - bw), c);
    }

    /// brutal 的块：硬阴影（偏移 sh，0 就不画）、平色填充、2 px 边框
    pub fn block(&self, r: D2D_RECT_F, fill: D2D1_COLOR_F, border: D2D1_COLOR_F, shadow: D2D1_COLOR_F, sh: f32) {
        if sh > 0.0 {
            self.fill(rect(r.left + sh, r.top + sh, r.right + sh, r.bottom + sh), shadow);
        }
        self.fill(r, fill);
        self.frame(r, BORDER, border);
    }

    fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: D2D1_COLOR_F) {
        self.color(c);
        unsafe { self.dc.DrawLine(Vector2 { X: x0, Y: y0 }, Vector2 { X: x1, Y: y1 }, self.br, 2.0, None) };
    }

    /// 在 r 里画一行字，返回文字实际宽度
    pub fn text(&self, s: &str, r: D2D_RECT_F, f: &IDWriteTextFormat, c: D2D1_COLOR_F, al: Align) -> f32 {
        self.draw_text(s, r, f, c, al, false)
    }

    /// 和 text 一样，但左右按笔画对齐：L / R 让笔画的左 / 右边贴着 r 的左 / 右边，C 让笔画居中。
    /// 字形两边自带留白，图标字体里各个图标的留白还不一样，按字框对齐会差一两像素
    pub fn ink(&self, s: &str, r: D2D_RECT_F, f: &IDWriteTextFormat, c: D2D1_COLOR_F, al: Align) -> f32 {
        self.draw_text(s, r, f, c, al, true)
    }

    fn draw_text(&self, s: &str, r: D2D_RECT_F, f: &IDWriteTextFormat, c: D2D1_COLOR_F, al: Align, ink: bool) -> f32 {
        let w: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let Ok(l) = self.dw.CreateTextLayout(&w, f, (r.right - r.left).max(1.0), (r.bottom - r.top).max(1.0)) else { return 0.0 };
            let _ = l.SetTextAlignment(match al {
                Align::L => DWRITE_TEXT_ALIGNMENT_LEADING,
                Align::C => DWRITE_TEXT_ALIGNMENT_CENTER,
                Align::R => DWRITE_TEXT_ALIGNMENT_TRAILING,
            });
            // 笔画伸出（正）或缩进（负）排版框左右边多少
            let o = if ink { l.GetOverhangMetrics().unwrap_or_default() } else { DWRITE_OVERHANG_METRICS::default() };
            let x = r.left + match al {
                Align::L => o.left,
                Align::C => (o.left - o.right) / 2.0,
                Align::R => -o.right,
            };
            if self.shadow {
                self.color(rgba(0.0, 0.0, 0.0, 0.55 * c.a));
                self.dc.DrawTextLayout(Vector2 { X: x + 1.0, Y: r.top + 1.0 }, &l, self.br, D2D1_DRAW_TEXT_OPTIONS_NONE);
            }
            self.color(c);
            self.dc.DrawTextLayout(Vector2 { X: x, Y: r.top }, &l, self.br, D2D1_DRAW_TEXT_OPTIONS_NONE);
            let mut m = DWRITE_TEXT_METRICS::default();
            let _ = l.GetMetrics(&mut m);
            m.width
        }
    }

    /// 对勾：(x, y) 是左端的点，2 px 笔画
    pub fn tick(&self, x: f32, y: f32, c: D2D1_COLOR_F) {
        self.line(x, y, x + 3.5, y + 4.0, c);
        self.line(x + 3.5, y + 4.0, x + 10.0, y - 3.5, c);
    }

    /// 工作树标记：git 分叉图标，(x, y) 是 10×12 的左上角
    fn fork(&self, x: f32, y: f32, c: D2D1_COLOR_F) {
        // 2 px 笔画、方头：一条竖线、一条斜线，三个方形节点
        self.line(x + 2.0, y + 3.0, x + 2.0, y + 9.0, c);
        self.line(x + 8.0, y + 3.0, x + 2.0, y + 9.0, c);
        for (dx, dy) in [(2.0, 2.0), (2.0, 10.0), (8.0, 2.0)] {
            self.fill(rect(x + dx - 2.0, y + dy - 2.0, x + dx + 2.0, y + dy + 2.0), c);
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
    let (th, f) = (&a.theme, &a.fonts);
    // 直接压在窗口底色上的字（可能透出壁纸）在低透明度时加阴影；色块上的字不加
    let p = P { dc: &a.gfx.dc, br: &a.gfx.brush, dw: &a.dw, shadow: th.text_shadow };
    let q = P { dc: &a.gfx.dc, br: &a.gfx.brush, dw: &a.dw, shadow: false };
    let mut hits = Vec::new();
    let (l, r) = (PAD, w - PAD);
    unsafe { p.dc.Clear(Some(&rgba(0.0, 0.0, 0.0, 0.0))) };
    p.fill(rect(0.0, 0.0, w, h), th.bg);
    p.frame(rect(0.0, 0.0, w, h), BORDER, th.border);
    let label = if th.text_shadow { th.text } else { th.muted };

    // 标题行：名字（字的左边贴着内容左边）、窗口数、右上角的置顶按钮
    let tw = p.ink("Ramulet", rect(l, 8.0, r, 36.0), &f.head16, th.text, Align::L);
    if !a.wins.is_empty() {
        p.text(&a.wins.len().to_string(), rect(l + tw + 8.0, 8.0, r, 36.0), &f.mono12, label, Align::L);
    }
    // 置顶按钮：24 见方的块，按下（置顶中）是强调色填充
    let pin = rect(r - 24.0, 10.0, r, 34.0);
    let hot = a.hover == Some(Hit::Pin);
    let (fill, gc) = if a.cfg.topmost { (th.accent, th.ink) } else { (th.surface, th.text) };
    q.block(pin, fill, th.border, th.shadow, if hot { SHADOW_HOVER } else { SHADOW });
    // Segoe Fluent Icons：E718 图钉，E840 按下的图钉
    q.ink(if a.cfg.topmost { "\u{E840}" } else { "\u{E718}" }, pin, &f.icon, gc, Align::C);
    hits.push((pin, Hit::Pin));

    let mut y = HEAD;
    let cols = columns(w);
    let tw = (r - l - GAP * (cols - 1) as f32) / cols as f32;
    let colors = repo_colors(a.wins.iter().chain(&a.closed));
    // 两种编辑器都开着时才标出 VS Code 窗口
    let mixed = a.wins.iter().any(|x| x.editor == Editor::Code) && a.wins.iter().any(|x| x.editor == Editor::Cursor);
    // 先画打开的，再画已关闭的（置灰，上面一行小标题）
    for (list, closed) in [(&a.wins, false), (&a.closed, true)] {
        if closed {
            if list.is_empty() {
                break;
            }
            y += 4.0;
            p.ink(tr("已关闭 · 点击重新打开", "Closed · click to reopen"), rect(l, y, r, y + 20.0), &f.body12, label, Align::L);
            y += 28.0;
        } else if list.is_empty() {
            p.ink(tr("没有打开的 Cursor / VS Code 窗口", "No Cursor / VS Code windows open"), rect(l, y, r, y + 20.0), &f.body12, label, Align::L);
            y += 20.0 + GAP;
            continue;
        }
        for (i, win) in list.iter().enumerate() {
            let (x, ty) = (l + (i % cols) as f32 * (tw + GAP), y + (i / cols) as f32 * (TILE_H + GAP));
            let t = rect(x, ty, x + tw, ty + TILE_H);
            let hit = if closed { Hit::Closed(i) } else { Hit::Win(win.hwnd) };
            let hot = a.hover == Some(hit);
            let c = hex(REPO[colors[win.repo.as_str()]]);
            // 文字色：色块上是 INK；已关闭的磁贴是灰底，用 muted
            let fg = if closed { th.muted } else { th.ink };
            if closed {
                q.block(t, th.sunk, if hot { th.border } else { th.muted }, th.shadow, 0.0);
                // 仓库的颜色只留左边一条，分组还认得出
                q.fill(rect(x + BORDER, ty + BORDER, x + BORDER + STRIP, ty + TILE_H - BORDER), c);
            } else {
                q.block(t, c, th.border, th.shadow, if hot { SHADOW_HOVER } else { SHADOW });
            }
            if win.active {
                // 最近用过的窗口：磁贴外 2 px 空隙再套一圈 2 px 强调色线
                q.frame(rect(x - 4.0, ty - 4.0, x + tw + 4.0, ty + TILE_H + 4.0), BORDER, th.accent_line);
            }
            let tx = x + INSET + if closed { 4.0 } else { 0.0 };
            let mut right = x + tw - INSET;
            if win.worktree {
                q.fork(right - 10.0, ty + 10.0, fg);
                right -= 16.0;
            }
            if mixed && !closed && win.editor == Editor::Code {
                right -= q.text("VS Code", rect(tx, ty + 8.0, right, ty + 26.0), &f.body12b, fg, Align::R) + 6.0;
            }
            // 有别名用别名；颜色和排序仍按原名，改别名不会换色、换位置
            let repo = a.cfg.repo_alias.get(&win.repo).unwrap_or(&win.repo);
            q.text(repo, rect(tx, ty + 7.0, right, ty + 27.0), &f.repo14, fg, Align::L);
            let alias = a.cfg.branch_alias.get(&(win.repo.clone(), win.branch.clone()));
            let branch = alias.map_or(if win.branch.is_empty() { "—" } else { &win.branch }, String::as_str);
            q.text(branch, rect(tx, ty + 27.0, x + tw - INSET, ty + TILE_H - 6.0), &f.mono12, fg, Align::L);
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
