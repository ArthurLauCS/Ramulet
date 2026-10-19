//! 读打开着的 VS Code / Cursor 窗口：窗口标题里取文件夹名，编辑器自己的 storage.json 里查完整路径，
//! 直接读 .git/HEAD 拿分支（不起 git 进程）。工作树的 .git 是文件，指向 <主仓库>/.git/worktrees/<名字>，仓库名取主仓库的目录名。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

use crate::json::Json;

/// VS Code、Cursor 以及其他 Electron 程序的顶层窗口类名
pub const ELECTRON_CLASS: &str = "Chrome_WidgetWin_1";

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Editor {
    Code,
    Cursor,
}

impl Editor {
    /// 配置文件里的写法
    pub fn key(self) -> &'static str {
        match self {
            Editor::Code => "code",
            Editor::Cursor => "cursor",
        }
    }

    pub fn from_key(s: &str) -> Option<Editor> {
        [Editor::Code, Editor::Cursor].into_iter().find(|e| e.key() == s)
    }

    /// 界面上显示的名字
    pub fn label(self) -> &'static str {
        match self {
            Editor::Code => "VS Code",
            Editor::Cursor => "Cursor",
        }
    }

    /// 没见过它运行时去默认安装位置找（用户安装在前，全机安装在后）
    pub fn default_exes(self) -> Vec<PathBuf> {
        let local = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("Programs");
        let pf = PathBuf::from(std::env::var_os("ProgramFiles").unwrap_or_default());
        match self {
            Editor::Code => vec![local.join("Microsoft VS Code\\Code.exe"), pf.join("Microsoft VS Code\\Code.exe")],
            Editor::Cursor => vec![local.join("cursor\\Cursor.exe"), pf.join("Cursor\\Cursor.exe")],
        }
    }

    /// 窗口标题里的程序名
    fn app_name(self) -> &'static str {
        match self {
            Editor::Code => "Visual Studio Code",
            Editor::Cursor => "Cursor",
        }
    }

    fn storage(self) -> PathBuf {
        let dir = match self {
            Editor::Code => "Code",
            Editor::Cursor => "Cursor",
        };
        PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join(dir).join("User\\globalStorage\\storage.json")
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Win {
    pub hwnd: isize,
    pub editor: Editor,
    pub repo: String,
    /// 空 = 不是 git 仓库，或查不到文件夹路径（远程窗口等）
    pub branch: String,
    pub worktree: bool,
    /// 最近用过的编辑器窗口（z 序最靠前）
    pub active: bool,
    /// 打开的文件夹；None = 查不到（远程窗口等）
    pub path: Option<PathBuf>,
}

/// 更新已关闭列表：上次还在、这次没了的窗口加到末尾；重新打开了的移除；超过 max 丢掉最早的。返回有没有变
pub fn track_closed(closed: &mut Vec<(Editor, PathBuf)>, old: &[Win], new: &[Win], max: usize) -> bool {
    let before = closed.clone();
    let open = |p: &Path| new.iter().any(|w| w.path.as_deref() == Some(p));
    for w in old {
        if let Some(p) = &w.path {
            if !open(p) && !closed.iter().any(|c| &c.1 == p) {
                closed.push((w.editor, p.clone()));
            }
        }
    }
    closed.retain(|c| !open(&c.1));
    let over = closed.len().saturating_sub(max);
    closed.drain(..over);
    *closed != before
}

/// 同一仓库的排在一起（主检出在前，工作树按分支名），仓库按名字排，顺序不随切换窗口变
pub fn sort(wins: &mut [Win]) {
    wins.sort_by(|a, b| (a.repo.to_lowercase(), a.worktree, &a.branch, a.hwnd).cmp(&(b.repo.to_lowercase(), b.worktree, &b.branch, b.hwnd)));
}

#[derive(Default)]
pub struct Scanner {
    /// 进程号 → 是哪个编辑器（None = 别的程序）
    procs: HashMap<u32, Option<Editor>>,
    /// 扫到过的编辑器程序路径
    pub exes: HashMap<Editor, PathBuf>,
    /// storage.json 的修改时间、文件夹名 → 路径
    folders: HashMap<Editor, (Option<SystemTime>, HashMap<String, PathBuf>)>,
    /// 文件夹路径 → 所在仓库
    repos: HashMap<PathBuf, Repo>,
}

/// (HEAD 文件, 仓库名, 是不是工作树)
type Repo = (PathBuf, String, bool);

pub fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

/// 进程是哪个编辑器、程序路径
fn editor_of(pid: u32) -> Option<(Editor, PathBuf)> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        let _ = CloseHandle(h);
        let path = String::from_utf16_lossy(&buf[..n as usize]);
        let ed = match path.rsplit('\\').next().map(str::to_lowercase).as_deref() {
            Some("code.exe") if ok => Editor::Code,
            Some("cursor.exe") if ok => Editor::Cursor,
            _ => return None,
        };
        Some((ed, PathBuf::from(path)))
    }
}

