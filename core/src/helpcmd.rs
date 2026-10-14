//! «Спросить у программы»: запускает справку программы (для подкоманды - её собственную,
//! `git status -h`, `cargo check --help`; как спросить - поле `help` в базе) и разбирает ключи.
//! Так Пульт знает аргументы любой программы, даже если её нет в базе знаний.
//!
//! Программа запускается из папки данных Пульта, а не из проекта: иначе чужие настройки
//! (yarnPath в .yarnrc.yml, путь в rust-toolchain) подменили бы её своей. Файл из самого
//! проекта (node_modules/.bin, путь в команде) без согласия не запускается: `ask` возвращает
//! его полный путь в `confirm`, окно спрашивает пользователя и зовёт `ask_confirmed` с этим путём.
use crate::explain::word_of;
use crate::kb::Kb;
use crate::model::{FlagInfo, Script};
use crate::util::{inside, is_unc};
use crate::{paths, runner};
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
    /// полный путь программы из папки проекта: запустится только после согласия пользователя
    pub confirm: Option<String>,
}

static CACHE: OnceLock<Mutex<HashMap<String, Help>>> = OnceLock::new();

/// Где лежит программа: сначала node_modules/.bin (от папки скрипта вверх до корня), потом PATH.
/// Второе значение - файл лежит в проекте (его написал автор проекта, а не вы).
fn locate(bin: &str, cwd: &Path, root: &Path) -> Result<Option<(PathBuf, bool)>, String> {
    if is_unc(bin) {
        return Err("Сетевые пути (\\\\сервер\\...) не запускаю".into());
    }
    if bin.contains(['/', '\\']) {
        let p = if Path::new(bin).is_absolute() { PathBuf::from(bin) } else { cwd.join(bin) };
        return Ok(p.is_file().then_some((p, true)));
    }
    let mut d = Some(cwd.to_path_buf());
    while let Some(dir) = d {
        if !inside(root, &dir) {
            break;
        }
        for name in [format!("{bin}.cmd"), bin.to_string()] {
            let p = dir.join("node_modules").join(".bin").join(&name);
            if p.is_file() && (cfg!(windows) == name.ends_with(".cmd") || !cfg!(windows)) {
                return Ok(Some((p, true)));
            }
        }
        if dir == root {
            break;
        }
        d = dir.parent().map(Path::to_path_buf);
    }
    Ok(runner::which(bin).map(|p| {
        let local = inside(root, &p);
        (p, local)
    }))
}

/// Разбор справки: строки вида «  -w, --watch   Описание». Описание может стоять строкой ниже
/// (длинный ключ у git: «--[no-]porcelain[=<version>]» и под ним текст); «--[no-]x» пишется как «--x».
pub fn parse(text: &str) -> (String, String, Vec<FlagInfo>) {
    let re = crate::re!(
        r"^\s{0,10}((?:-[A-Za-z0-9?](?:,\s*|\s*\|\s*|\s+))?--?(?:\[no-\])?[A-Za-z0-9][\w.\-]*(?:\[?[ =]?(?:<[^>]+>|\[[^\]]+\]|[A-Z_]{2,}|\{[^}]+\})\]?)?)(?:\s{2,}(\S.*))?$"
    );
    let mut flags: Vec<FlagInfo> = Vec::new();
    let mut usage = String::new();
    let mut about = String::new();
    let mut last: Option<usize> = None;
    for l in text.lines() {
        let indent = l.len() - l.trim_start().len();
        if usage.is_empty() && l.trim_start().to_lowercase().starts_with("usage") {
            usage = l.trim().to_string();
        }
        if about.is_empty()
            && indent < 8
            && !l.trim().is_empty()
            && !l.trim_start().starts_with('-')
            && !l.to_lowercase().contains("usage")
            && l.trim().len() > 12
            && !l.trim().ends_with(':')
        {
            about = l.trim().to_string();
        }
        if let Some(c) = re.captures(l) {
            let flag = c[1].trim().replace("--[no-]", "--");
            if !flags.iter().any(|f| f.flag == flag) && flags.len() < 200 {
                let text = c.get(2).map(|m| m.as_str().trim().to_string()).unwrap_or_default();
                flags.push(FlagInfo { flag, text, used: false, from: "help".into() });
                last = Some(flags.len() - 1);
            }
        } else if let Some(i) = last {
            // продолжение описания на следующей строке
            let t = l.trim();
            if !t.is_empty() && !t.starts_with('-') && l.starts_with("      ") && flags[i].text.len() < 300 {
                if !flags[i].text.is_empty() {
                    flags[i].text.push(' ');
                }
                flags[i].text.push_str(t);
            } else {
                last = None;
            }
        }
    }
    // ключ без описания ни в строке, ни под ней - скорее строка примера, чем ключ
    flags.retain(|f| !f.text.is_empty());
    (usage, about, flags)
}

