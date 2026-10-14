//! Движок объяснений. Знаний в коде нет: что делает программа, её подкоманды и ключи -
//! из knowledge/tools/*.json. Код только разбирает команду и применяет правила.
//! Если программы нет в базе - описание берётся из её пакета в node_modules,
//! а у файлов-скриптов - из шапки-комментария.
use crate::kb::{Desc, Kb, Tool};
use crate::model::{FlagInfo, Info, Part, Script};
use crate::util::{cap, inside, is_unc, lower_first, mtime_ms, read_head, resolve_inside};
use regex::Regex;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub struct Ctx<'a> {
    pub root: &'a Path,
    pub kb: &'a Kb,
    /// where -> (имя скрипта -> текст команды) для package.json
    pub pkg_scripts: &'a HashMap<String, HashMap<String, String>>,
    pub depth: u8,
}

// ---------- разбор строки ----------
/// Цепочка «a && b | c ; d» -> сегменты (кавычки уважаются).
pub fn split_segments(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut q: Option<char> = None;
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(qc) = q {
            cur.push(c);
            if c == qc {
                q = None;
            }
            i += 1;
            continue;
        }
        if c == '"' || c == '\'' {
            q = Some(c);
            cur.push(c);
        } else if (c == '&' || c == '|') && chars.get(i + 1) == Some(&c) {
            out.push(std::mem::take(&mut cur));
            i += 1;
        } else if c == ';' || c == '&' || c == '|' {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
        i += 1;
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// Слова команды с учётом кавычек. Обратная косая - разделитель папок Windows
/// (`tools\build.ps1`, `D:\app\gradlew.bat`), а не экранирование, как в sh: она остаётся в слове.
/// Экранирует она только кавычку и пробел (`\"`, `\ `); `\` в конце строки - перенос, его нет.
pub fn tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut started = false;
    let mut q: Option<char> = None;
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match (q, c) {
            (Some(qc), _) if c == qc => q = None,
            (Some('"'), '\\') if it.peek() == Some(&'"') => cur.push(it.next().unwrap_or('"')),
            (Some(_), _) => cur.push(c),
            (None, '"' | '\'') => {
                q = Some(c);
                started = true;
            }
            (None, '\\') => match it.peek() {
                Some(&n) if n == '"' || n == '\'' || n == ' ' => {
                    cur.push(n);
                    it.next();
                    started = true;
                }
                Some('\n') | None => {}
                Some(_) => {
                    cur.push(c);
                    started = true;
                }
            },
            (None, c) if c.is_whitespace() => {
                if started {
                    out.push(std::mem::take(&mut cur));
                    started = false;
                }
            }
            (None, c) => {
                cur.push(c);
                started = true;
            }
        }
    }
    if started {
        out.push(cur);
    }
    out
}

fn normalize(seg: &str, kb: &Kb) -> String {
    let mut s = seg.trim().to_string();
    for _ in 0..4 {
        let before = s.clone();
        for r in &kb.wrappers {
            s = r.replace(&s, "").into_owned();
        }
        if s == before {
            break;
        }
    }
    s
}

/// Слово команды для поиска в базе: имя файла без пути и расширения, строчными.
pub fn word_of(tok: &str) -> String {
    let base = tok.rsplit(['/', '\\']).next().unwrap_or(tok).to_lowercase();
    for ext in [".exe", ".cmd", ".bat", ".ps1"] {
        if let Some(b) = base.strip_suffix(ext) {
            return b.to_string();
        }
    }
    base
}

struct Ref {
    name: String,
    recursive: bool,
    filter: Option<String>,
    dir: Option<String>,
}
const PM_BUILTIN: [&str; 40] = [
    "install",
    "i",
    "ci",
    "add",
    "remove",
    "rm",
    "uninstall",
    "update",
    "up",
    "upgrade",
    "exec",
    "dlx",
    "run",
    "publish",
    "link",
    "why",
    "outdated",
    "audit",
    "list",
    "ls",
    "store",
    "init",
    "create",
    "prune",
    "rebuild",
    "import",
    "deploy",
    "pack",
    "setup",
    "env",
    "licenses",
    "dedupe",
    "config",
    "cache",
    "version",
    "help",
    "login",
    "logout",
    "patch",
    "fetch",
];

