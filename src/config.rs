//! 设置：%APPDATA%\Ramulet\config.txt，每行 键|值。
//! 别名一个一行：repo_alias|仓库\t别名、branch_alias|仓库\t分支\t别名（分支名和别名里可能有 |，没有 Tab）。
//! 关掉的窗口一个一行：closed|编辑器\t文件夹路径，按关闭先后；编辑器程序路径：exe|编辑器\t路径。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use windows::Win32::Globalization::GetUserDefaultUILanguage;

use crate::editors::Editor;

/// 已关闭的窗口最多记多少个，超出时丢掉最早关的
pub const MAX_CLOSED: usize = 30;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// 窗口左上角和宽度（物理像素）；宽度为 0 表示还没摆过
    pub pos: [i32; 3],
    pub topmost: bool,
    /// None 时跟随系统界面语言
    pub en: Option<bool>,
    /// 磁贴底色的不透明度（20～100）
    pub opacity: u32,
    /// 仓库名 → 别名，这个仓库的所有磁贴都用
    pub repo_alias: BTreeMap<String, String>,
    /// (仓库名, 分支名) → 别名；不同仓库的同名分支（如 develop）各自设
    pub branch_alias: BTreeMap<(String, String), String>,
    /// 关掉的编辑器窗口：(当时用的编辑器, 文件夹)，按关闭先后；再打开就移除
    pub closed: Vec<(Editor, PathBuf)>,
    /// 点已关闭的磁贴时用哪个编辑器打开；None = 还没选过，第一次点时让用户选
    pub opener: Option<Editor>,
    /// 见过的编辑器程序路径（编辑器全关了也能拉起）
    pub exes: BTreeMap<Editor, PathBuf>,
}

impl Default for Config {
    fn default() -> Config {
        Config { pos: [0; 3], topmost: false, en: None, opacity: 100, repo_alias: BTreeMap::new(), branch_alias: BTreeMap::new(), closed: Vec::new(), opener: None, exes: BTreeMap::new() }
    }
}

impl Config {
    pub fn parse(s: &str) -> Config {
        let mut c = Config::default();
        for l in s.trim_start_matches('\u{feff}').lines() {
            let Some((k, v)) = l.split_once('|') else { continue };
            match k {
                "pos" => {
                    let n: Vec<i32> = v.split(',').filter_map(|x| x.parse().ok()).collect();
                    if let [x, y, w] = n[..] {
                        c.pos = [x, y, w];
                    }
                }
                "topmost" => c.topmost = v == "1",
                "lang" => c.en = Some(v == "en"),
                "opacity" => c.opacity = v.parse().unwrap_or(100),
                "repo_alias" => {
                    if let Some((r, a)) = v.split_once('\t') {
                        c.repo_alias.insert(r.into(), a.into());
                    }
                }
                "branch_alias" => {
                    if let [r, b, a] = v.splitn(3, '\t').collect::<Vec<_>>()[..] {
                        c.branch_alias.insert((r.into(), b.into()), a.into());
                    }
                }
                "closed" => {
                    if let Some((e, p)) = v.split_once('\t').and_then(|(e, p)| Some((Editor::from_key(e)?, p))) {
                        c.closed.push((e, p.into()));
                    }
                }
                "opener" => c.opener = Editor::from_key(v),
                "exe" => {
                    if let Some((e, p)) = v.split_once('\t').and_then(|(e, p)| Some((Editor::from_key(e)?, p))) {
                        c.exes.insert(e, p.into());
                    }
                }
                _ => {}
            }
        }
        c
    }

    pub fn to_text(&self) -> String {
        let mut s = format!("pos|{},{},{}\ntopmost|{}\nopacity|{}\n", self.pos[0], self.pos[1], self.pos[2], if self.topmost { "1" } else { "0" }, self.opacity);
        if let Some(en) = self.en {
            s += if en { "lang|en\n" } else { "lang|zh\n" };
        }
        for (r, a) in &self.repo_alias {
            s += &format!("repo_alias|{r}\t{a}\n");
        }
        for ((r, b), a) in &self.branch_alias {
            s += &format!("branch_alias|{r}\t{b}\t{a}\n");
        }
        for (e, p) in &self.closed {
            s += &format!("closed|{}\t{}\n", e.key(), p.display());
        }
        if let Some(e) = self.opener {
            s += &format!("opener|{}\n", e.key());
        }
        for (e, p) in &self.exes {
            s += &format!("exe|{}\t{}\n", e.key(), p.display());
        }
        s
    }
}

static EN: AtomicBool = AtomicBool::new(false);

pub fn set_lang(en: Option<bool>) {
    // 主语言号在低 10 位，0x04 是中文
    EN.store(en.unwrap_or_else(|| (unsafe { GetUserDefaultUILanguage() } & 0x3FF) != 0x04), Relaxed);
}

pub fn en() -> bool {
    EN.load(Relaxed)
}

pub fn tr(zh: &'static str, en: &'static str) -> &'static str {
    if self::en() { en } else { zh }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let c = Config {
            pos: [-10, 20, 400],
            topmost: true,
            en: Some(true),
            opacity: 45,
            repo_alias: BTreeMap::from([("Champions_062_code".into(), "冠军 | 主".into())]),
            branch_alias: BTreeMap::from([(("rokemon_061".into(), "feat|x".into()), "宠物升星".into())]),
            closed: vec![(Editor::Cursor, "c:/WorkFile/中文 dir".into()), (Editor::Code, "d:/a".into())],
            opener: Some(Editor::Code),
            exes: BTreeMap::from([(Editor::Cursor, r"C:\Users\x\AppData\Local\Programs\cursor\Cursor.exe".into())]),
        };
        assert_eq!(Config::parse(&c.to_text()), c);
        assert_eq!(Config::parse("\u{feff}pos|1,2\nbad\nlang|zh"), Config { en: Some(false), ..Default::default() });
    }
}