/// Подкоманда скрипта для справки: часть сразу за программой `ti`, если база знаний узнала в ней
/// подкоманду, и это простое слово - не файл, не путь и не ключ. Иначе справка общая.
fn sub_of(s: &Script, kb: &Kb, ti: usize) -> Option<String> {
    let parts = &s.info.parts;
    let i = parts.iter().position(|p| {
        let words: Vec<String> =
            p.text.split_whitespace().enumerate().map(|(k, w)| if k == 0 { word_of(w) } else { w.to_lowercase() }).collect();
        p.kind == "bin" && kb.match_tool(&words).is_some_and(|(t, _)| t == ti)
    })?;
    let w = parts.get(i + 1).filter(|p| p.kind == "sub")?.text.split_whitespace().next()?;
    crate::re!(r"^[A-Za-z][A-Za-z0-9_-]{0,40}$").is_match(w).then(|| w.to_string())
}

/// Варианты справки программы `ti` из базы по её шаблонам `help`.
fn kb_variants(s: &Script, kb: &Kb, ti: usize) -> Option<(String, Vec<Vec<String>>)> {
    let t = &kb.tools[ti];
    let words: Vec<String> = t.bins.first()?.split_whitespace().map(str::to_string).collect();
    let (first, rest) = words.split_first()?;
    let sub = sub_of(s, kb, ti);
    let mut out: Vec<Vec<String>> = Vec::new();
    for pat in t.help_patterns() {
        if pat.contains("{sub}") && sub.is_none() {
            continue;
        }
        let mut args = Vec::new();
        for w in pat.split_whitespace() {
            match w {
                // первое слово программы - сама программа (её путь найдёт locate), остальные - аргументы
                "{bin}" => args.extend(rest.iter().cloned()),
                "{sub}" => args.extend(sub.clone()),
                x => args.push(x.to_string()),
            }
        }
        if !args.is_empty() && !out.contains(&args) {
            out.push(args);
        }
    }
    (!out.is_empty()).then(|| (first.clone(), out))
}

/// Какую программу спрашивать и с какими аргументами - варианты по порядку (шаблоны `help` из базы:
/// сначала справка подкоманды, потом общая).
///
/// Программа - первая из самой команды скрипта (части «bin» объяснения): у npm-скрипта
/// «mytool --check» это mytool, а не npm. Из базы знаний - по её шаблонам, незнакомая - с `--help`.
/// `help: null` в базе - у программы справки нет, берётся следующая. Путь (скрипт .bat, обёртка)
/// не спрашивается: «--help» он не поймёт, а просто выполнится.
pub fn pick(s: &Script, kb: &Kb) -> Option<(String, Vec<Vec<String>>)> {
    let plain = |w: &str| !w.is_empty() && !w.contains(['/', '\\']);
    for p in s.info.parts.iter().filter(|p| p.kind == "bin") {
        let words: Vec<String> =
            p.text.split_whitespace().enumerate().map(|(k, w)| if k == 0 { word_of(w) } else { w.to_lowercase() }).collect();
        match kb.match_tool(&words) {
            Some((ti, _)) if kb.tools[ti].help.is_some() => return kb_variants(s, kb, ti),
            Some(_) => continue,
            None => {
                let first = p.text.split_whitespace().next().unwrap_or("");
                if plain(first) {
                    return Some((word_of(first), vec![vec!["--help".into()]]));
                }
            }
        }
    }
    // частей нет (ссылка на другой npm-скрипт): программа из его объяснения, иначе первое слово
    if let Some(ti) = s.info.tools.iter().filter_map(|id| kb.tools.iter().position(|t| &t.id == id)).find(|&i| kb.tools[i].help.is_some()) {
        return kb_variants(s, kb, ti);
    }
    let bin = s.tool.clone().filter(|b| plain(b))?;
    if kb.match_tool(std::slice::from_ref(&bin)).is_some_and(|(i, _)| kb.tools[i].help.is_none()) {
        return None;
    }
    Some((bin, vec![vec!["--help".into()]]))
}