/// «pnpm run x», «npm run x», «yarn x», «pnpm --filter web build» -> ссылка на скрипт.
fn script_ref(toks: &[String]) -> Option<Ref> {
    let pm = word_of(toks.first()?);
    if !["npm", "pnpm", "yarn", "bun"].contains(&pm.as_str()) {
        return None;
    }
    let (mut recursive, mut filter, mut dir) = (false, None, None);
    let mut i = 1;
    let mut saw_run = false;
    while i < toks.len() {
        let t = toks[i].as_str();
        match t {
            "-r" | "--recursive" => recursive = true,
            "--filter" | "-F" => {
                filter = toks.get(i + 1).cloned();
                i += 1;
            }
            "-C" | "--dir" | "--prefix" => {
                dir = toks.get(i + 1).cloned();
                i += 1;
            }
            "-w" | "--workspace" => {
                filter = toks.get(i + 1).cloned();
                i += 1;
            }
            "--parallel" | "--if-present" | "--silent" | "-s" | "--stream" => {}
            "run" | "run-script" => saw_run = true,
            "--" => {}
            _ if t.starts_with('-') => {}
            _ => {
                if !saw_run && (pm == "npm" && !["test", "start", "stop", "restart"].contains(&t)) {
                    return None;
                }
                if !saw_run && PM_BUILTIN.contains(&t) {
                    return None;
                }
                return Some(Ref { name: t.to_string(), recursive, filter, dir });
            }
        }
        i += 1;
    }
    None
}

// ---------- файлы-скрипты ----------
#[derive(Clone, Default)]
pub struct FileInfo {
    pub header: String,
    pub flags: Vec<String>,
    pub fx: Vec<String>,
    pub long: bool,
}

static FILE_CACHE: OnceLock<Mutex<HashMap<PathBuf, (u64, FileInfo)>>> = OnceLock::new();

fn clean(t: &str) -> String {
    let bold = crate::re!(r"\*\*([^*]+)\*\*");
    let tick = crate::re!(r"`([^`]+)`");
    let t = tick.replace_all(&bold.replace_all(t, "$1"), "«$1»").replace('\r', "");
    let lines: Vec<&str> = t.lines().map(str::trim).collect();
    let mut s = lines.join("\n");
    while s.contains("\n\n\n") {
        s = s.replace("\n\n\n", "\n\n");
    }
    let s = s.trim().replace(['\u{2014}', '\u{2013}'], "-");
    if s.chars().count() > 700 {
        let cut: String = s.chars().take(697).collect();
        format!("{}…", cut.rsplit_once(' ').map(|x| x.0).unwrap_or(&cut))
    } else {
        s
    }
}

fn header_of(text: &str, ext: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut i = usize::from(lines.first().is_some_and(|l| l.starts_with("#!")));
    let head6: String = text.chars().take(6000).collect();
    if ext == "ps1" {
        if let Some(c) = crate::re!(r"(?s)<#(.*?)#>").captures(&head6) {
            let body = &c[1];
            let grab = |name: &str| {
                crate::util::cached_re(&format!(r"(?is)\.{name}\s*\n(.*?)(?:\n\s*\.[A-Z]|\z)"))
                    .and_then(|r| r.captures(body).map(|m| m[1].trim().to_string()))
            };
            let t = [grab("SYNOPSIS"), grab("DESCRIPTION")].into_iter().flatten().collect::<Vec<_>>().join("\n");
            let t = if t.is_empty() && !crate::re!(r"\.[A-Z]{4,}").is_match(body) { body.trim().to_string() } else { t };
            if !t.is_empty() {
                return clean(&t);
            }
        }
    }
    if ext == "py" {
        if let Some(c) = crate::re!(r#"(?s)^\s*(?:#[^\n]*\n)*\s*("""|''')(.*?)("""|''')"#).captures(&head6) {
            return clean(&c[2]);
        }
    }
    let js = ["js", "mjs", "cjs", "ts", "mts", "cts"].contains(&ext);
    if js {
        let rest = lines[i.min(lines.len())..].join("\n");
        if let Some(c) = crate::re!(r"(?s)^\s*/\*\*?(.*?)\*/").captures(&rest) {
            let body = crate::re!(r"(?m)^\s*\*\s?").replace_all(&c[1], "");
            return clean(&body);
        }
    }
    let prefix = if ext == "bat" || ext == "cmd" {
        crate::re!(r"(?i)^\s*(?:@?rem\b|::)\s?(.*)$")
    } else if js {
        crate::re!(r"^\s*//\s?(.*)$")
    } else {
        crate::re!(r"^\s*#\s?(.*)$")
    };
    let skip = crate::re!(r"(?i)^\s*(@?echo\s+off|set\s+-|setlocal|#requires|import\s|'use strict'|\x22use strict\x22)");
    let noise = crate::re!(r"eslint-disable|@ts-|prettier-ignore|shellcheck|^[-=_*#\s]*$");
    let mut out: Vec<String> = Vec::new();
    while i < lines.len() && i < 60 {
        let l = lines[i];
        i += 1;
        if l.trim().is_empty() {
            if !out.is_empty() {
                break;
            }
            continue;
        }
        if skip.is_match(l) {
            if !out.is_empty() {
                break;
            }
            continue;
        }
        let Some(m) = prefix.captures(l) else { break };
        if noise.is_match(&m[1]) {
            continue;
        }
        out.push(m[1].to_string());
        if out.join(" ").chars().count() > 700 {
            break;
        }
    }
    clean(&out.join("\n"))
}

