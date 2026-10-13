//! «Спросить у программы»: запускает `<программа> --help` и разбирает ключи из её справки.
//! Так Пульт знает аргументы любой программы, даже если её нет в базе знаний.
use crate::kb::Kb;
use crate::model::{FlagInfo, Script};
use crate::runner;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Help {
    pub program: String,
    pub argv: Vec<String>,
    pub usage: String,
    pub about: String,
    pub flags: Vec<FlagInfo>,
    pub text: String,
    pub error: Option<String>,
}

static CACHE: OnceLock<Mutex<HashMap<String, Help>>> = OnceLock::new();

/// Где лежит программа: сначала node_modules/.bin (от папки скрипта вверх до корня), потом PATH.
fn locate(bin: &str, cwd: &Path, root: &Path) -> Option<PathBuf> {
    let mut d = Some(cwd.to_path_buf());
    while let Some(dir) = d {
        for name in [format!("{bin}.cmd"), bin.to_string()] {
            let p = dir.join("node_modules").join(".bin").join(&name);
            if p.is_file() && (cfg!(windows) == name.ends_with(".cmd") || !cfg!(windows)) {
                return Some(p);
            }
        }
        if dir == root {
            break;
        }
        d = dir.parent().map(Path::to_path_buf);
    }
    runner::which(bin)
}

/// Разбор справки: строки вида «  -w, --watch   Описание».
pub fn parse(text: &str) -> (String, String, Vec<FlagInfo>) {
    let re = crate::re!(r"^\s{0,10}((?:-[A-Za-z0-9?](?:,\s*|\s*\|\s*|\s+))?--?[A-Za-z0-9][\w.\-]*(?:[ =]?(?:<[^>]+>|\[[^\]]+\]|[A-Z_]{2,}|\{[^}]+\}))?)\s{2,}(\S.*)$");
    let mut flags: Vec<FlagInfo> = Vec::new();
    let mut usage = String::new();
    let mut about = String::new();
    let mut last: Option<usize> = None;
    for l in text.lines() {
        if usage.is_empty() && l.trim_start().to_lowercase().starts_with("usage") {
            usage = l.trim().to_string();
        }
        if about.is_empty() && !l.trim().is_empty() && !l.trim_start().starts_with('-') && !l.to_lowercase().contains("usage") && l.trim().len() > 12 && !l.trim().ends_with(':') {
            about = l.trim().to_string();
        }
        if let Some(c) = re.captures(l) {
            let flag = c[1].trim().to_string();
            if !flags.iter().any(|f| f.flag == flag) && flags.len() < 200 {
                flags.push(FlagInfo { flag, text: c[2].trim().to_string(), used: false, from: "help".into() });
                last = Some(flags.len() - 1);
            }
        } else if let Some(i) = last {
            // продолжение описания на следующей строке
            let t = l.trim();
            if !t.is_empty() && !t.starts_with('-') && l.starts_with("      ") && flags[i].text.len() < 300 {
                flags[i].text.push(' ');
                flags[i].text.push_str(t);
            } else {
                last = None;
            }
        }
    }
    (usage, about, flags)
}

pub fn ask(s: &Script, root: &Path, kb: &Kb) -> Help {
    let cwd = root.join(if s.where_ == "." { "" } else { &s.where_ });
    // какую программу спрашивать: первую из базы знаний с полем help, иначе первое слово команды
    let (bin, help_arg, sub) = match s.info.tools.iter().find_map(|t| kb.tool(t)) {
        Some(t) if t.help.is_some() || t.help.is_none() => {
            let first_bin = t.bins.first().cloned().unwrap_or_default();
            let words: Vec<String> = first_bin.split_whitespace().map(str::to_string).collect();
            (words[0].clone(), t.help.clone().unwrap_or_else(|| "--help".into()), words[1..].to_vec())
        }
        _ => (s.tool.clone().unwrap_or_default(), "--help".into(), vec![]),
    };
    if bin.is_empty() || help_arg.is_empty() {
        return Help { error: Some("Не понял, у какой программы спрашивать".into()), ..Default::default() };
    }
    let Some(prog) = locate(&bin, &cwd, root) else {
        return Help { program: bin.clone(), error: Some(format!("Программа «{bin}» не найдена на компьютере")), ..Default::default() };
    };
    let mut argv = vec![prog.to_string_lossy().into_owned()];
    argv.extend(sub);
    argv.push(help_arg);
    let key = argv.join(" ");
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(h) = cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
        return h;
    }
    let h = match runner::capture_env(&argv, &cwd, Duration::from_secs(10), &[("NO_COLOR", "1"), ("FORCE_COLOR", "0")]) {
        Ok(c) => {
            let ansi = crate::re!(r"\x1b\[[0-9;?]*[A-Za-z]");
            let text = ansi.replace_all(&format!("{}{}", c.out, if c.out.trim().is_empty() { &c.err } else { "" }), "").into_owned();
            let (usage, about, flags) = parse(&text);
            Help { program: bin, argv, usage, about, flags, text: text.chars().take(60_000).collect(), error: c.timed_out.then(|| "Программа не ответила за 10 с".to_string()) }
        }
        Err(e) => Help { program: bin, argv, error: Some(e.to_string()), ..Default::default() },
    };
    if h.error.is_none() {
        if let Ok(mut c) = cache.lock() {
            c.insert(key, h.clone());
        }
    }
    h
}