/// Спросить справку. Программу из папки проекта не запускает - возвращает её путь в `confirm`.
pub fn ask(s: &Script, root: &Path, kb: &Kb) -> Help {
    ask_confirmed(s, root, kb, None)
}

/// `confirmed` - полный путь, который пользователь разрешил запустить (из `Help::confirm`).
pub fn ask_confirmed(s: &Script, root: &Path, kb: &Kb, confirmed: Option<&str>) -> Help {
    let cwd = root.join(if s.where_ == "." { "" } else { &s.where_ });
    let Some((bin, variants)) = pick(s, kb) else {
        return Help { error: Some("Не понял, у какой программы спрашивать".into()), ..Default::default() };
    };
    let prog = match locate(&bin, &cwd, root) {
        Ok(Some((p, local))) => {
            let full = p.to_string_lossy().into_owned();
            if local && confirmed != Some(full.as_str()) {
                return Help { program: bin, confirm: Some(full), ..Default::default() };
            }
            p
        }
        Ok(None) => {
            return Help {
                program: bin.clone(), error: Some(format!("Программа «{bin}» не найдена на компьютере")), ..Default::default()
            }
        }
        Err(e) => return Help { program: bin, error: Some(e), ..Default::default() },
    };
    // варианты по порядку: справка подкоманды, если она дала ключи, иначе следующий вариант
    let mut first_ok: Option<Help> = None;
    let mut last = Help::default();
    for args in variants {
        let mut argv = vec![prog.to_string_lossy().into_owned()];
        argv.extend(args);
        let h = run_help(&bin, argv);
        if h.error.is_none() && !h.flags.is_empty() {
            return h;
        }
        if h.error.is_none() && first_ok.is_none() {
            first_ok = Some(h.clone());
        }
        last = h;
    }
    first_ok.unwrap_or(last)
}

/// Один запуск справки из папки данных Пульта, с кэшем по командной строке.
fn run_help(bin: &str, argv: Vec<String>) -> Help {
    let key = argv.join(" ");
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(h) = cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
        return h;
    }
    let h = match runner::capture_env(&argv, &paths::data_dir(), Duration::from_secs(10), &[("NO_COLOR", "1"), ("FORCE_COLOR", "0")]) {
        Ok(c) => {
            let ansi = crate::re!(r"\x1b\[[0-9;?]*[A-Za-z]");
            let text = ansi.replace_all(&format!("{}{}", c.out, if c.out.trim().is_empty() { &c.err } else { "" }), "").into_owned();
            let (usage, about, flags) = parse(&text);
            Help {
                program: bin.to_string(),
                argv,
                usage,
                about,
                flags,
                text: text.chars().take(60_000).collect(),
                error: c.timed_out.then(|| "Программа не ответила за 10 с".to_string()),
                confirm: None,
            }
        }
        Err(e) => Help { program: bin.to_string(), argv, error: Some(e.to_string()), ..Default::default() },
    };
    if h.error.is_none() {
        if let Ok(mut c) = cache.lock() {
            c.insert(key, h.clone());
        }
    }
    h
}