/// Первая фраза шапки - для краткого описания. Фраза в комментарии часто переносится на
/// следующую строку («...держатель,» / «Docker Engine и раннер.»): строки первого абзаца
/// склеиваются до конца первого предложения. Пункт списка начинает новую мысль - на нём стоп.
pub fn lead(header: &str) -> String {
    let mut text = String::new();
    let mut prev_len = 0;
    for l in header.lines().map(str::trim) {
        if !text.is_empty() {
            // следующая строка - продолжение, если прежняя оборвалась на запятой и т.п., была
            // длинной (перенос по ширине комментария) или эта начинается не с заглавной буквы;
            // короткая строка без точки и заглавная следом - это заголовок и новая мысль
            let open = crate::re!(r"[,;:(\-]$").is_match(&text);
            let wrapped = prev_len >= 60;
            let upper = l.chars().next().is_some_and(char::is_uppercase);
            if l.is_empty() || crate::re!(r"^(?:[-*•]\s|\d+[.)]\s)").is_match(l) || !(open || wrapped || !upper) {
                break;
            }
            text.push(' ');
        }
        prev_len = l.chars().count();
        text.push_str(l);
        if crate::re!(r"[.!?…]$").is_match(l) || text.chars().count() > 220 {
            break;
        }
    }
    // конец предложения: знак, пробел и заглавная буква (не «т.е. в», не «ci-off.ps1 и»)
    if let Some(m) = crate::re!(r"[.!?…]\s+\p{Lu}").find(&text) {
        text.truncate(m.start() + text[m.start()..].chars().next().map_or(1, char::len_utf8));
    }
    if text.chars().count() > 220 {
        let cut: String = text.chars().take(219).collect();
        text = format!("{}…", cut.rsplit_once(' ').map_or(cut.as_str(), |x| x.0).trim_end_matches([',', ';', ':']));
    }
    text
}

fn flags_of(text: &str, ext: &str, kb: &Kb) -> Vec<String> {
    let mut set: BTreeSet<String> = BTreeSet::new();
    let ff = &kb.heur["fileFlags"];
    if ext == "ps1" {
        if let Some(c) = crate::re!(r"(?is)param\s*\((.*?)\)\s*(?:\n|$)").captures(&text.chars().take(8000).collect::<String>()) {
            for m in crate::re!(r"\$(\w+)").captures_iter(&c[1]) {
                set.insert(format!("-{}", &m[1]));
            }
        }
    } else {
        let key = match ext {
            "py" => ".py",
            "sh" | "bash" => ".sh",
            _ => ".js",
        };
        let pats: Vec<Regex> = ff[key]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).filter_map(|p| crate::util::cached_re(&format!("(?m){p}"))).collect())
            .unwrap_or_default();
        let only_lines =
            (key == ".js").then(|| ff["linesWith"].as_str().and_then(|p| crate::util::cached_re(&format!("(?i){p}")))).flatten();
        for line in text.lines() {
            if let Some(o) = &only_lines {
                if !o.is_match(line) {
                    continue;
                }
            }
            for p in &pats {
                for m in p.captures_iter(line) {
                    set.insert(m[1].to_string());
                }
            }
        }
    }
    set.into_iter().take(16).collect()
}