/// 可见、无所有者、带标题的 Electron 顶层窗口，按 z 序从前到后：(窗口, 进程号, 标题)
pub fn electron_windows() -> Vec<(HWND, u32, String)> {
    unsafe extern "system" fn cb(w: HWND, lp: LPARAM) -> BOOL {
        unsafe {
            let out = &mut *(lp.0 as *mut Vec<(HWND, u32, String)>);
            if IsWindowVisible(w).as_bool() && GetWindow(w, GW_OWNER).is_err() && class_name(w) == ELECTRON_CLASS {
                let mut buf = [0u16; 512];
                let n = GetWindowTextW(w, &mut buf) as usize;
                if n > 0 {
                    let mut pid = 0;
                    GetWindowThreadProcessId(w, Some(&mut pid));
                    out.push((w, pid, String::from_utf16_lossy(&buf[..n])));
                }
            }
            TRUE
        }
    }
    let mut out = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

/// 标题里的文件夹名。默认标题是“文件 - 文件夹 - [配置名 -] 程序名”，Cursor 还会在程序名后面再接一段；
/// 从程序名往前找第一个已知的文件夹名，都不认识就取程序名前一段
fn root_name<'a>(title: &'a str, app: &str, known: &HashMap<String, PathBuf>) -> Option<&'a str> {
    let parts: Vec<&str> = title.split(" - ").collect();
    let i = parts.iter().rposition(|p| *p == app)?;
    parts[..i].iter().rev().find(|p| known.contains_key(**p)).or(parts[..i].last()).copied()
}

