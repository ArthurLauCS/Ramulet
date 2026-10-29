//! 随程序带的字体：exe 旁的 fonts 文件夹（开发时是仓库的 assets/fonts），由 scripts/fonts.ps1 取得。
//! 做成 DirectWrite 字体集合（GDI 的 FR_PRIVATE 字体 DirectWrite 看不到）。任何一步失败都返回 None，
//! 调用方退回系统字体，不报错。

use std::path::PathBuf;

use windows::Win32::Graphics::DirectWrite::*;
use windows::core::*;

use crate::theme::{FONT_BODY, FONT_FILES};

pub struct Bundled {
    /// 得意黑（仓库名、标题）
    pub display: Option<IDWriteFontCollection>,
    /// IBM Plex Mono（分支名、数字）
    pub mono: Option<IDWriteFontCollection>,
    /// 得意黑只覆盖一部分汉字：缺的字先落到微软雅黑 UI，再交给系统的后备链，不显示方框
    pub fallback: Option<IDWriteFontFallback>,
}

/// 字体文件夹：先看 exe 旁的 fonts，再看开发布局 target\<profile>\ramulet.exe 往上两级的 assets\fonts
fn font_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join("fonts"), dir.join("..").join("..").join("assets").join("fonts")].into_iter().find(|d| d.join(FONT_FILES[0]).is_file() || d.join(FONT_FILES[1]).is_file())
}

fn collection(dw3: &IDWriteFactory3, path: &std::path::Path) -> Option<IDWriteFontCollection> {
    if !path.is_file() {
        return None;
    }
    unsafe {
        let wide: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
        let face = dw3.CreateFontFaceReference2(PCWSTR(wide.as_ptr()), None, 0, DWRITE_FONT_SIMULATIONS_NONE).ok()?;
        let builder = dw3.CreateFontSetBuilder().ok()?;
        builder.AddFontFaceReference2(&face).ok()?;
        let set = builder.CreateFontSet().ok()?;
        dw3.CreateFontCollectionFromFontSet(&set).ok()?.cast().ok()
    }
}

fn fallback(dw: &IDWriteFactory) -> Option<IDWriteFontFallback> {
    unsafe {
        let dw2: IDWriteFactory2 = dw.cast().ok()?;
        let b = dw2.CreateFontFallbackBuilder().ok()?;
        // 汉字、部首、标点、全角、兼容区、扩展 B 及以后
        let ranges = [
            (0x2E80, 0x2FDF),
            (0x3000, 0x303F),
            (0x3400, 0x4DBF),
            (0x4E00, 0x9FFF),
            (0xF900, 0xFAFF),
            (0xFF00, 0xFFEF),
            (0x20000, 0x2FA1F),
        ]
        .map(|(first, last)| DWRITE_UNICODE_RANGE { first, last });
        let name: Vec<u16> = FONT_BODY.encode_utf16().chain([0]).collect();
        b.AddMapping(&ranges, &[name.as_ptr()], None, None, PCWSTR::null(), 1.0).ok()?;
        if let Ok(sys) = dw2.GetSystemFontFallback() {
            let _ = b.AddMappings(&sys);
        }
        b.CreateFontFallback().ok()
    }
}

impl Bundled {
    pub fn load(dw: &IDWriteFactory) -> Bundled {
        let none = Bundled { display: None, mono: None, fallback: None };
        let (Some(dir), Ok(dw3)) = (font_dir(), dw.cast::<IDWriteFactory3>()) else { return none };
        Bundled { display: collection(&dw3, &dir.join(FONT_FILES[0])), mono: collection(&dw3, &dir.join(FONT_FILES[1])), fallback: fallback(dw) }
    }
}