pub(crate) fn analyze_file(abs: &Path, kb: &Kb) -> Option<FileInfo> {
    let m = mtime_ms(abs);
    if m == 0 {
        return None;
    }
    let cache = FILE_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some((t, fi)) = cache.lock().ok().and_then(|c| c.get(abs).cloned()) {
        if t == m {
            return Some(fi);
        }
    }
    let text = read_head(abs, 16 * 1024)?;
    let ext = abs.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut fx = Vec::new();
    let mut long = false;
    for (k, r) in &kb.file_fx {
        if r.is_match(&text) {
            if k == "long" {
                long = true;
            } else {
                fx.push(k.clone());
            }
        }
    }
    let fi = FileInfo { header: header_of(&text, &ext), flags: flags_of(&text, &ext, kb), fx, long };
    if let Ok(mut c) = cache.lock() {
        c.insert(abs.to_path_buf(), (m, fi.clone()));
    }
    Some(fi)
}

const FILE_EXT: [&str; 12] = ["js", "mjs", "cjs", "ts", "mts", "cts", "py", "sh", "bash", "ps1", "bat", "cmd"];
fn is_script_path(t: &str) -> bool {
    let low = t.to_lowercase();
    FILE_EXT.iter().any(|e| low.ends_with(&format!(".{e}")))
}

/// Безопасно разрешить путь файла из команды: внутри проекта и не сетевой, в том числе после
/// разрешения ссылок (ссылка из проекта на \\host\share или за его пределы не читается).
fn safe_file(tok: &str, cwd: &Path, root: &Path) -> Option<PathBuf> {
    if is_unc(tok) {
        return None;
    }
    let p = Path::new(tok.trim_matches(['"', '\'']));
    let abs = if p.is_absolute() { p.to_path_buf() } else { cwd.join(p) };
    if !inside(root, &abs) {
        return None;
    }
    resolve_inside(root, &abs).filter(|a| a.is_file())
}