/// file:///c%3A/WorkFile/x → c:/WorkFile/x；不是本地文件（远程、WSL）返回 None
fn uri_path(uri: &str) -> Option<PathBuf> {
    let s = uri.strip_prefix("file:///")?.as_bytes();
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let hex = s.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
        match (s[i], hex) {
            (b'%', Some(b)) => {
                out.push(b);
                i += 3;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    Some(PathBuf::from(String::from_utf8_lossy(&out).into_owned()))
}

/// storage.json 里记的已打开窗口和有备份的文件夹：文件夹名 → 路径。同名文件夹只认第一个
fn load_folders(text: &str) -> HashMap<String, PathBuf> {
    let mut m = HashMap::new();
    let Some(j) = Json::parse(text.trim_start_matches('\u{feff}')) else { return m };
    let ws = j.get("windowsState");
    let opened = ws.get("openedWindows").arr().iter().chain([ws.get("lastActiveWindow")]).map(|w| w.get("folder"));
    let backups = j.get("backupWorkspaces").get("folders").arr().iter().map(|f| f.get("folderUri"));
    for p in opened.chain(backups).filter_map(|v| v.str()).filter_map(uri_path) {
        if let Some(n) = p.file_name() {
            m.entry(n.to_string_lossy().into_owned()).or_insert(p);
        }
    }
    m
}

/// 文件夹所在的仓库：(HEAD 文件, 仓库名, 是不是工作树)；往上找 .git，打开的是仓库里的子目录也行
fn locate(dir: &Path) -> Option<Repo> {
    let root = dir.ancestors().find(|d| d.join(".git").exists())?;
    let dot = root.join(".git");
    let gitdir = if dot.is_dir() {
        dot
    } else {
        let s = std::fs::read_to_string(&dot).ok()?;
        let g = PathBuf::from(s.trim().strip_prefix("gitdir:")?.trim());
        if g.is_relative() { root.join(g) } else { g }
    };
    let main = gitdir.parent().filter(|p| p.file_name().is_some_and(|n| n == "worktrees")).and_then(Path::parent).and_then(Path::parent);
    let name = main.unwrap_or(root).file_name()?.to_string_lossy().into_owned();
    Some((gitdir.join("HEAD"), name, main.is_some()))
}

/// 当前分支；分离头指针时是提交号前 7 位
fn branch(head: &Path) -> String {
    let s = std::fs::read_to_string(head).unwrap_or_default();
    let s = s.trim();
    s.strip_prefix("ref: refs/heads/").map_or_else(|| s.chars().take(7).collect(), str::to_string)
}

impl Scanner {
    /// stat = 本次扫描第一次用到这个编辑器：看 storage.json 变了没有，变了重读
    fn known(&mut self, ed: Editor, stat: bool) -> &HashMap<String, PathBuf> {
        let e = self.folders.entry(ed).or_default();
        if stat {
            let file = ed.storage();
            let mtime = std::fs::metadata(&file).and_then(|m| m.modified()).ok();
            if e.0 != mtime || mtime.is_none() {
                *e = (mtime, load_folders(&std::fs::read_to_string(&file).unwrap_or_default()));
            }
        }
        &e.1
    }

    /// 文件夹的 (仓库名, 分支, 是不是工作树)；不是仓库时仓库名就是 name，没有分支。
    /// 仓库位置不会变，找到一次就记住，之后每次只读 HEAD；不是仓库的不记，git init 之后能认出来
    pub fn describe(&mut self, path: Option<&Path>, name: &str) -> (String, String, bool) {
        let repo = path.and_then(|p| {
            let r = self.repos.get(p).cloned().or_else(|| locate(p));
            if let Some(r) = &r {
                self.repos.insert(p.to_path_buf(), r.clone());
            }
            r
        });
        match repo {
            Some((head, name, wt)) => (name, branch(&head), wt),
            None => (name.to_string(), String::new(), false),
        }
    }

    /// 当前所有编辑器窗口，按 sort 的顺序
    pub fn scan(&mut self) -> Vec<Win> {
        if self.procs.len() > 512 {
            self.procs.clear();
        }
        if self.repos.len() > 256 {
            self.repos.clear();
        }
        let mut out = Vec::new();
        let mut statted = Vec::new();
        for (w, pid, title) in electron_windows() {
            let ed = match self.procs.get(&pid) {
                Some(e) => *e,
                None => {
                    let e = editor_of(pid).map(|(e, exe)| {
                        self.exes.insert(e, exe);
                        e
                    });
                    self.procs.insert(pid, e);
                    e
                }
            };
            let Some(ed) = ed else { continue };
            let stat = !statted.contains(&ed);
            if stat {
                statted.push(ed);
            }
            let known = self.known(ed, stat);
            let Some(root) = root_name(&title, ed.app_name(), known) else { continue };
            let path = known.get(root).cloned();
            let root = root.to_string();
            let (repo, branch, worktree) = self.describe(path.as_deref(), &root);
            let active = out.is_empty();
            out.push(Win { hwnd: w.0 as isize, editor: ed, repo, branch, worktree, active, path });
        }
        sort(&mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed() {
        let w = |p: Option<&str>| Win { hwnd: 1, editor: Editor::Cursor, repo: String::new(), branch: String::new(), worktree: false, active: false, path: p.map(PathBuf::from) };
        let mut c = vec![(Editor::Code, PathBuf::from("c:/old"))];
        // b 关了、远程窗口（没有路径）关了不记；a 还开着
        assert!(track_closed(&mut c, &[w(Some("c:/a")), w(Some("c:/b")), w(None)], &[w(Some("c:/a"))], 30));
        assert_eq!(c, vec![(Editor::Code, PathBuf::from("c:/old")), (Editor::Cursor, PathBuf::from("c:/b"))]);
        // 没变化
        assert!(!track_closed(&mut c, &[w(Some("c:/a"))], &[w(Some("c:/a"))], 30));
        // old 重新打开了；a 关了，超过上限丢掉最早的 b
        assert!(track_closed(&mut c, &[w(Some("c:/a"))], &[w(Some("c:/old"))], 1));
        assert_eq!(c, vec![(Editor::Cursor, PathBuf::from("c:/a"))]);
    }

    #[test]
    fn titles() {
        let known = HashMap::from([("Champions_062_code".to_string(), PathBuf::new())]);
        let none = HashMap::new();
        assert_eq!(root_name("RewardConst.luau (未暂存的更改) (RewardConst.luau) - Champions_062_code - Cursor - 修改", "Cursor", &none), Some("Champions_062_code"));
        assert_eq!(root_name("● a - b.ts - Champions_062_code - Work - Visual Studio Code", "Visual Studio Code", &known), Some("Champions_062_code"));
        assert_eq!(root_name("repo - Visual Studio Code", "Visual Studio Code", &none), Some("repo"));
        assert_eq!(root_name("Visual Studio Code", "Visual Studio Code", &none), None);
        assert_eq!(root_name("Cursor Settings - Google Chrome", "Cursor", &none), None);
    }

    #[test]
    fn uris() {
        assert_eq!(uri_path("file:///c%3A/WorkFile/%E4%B8%AD%E6%96%87"), Some(PathBuf::from("c:/WorkFile/中文")));
        assert_eq!(uri_path("vscode-remote://ssh-remote%2Bhost/x"), None);
        let m = load_folders(r#"{"windowsState":{"lastActiveWindow":{"folder":"file:///c%3A/a/repo"},"openedWindows":[{"folder":"file:///d%3A/repo"},{"workspaceIdentifier":{}}]},"backupWorkspaces":{"folders":[{"folderUri":"file:///c%3A/b"}]}}"#);
        assert_eq!(m.get("repo"), Some(&PathBuf::from("d:/repo")));
        assert_eq!(m.get("b"), Some(&PathBuf::from("c:/b")));
    }

    #[test]
    fn git() {
        let t = std::env::temp_dir().join(format!("ramulet-test-{}", std::process::id()));
        let main = t.join("Main_repo");
        std::fs::create_dir_all(main.join(".git/worktrees/wt1")).unwrap();
        std::fs::create_dir_all(main.join("sub/dir")).unwrap();
        std::fs::create_dir_all(t.join("wt1")).unwrap();
        std::fs::write(main.join(".git/HEAD"), "ref: refs/heads/feat_S_x\n").unwrap();
        std::fs::write(main.join(".git/worktrees/wt1/HEAD"), "0123456789abcdef\n").unwrap();
        std::fs::write(t.join("wt1/.git"), format!("gitdir: {}\n", main.join(".git/worktrees/wt1").display())).unwrap();
        let (head, name, wt) = locate(&main.join("sub/dir")).unwrap();
        assert_eq!((branch(&head).as_str(), name.as_str(), wt), ("feat_S_x", "Main_repo", false));
        let (head, name, wt) = locate(&t.join("wt1")).unwrap();
        assert_eq!((branch(&head).as_str(), name.as_str(), wt), ("0123456", "Main_repo", true));
        assert_eq!(locate(&t), None);
        std::fs::remove_dir_all(&t).unwrap();
    }
}
