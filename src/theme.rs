//! 样式 brutal（新野兽派·柔和，ui-style 目录，2026-10-29）：全部取值都在这里，绘制代码只用名字。
//! 平色块、2 px 深灰边框、直角、硬阴影（偏移不模糊）；强调色是图标的绿。

use windows::Win32::Graphics::Direct2D::Common::*;

// ---- 色板（brutal 的 light / dark 两列） ----
pub const BG_LIGHT: u32 = 0xF4F3EF;
pub const BG_DARK: u32 = 0x1B1B1D;
pub const SURFACE_LIGHT: u32 = 0xFFFFFF;
pub const SURFACE_DARK: u32 = 0x262628;
pub const BORDER_LIGHT: u32 = 0x2B2B2B;
pub const BORDER_DARK: u32 = 0xD6D6D6;
pub const TEXT_LIGHT: u32 = 0x1A1A1A;
pub const TEXT_DARK: u32 = 0xF0EFEA;
pub const MUTED_LIGHT: u32 = 0x50504C;
pub const MUTED_DARK: u32 = 0xA8A8A3;
/// 细分隔线（菜单里的 1 px 线）：比 border 淡一档
pub const RULE_LIGHT: u32 = 0xCFCDC6;
pub const RULE_DARK: u32 = 0x45454A;
/// 悬停 / 键盘焦点行之外的“下沉一档”底色
pub const SUNK_LIGHT: u32 = 0xEAE9E4;
pub const SUNK_DARK: u32 = 0x1B1B1D;

/// 强调色（绿，取自 ui-style 的 App accents）：大块填充两种模式都用深色那个值，字用 #1a1a1a；
/// 作文字、线条用时浅色背景上取 ACCENT_LIGHT
pub const ACCENT_DARK: u32 = 0x4CC47A;
pub const ACCENT_LIGHT: u32 = 0x1D7A45;
/// 色块上的字
pub const INK: u32 = 0x1A1A1A;
/// 危险项（退出）悬停时的填充，取 brutal 的 red
pub const RED_LIGHT: u32 = 0xEF5A5F;
pub const RED_DARK: u32 = 0xF0595E;

/// 仓库配色：数据色，不是强调色。固定 6 种平色块，色相拉开（珊瑚、橙、黄、青柠、天蓝、粉），
/// 两种模式相同：色块上的字一律 INK，对比度都 ≥ 7:1；不用紫 / 靛，也不和强调色的绿撞。超过 6 个仓库才重色
pub const REPO: [u32; 6] = [0xF68582, 0xF09A3E, 0xF2D24A, 0xA9D65C, 0x56BDE8, 0xF2A6C8];
/// 已关闭磁贴左边那条色带的宽度
pub const STRIP: f32 = 6.0;

// ---- 形状 ----
pub const BORDER: f32 = 2.0;
/// 硬阴影偏移：静止、悬停、弹出层
pub const SHADOW: f32 = 3.0;
pub const SHADOW_HOVER: f32 = 4.0;
pub const SHADOW_POP: f32 = 4.0;

// ---- 字体 ----
// 每个角色一套（ui-style Type roles）：DISPLAY 仓库名 = 得意黑，MONO 分支名 = IBM Plex Mono，UI = 微软雅黑 UI。
// 前两个随程序带（fonts 文件夹），缺文件时退回 FONT_BODY / FONT_MONO_FALLBACK
pub const FONT_DISPLAY: &str = "Smiley Sans";
pub const FONT_MONO: &str = "IBM Plex Mono";
pub const FONT_MONO_FALLBACK: &str = "Cascadia Mono";
pub const FONT_BODY: &str = "Microsoft YaHei UI";
/// 随程序带的字体文件（fonts 文件夹里）
pub const FONT_FILES: [&str; 2] = ["SmileySans-Oblique.ttf", "IBMPlexMono-Regular.ttf"];

pub fn hex(v: u32) -> D2D1_COLOR_F {
    rgba((v >> 16) as f32 / 255.0, (v >> 8 & 0xFF) as f32 / 255.0, (v & 0xFF) as f32 / 255.0, 1.0)
}

pub fn rgba(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}

pub fn with_a(c: D2D1_COLOR_F, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { a: c.a * a, ..c }
}

pub struct Theme {
    pub dark: bool,
    /// 窗口底色，带磁贴透明度
    pub bg: D2D1_COLOR_F,
    /// 弹出层和已关闭磁贴的底色（不透明）
    pub surface: D2D1_COLOR_F,
    pub sunk: D2D1_COLOR_F,
    pub border: D2D1_COLOR_F,
    pub rule: D2D1_COLOR_F,
    pub text: D2D1_COLOR_F,
    pub muted: D2D1_COLOR_F,
    /// 强调色填充和它上面的字
    pub accent: D2D1_COLOR_F,
    pub ink: D2D1_COLOR_F,
    /// 强调色作线条 / 文字
    pub accent_line: D2D1_COLOR_F,
    pub red: D2D1_COLOR_F,
    /// 硬阴影颜色
    pub shadow: D2D1_COLOR_F,
    /// 底色太薄时，直接压在壁纸上的字加阴影（和桌面图标一样）
    pub text_shadow: bool,
}

impl Theme {
    /// 磁贴的配色；opacity 是窗口底色的不透明度（0.2～1）
    pub fn new(dark: bool, opacity: f32) -> Theme {
        let pick = |l: u32, d: u32| hex(if dark { d } else { l });
        Theme {
            dark,
            bg: with_a(pick(BG_LIGHT, BG_DARK), opacity),
            surface: pick(SURFACE_LIGHT, SURFACE_DARK),
            sunk: pick(SUNK_LIGHT, SUNK_DARK),
            border: pick(BORDER_LIGHT, BORDER_DARK),
            rule: pick(RULE_LIGHT, RULE_DARK),
            text: pick(TEXT_LIGHT, TEXT_DARK),
            muted: pick(MUTED_LIGHT, MUTED_DARK),
            accent: hex(ACCENT_DARK),
            ink: hex(INK),
            accent_line: pick(ACCENT_LIGHT, ACCENT_DARK),
            red: pick(RED_LIGHT, RED_DARK),
            shadow: if dark { rgba(0.0, 0.0, 0.0, 1.0) } else { rgba(0x1A as f32 / 255.0, 0x1A as f32 / 255.0, 0x1A as f32 / 255.0, 0x33 as f32 / 255.0) },
            text_shadow: opacity < 0.5,
        }
    }

    /// 右键菜单和别名框：不透明，不跟磁贴的透明度走
    pub fn panel(dark: bool) -> Theme {
        Theme::new(dark, 1.0)
    }
}

/// GDI 用的 COLORREF 顺序
pub fn colorref(v: u32) -> u32 {
    (v & 0xFF) << 16 | (v & 0xFF00) | (v >> 16 & 0xFF)
}