/// Описание незнакомой программы - из её пакета в node_modules (офлайн, без базы знаний).
fn package_about(bin: &str, cwd: &Path, root: &Path) -> Option<(String, String)> {
    let pkg_re = crate::re!(r#"(?:\.\.[\\/]|node_modules[\\/])((?:@[^\\/"'\s]+[\\/])?[^\\/"'\s.][^\\/"'\s]*)[\\/]"#);
    let mut dir = Some(cwd.to_path_buf());
    while let Some(d) = dir {
        if !inside(root, &d) {
            break;
        }
        let nm = d.join("node_modules");
        let bindir = nm.join(".bin");
        for shim in [bindir.join(format!("{bin}.cmd")), bindir.join(bin)] {
            if let Some(text) = read_head(&shim, 4096) {
                let pkg = pkg_re
                    .captures_iter(&text)
                    .map(|c| c[1].replace('\\', "/"))
                    .find(|p| p != ".bin" && nm.join(p).join("package.json").is_file());
                if let Some(pkg) = pkg {
                    let j: serde_json::Value = serde_json::from_str(&read_head(&nm.join(&pkg).join("package.json"), 200_000)?).ok()?;
                    let desc = j["description"].as_str().unwrap_or("").trim().to_string();
                    return Some((pkg, desc));
                }
            }
        }
        if nm.join(bin).join("package.json").is_file() {
            let j: serde_json::Value = serde_json::from_str(&read_head(&nm.join(bin).join("package.json"), 200_000)?).ok()?;
            return Some((bin.to_string(), j["description"].as_str().unwrap_or("").trim().to_string()));
        }
        dir = d.parent().map(Path::to_path_buf);
    }
    None
}

// ---------- применение правил программы ----------
struct Applied {
    say: String,
    fx: BTreeSet<String>,
    long: bool,
    url: bool,
    danger: bool,
    parts: Vec<Part>,
    flags: Vec<FlagInfo>,
    how: Vec<String>,
}

fn merge(base: &Desc, over: &Desc) -> Desc {
    Desc {
        say: over.say.clone().or_else(|| base.say.clone()),
        part: over.part.clone(),
        fx: if over.fx.is_empty() { base.fx.clone() } else { over.fx.clone() },
        fx_remove: over.fx_remove.clone(),
        // подкоманда сама решает, долгая ли она и опасна ли: «next» долгий, «next build» - нет
        long: over.long,
        url: over.url,
        danger: over.danger,
        how: if over.how.is_empty() { base.how.clone() } else { over.how.clone() },
    }
}

fn flag_hit(rule_flag: &str, aliases: &[String], tok: &str) -> bool {
    tok == rule_flag || aliases.iter().any(|a| a == tok) || (rule_flag.starts_with("--") && tok.starts_with(&format!("{rule_flag}=")))
}

fn apply(tool: &Tool, toks: &[String], n: usize) -> Applied {
    let rest_toks = &toks[n..];
    let rest = rest_toks.join(" ");
    let mut d = tool.default.clone();
    let mut sub_hit = None;
    for s in &tool.sub {
        if s.re().is_some_and(|r| r.is_match(&rest)) {
            d = merge(&tool.default, &s.d);
            sub_hit = Some(s);
            break;
        }
    }
    let mut say = d.say.clone().unwrap_or_else(|| tool.name.clone());
    let mut fx: BTreeSet<String> = d.fx.iter().cloned().collect();
    for r in &d.fx_remove {
        fx.remove(r);
    }
    let mut long = d.long.unwrap_or(false);
    let url = d.url.unwrap_or(false);
    let mut danger = d.danger.unwrap_or(false);
    let mut parts = vec![Part {
        text: toks[..n].join(" "),
        note: if tool.about.is_empty() { tool.name.clone() } else { tool.about.clone() },
        kind: "bin".into(),
    }];
    // какие слова «съела» подкоманда: все не-ключи до первого ключа, которые входят в совпадение
    let sub_words = match sub_hit {
        Some(s) => s
            .re()
            .and_then(|r| r.find_end(&rest))
            .map(|end| rest[..end].split_whitespace().filter(|w| !w.starts_with('-')).count())
            .unwrap_or(0),
        None => 0,
    };
    let mut used = vec![false; tool.flags.len()];
    let mut said_sub = false;
    for (k, t) in rest_toks.iter().enumerate() {
        if let Some((fi, f)) = tool.flags.iter().enumerate().find(|(_, f)| flag_hit(&f.flag, &f.alias, t)) {
            used[fi] = true;
            parts.push(Part { text: t.clone(), note: f.text.clone(), kind: "flag".into() });
            if let Some(s) = &f.d.say {
                if !say.contains(s.as_str()) {
                    say = if s.starts_with(',') { format!("{say}{s}") } else { format!("{say} {s}") };
                }
            }
            fx.extend(f.d.fx.iter().cloned());
            for r in &f.d.fx_remove {
                fx.remove(r);
            }
            if let Some(l) = f.d.long {
                long = l;
            }
            if f.d.danger == Some(true) {
                danger = true;
            }
            continue;
        }
        if k < sub_words && !t.starts_with('-') {
            if !said_sub {
                let note = sub_hit.and_then(|s| s.d.part.clone().or(s.d.say.clone())).unwrap_or_default();
                parts.push(Part { text: t.clone(), note, kind: "sub".into() });
                said_sub = true;
            } else if let Some(last) = parts.last_mut() {
                last.text.push(' ');
                last.text.push_str(t);
            }
            continue;
        }
        let kind = if is_script_path(t) {
            "file"
        } else if t.starts_with('-') {
            "flag"
        } else {
            "arg"
        };
        parts.push(Part { text: t.clone(), note: String::new(), kind: kind.into() });
    }
    let flags = tool
        .flags
        .iter()
        .zip(&used)
        .map(|(f, u)| FlagInfo { flag: f.flag.clone(), text: f.text.clone(), used: *u, from: "kb".into() })
        .collect();
    let how = if d.how.is_empty() { tool.how.clone() } else { d.how.clone() };
    Applied { say, fx, long, url, danger, parts, flags, how }
}

// ---------- главный вход ----------
pub fn explain(s: &Script, cwd: &Path, ctx: &Ctx) -> Info {
    let kb = ctx.kb;
    let mut info = Info::default();
    let mut fx: BTreeSet<String> = BTreeSet::new();
    let mut danger = false;

    // хуки npm: preinstall, postbuild, prepare
    if s.source == "package.json" {
        let siblings = ctx.pkg_scripts.get(&s.where_);
        for (p, word) in [("pre", "перед"), ("post", "после")] {
            if let Some(base) = s.name.strip_prefix(p) {
                if siblings.is_some_and(|m| m.contains_key(base)) {
                    info.hook = Some(format!("{word} «{base}»"));
                }
            }
        }
        if ["prepare", "postinstall", "preinstall", "install"].contains(&s.name.as_str()) {
            info.hook = Some("при установке зависимостей".into());
        }
    }

    let mut says: Vec<String> = Vec::new();
    let add_file = |abs: &Path, info: &mut Info, fx: &mut BTreeSet<String>| -> Option<FileInfo> {
        let fi = analyze_file(abs, kb)?;
        info.file = Some(crate::util::rel(ctx.root, abs));
        if info.header.is_empty() {
            info.header = fi.header.clone();
        }
        for f in &fi.flags {
            if !info.flags.iter().any(|x| &x.flag == f) {
                info.flags.push(FlagInfo { flag: f.clone(), text: String::new(), used: false, from: "file".into() });
            }
        }
        if fi.fx.iter().any(|x| x != "long") {
            info.fx_from_file = true;
        }
        fx.extend(fi.fx.iter().cloned());
        if fi.long {
            info.long = true;
        }
        Some(fi)
    };

    let text = if s.raw.is_empty() { s.cmd.clone() } else { s.raw.join(" && ") };
    let segs: Vec<String> = split_segments(&text).into_iter().take(8).collect();

    for seg in &segs {
        let norm = normalize(seg, kb);
        if kb.skip_seg.as_ref().is_some_and(|r| r.is_match(&norm)) {
            continue;
        }
        let toks = tokens(&norm);
        if toks.is_empty() {
            continue;
        }
        // ссылка на другой npm-скрипт
        if s.source == "package.json" || ctx.depth > 0 {
            if let Some(r) = script_ref(&toks) {
                info.refs.push(r.name.clone());
                if r.recursive {
                    says.push(format!("запускает «{}» во всех пакетах", r.name));
                    continue;
                }
                if let Some(f) = &r.filter {
                    says.push(format!("запускает «{}» в пакете {f}", r.name));
                    continue;
                }
                let where_ = r.dir.clone().unwrap_or_else(|| s.where_.clone());
                let target = ctx.pkg_scripts.get(&where_).and_then(|m| m.get(&r.name)).cloned();
                match target {
                    Some(t) if ctx.depth < 3 && r.name != s.name => {
                        let sub = Script { name: r.name.clone(), raw: vec![t.clone()], cmd: t, file_rel: None, ..s.clone() };
                        let si = explain(&sub, cwd, &Ctx { depth: ctx.depth + 1, ..*ctx });
                        says.push(if ctx.depth > 1 {
                            format!("скрипт «{}»", r.name)
                        } else {
                            format!("«{}»: {}", r.name, lower_first(&si.summary))
                        });
                        fx.extend(si.fx.iter().cloned());
                        info.long |= si.long;
                        info.url |= si.url;
                        info.known |= si.known;
                        for t in si.tools {
                            if !info.tools.contains(&t) {
                                info.tools.push(t);
                            }
                        }
                        for t in si.topics {
                            if !info.topics.contains(&t) {
                                info.topics.push(t);
                            }
                        }
                        if info.how.is_empty() {
                            info.how = si.how;
                        }
                        if info.header.is_empty() && !si.header.is_empty() {
                            info.header = si.header;
                            info.file = si.file;
                            info.flags = si.flags;
                        }
                        if info.about.is_empty() {
                            info.about = si.about;
                        }
                    }
                    _ => says.push(format!("запускает скрипт «{}»", r.name)),
                }
                continue;
            }
        }
        let words: Vec<String> = toks.iter().enumerate().map(|(i, t)| if i == 0 { word_of(t) } else { t.to_lowercase() }).collect();
        if let Some((ti, n)) = kb.match_tool(&words) {
            let tool = &kb.tools[ti];
            let a = apply(tool, &toks, n);
            let mut say = a.say.clone();
            if tool.runs_file {
                if let Some(abs) = toks[n..].iter().filter(|t| is_script_path(t)).find_map(|t| safe_file(t, cwd, ctx.root)) {
                    if let Some(fi) = add_file(&abs, &mut info, &mut fx) {
                        if !fi.header.is_empty() {
                            say = lead(&fi.header);
                        }
                        info.source = "file".into();
                    }
                }
            }
            says.push(say);
            fx.extend(a.fx);
            danger |= a.danger;
            info.long |= a.long;
            info.url |= a.url;
            info.known = true;
            if !info.tools.contains(&tool.id) {
                info.tools.push(tool.id.clone());
            }
            if !tool.topic.is_empty() && !info.topics.contains(&tool.topic) {
                info.topics.push(tool.topic.clone());
            }
            if info.about.is_empty() {
                info.about = tool.about.clone();
            }
            if !a.how.is_empty() && info.how.len() < 6 {
                for h in &a.how {
                    if info.how.len() < 6 && !info.how.contains(h) {
                        info.how.push(h.clone());
                    }
                }
            }
            if !info.parts.is_empty() {
                info.parts.push(Part { text: "&&".into(), note: "затем".into(), kind: "join".into() });
            }
            info.parts.extend(a.parts);
            for f in a.flags {
                if !info.flags.iter().any(|x| x.flag == f.flag) {
                    info.flags.push(f);
                }
            }
            if info.source.is_empty() {
                info.source = "kb".into();
            }
            continue;
        }
        // незнакомая программа: может, это файл-скрипт
        if is_script_path(&toks[0]) {
            if let Some(abs) = safe_file(&toks[0], cwd, ctx.root) {
                let fi = add_file(&abs, &mut info, &mut fx);
                says.push(fi.map(|f| lead(&f.header)).filter(|h| !h.is_empty()).unwrap_or_else(|| format!("выполняет {}", toks[0])));
                info.source = "file".into();
                info.known |= !info.header.is_empty();
                continue;
            }
        }
        // ... или пакет из node_modules
        let bin = word_of(&toks[0]);
        if let Some((pkg, desc)) = package_about(&bin, cwd, ctx.root) {
            says.push(format!("запускает {bin}"));
            if info.about.is_empty() {
                info.about =
                    if desc.is_empty() { format!("Пакет npm «{pkg}»") } else { format!("Пакет npm «{pkg}»: {desc}") };
            }
            info.parts.push(Part { text: toks[0].clone(), note: info.about.clone(), kind: "bin".into() });
            if info.source.is_empty() {
                info.source = "package".into();
            }
            continue;
        }
        says.push(format!("выполняет «{}»", toks.iter().take(3).cloned().collect::<Vec<_>>().join(" ")));
        info.parts.push(Part { text: toks[0].clone(), note: String::new(), kind: "bin".into() });
    }

    // файл-скрипт, найденный в папке tools/scripts
    if let Some(fr) = &s.file_rel {
        let abs = ctx.root.join(fr);
        if let Some(fi) = add_file(&abs, &mut info, &mut fx) {
            let h = lead(&fi.header);
            if !h.is_empty() {
                says = vec![h];
                info.known = true;
            }
            info.source = "file".into();
        }
    }

    info.summary = match says.len() {
        0 => String::new(),
        1 => cap(&says[0]),
        _ => {
            let joined = format!("По очереди: {}", says.iter().map(|x| lower_first(x)).collect::<Vec<_>>().join(", затем "));
            if joined.chars().count() > 160 {
                info.steps = says.iter().map(|x| cap(x)).collect();
            }
            joined
        }
    };
    // Makefile / justfile / Taskfile: комментарий автора главнее разбора рецепта
    if !s.desc.is_empty() && s.source != "package.json" && !s.virtual_ && s.file_rel.is_none() && s.desc != info.summary {
        if !info.summary.is_empty() && info.steps.is_empty() {
            info.steps = vec![format!("Внутри: {}", lower_first(&info.summary))];
        }
        info.summary = cap(&s.desc);
        info.known = true;
    }
    if info.summary.is_empty() {
        info.summary = "Что делает - не понял. Посмотрите команду ниже".into();
    }
    if danger {
        fx.insert("danger".into());
    }
    if fx.contains("write") || fx.contains("delete") || fx.contains("install") || fx.contains("db") {
        fx.remove("read");
    }
    if fx.is_empty() && info.known {
        fx.insert("read".into());
        info.fx_default = true;
    }
    fx.remove("long");
    info.fx = fx.into_iter().collect();
    info
}
