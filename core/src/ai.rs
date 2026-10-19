//! ИИ-часть проекта: что в папке настроено для работы с ИИ-агентом и как это связано.
//!
//! Код умеет читать несколько форматов (markdown с шапкой, JSON настроек, скрипты, джобы CI), а какой
//! файл какой деталью считается и что о детали сказать простыми словами, решает `knowledge/ai.json`.
//! Новый вид детали - запись в этом файле, без правки кода.
//!
//! Что читается, а что нет. Ничего из проекта не запускается: ни хук, ни MCP-сервер, ни скрипт. `.env*`
//! не читаются. Ссылки (symlink, junction) пропускаются, сетевые пути не открываются. Значения env и
//! похожие на секрет части команд заменяются на «•••»; из личных настроек (`private`) выводятся только
//! числа. Связи между деталями берутся только из данных: упоминание в обратных кавычках, ссылка
//! markdown, путь к скрипту в команде хука. Угадывания нет.
use crate::kb::Kb;
use crate::model::Project;
use crate::util::{is_link, key_of, ls, mtime_ms, now_ms, read_head};
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::time::Instant;

// ---------------------------------------------------------------------------------------------
// Настройки из knowledge/ai.json
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
struct Limits {
    file_bytes: usize,
    walk_files: usize,
    items_per_kind: usize,
    summary: usize,
    desc: usize,
    body_scan: usize,
    hits: usize,
    mention_min: usize,
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
struct SecretCfg {
    key: String,
    value: String,
    inline: String,
    userinfo: String,
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
struct ModeRule {
    key: String,
    equals: Option<String>,
    mode: String,
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
struct Detect {
    glob: String,
    eco: String,
    mode: String,
    modes: Vec<ModeRule>,
    /// формат разбора файла; пусто - формат вида
    format: String,
    role: String,
    private: bool,
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
struct ParseCfg {
    name: Vec<String>,
    /// откуда имя, если в шапке его нет: file (без расширения), filename, dir (папка файла), path
    name_from: String,
    desc: Vec<String>,
    paths: Vec<String>,
    tools: Vec<String>,
    model: Vec<String>,
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
struct ScriptsCfg {
    stages: Vec<String>,
    also_re: String,
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
struct SignalDef {
    id: String,
    title: String,
    say: String,
    dir: String,
    file: String,
    scan: String,
    count_dirs: bool,
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
struct ModeDef {
    label: String,
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
struct KindDef {
    id: String,
    title: String,
    format: String,
    detect: Vec<Detect>,
    parse: ParseCfg,
    refs: Vec<String>,
    modes: BTreeMap<String, ModeDef>,
    events: Vec<String>,
    scripts: ScriptsCfg,
    ci: bool,
    meta_re: String,
    measure_re: String,
    signals: Vec<SignalDef>,
    scan_kinds: Vec<String>,
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
struct Cond {
    kind: String,
    mode: String,
    role: String,
    #[serde(rename = "match")]
    pat: String,
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
struct StepDef {
    id: String,
    title: String,
    say: String,
    any: Vec<Cond>,
    always: bool,
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
struct Cfg {
    limits: Limits,
    walk_skip: Vec<String>,
    markers: Vec<String>,
    secrets: SecretCfg,
    facts: BTreeMap<String, String>,
    story_more: String,
    ignore_files: Vec<String>,
    kinds: Vec<KindDef>,
    story: Vec<StepDef>,
}

impl Cfg {
    fn load(kb: &Kb) -> Cfg {
        let mut c: Cfg = serde_json::from_value(kb.ai.clone()).unwrap_or_default();
        let l = &mut c.limits;
        for (v, d) in [
            (&mut l.file_bytes, 262_144),
            (&mut l.walk_files, 80_000),
            (&mut l.items_per_kind, 400),
            (&mut l.summary, 240),
            (&mut l.desc, 1200),
            (&mut l.body_scan, 65_536),
            (&mut l.hits, 6),
            (&mut l.mention_min, 4),
        ] {
            if *v == 0 {
                *v = d;
            }
        }
        c
    }
}

// ---------------------------------------------------------------------------------------------
// Результат
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AiGlob {
    pub glob: String,
    /// сколько файлов проекта подходит под шаблон
    pub files: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AiFact {
    pub k: String,
    pub v: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AiList {
    pub k: String,
    pub items: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AiEnv {
    pub key: String,
    /// пусто, если значение скрыто
    pub value: String,
    pub masked: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AiHit {
    pub file: String,
    pub line: u32,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AiItem {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    pub eco: String,
    pub mode: String,
    pub role: String,
    pub summary: String,
    pub desc: String,
    pub size: u64,
    pub mtime: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<AiGlob>,
    /// сколько файлов проекта подходит хотя бы под один шаблон путей
    pub files_matched: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// файл-скрипт, на который указывает команда хука
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<AiEnv>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub facts: Vec<AiFact>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lists: Vec<AiList>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hits: Vec<AiHit>,
    /// на кого ссылается эта деталь (id)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub refs_out: Vec<String>,
    /// кто ссылается на эту деталь (id)
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub refs_in: Vec<String>,
    /// хук-скрипт лежит в папке хуков, но ни одна запись настроек его не запускает
    pub unwired: bool,
    /// git видит эту папку хуков (core.hooksPath); None - не известно или не про git
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub broken: Option<String>,
    pub private: bool,
    /// файл длиннее потолка чтения: разобрано начало
    pub truncated: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AiKindStat {
    pub id: String,
    pub count: u32,
    /// сколько деталей в каждом режиме загрузки (always, paths...)
    pub modes: BTreeMap<String, u32>,
    pub roles: BTreeMap<String, u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AiKindEdge {
    pub from: String,
    pub to: String,
    pub n: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AiStep {
    pub id: String,
    pub title: String,
    pub text: String,
    pub parts: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AiProblem {
    pub file: String,
    pub error: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AiMap {
    pub path: String,
    /// отпечаток содержимого: меняется, когда поменялась любая деталь
    pub fp: String,
    pub built_at: u64,
    pub ms: u64,
    pub items: Vec<AiItem>,
    pub kinds: Vec<AiKindStat>,
    pub kind_edges: Vec<AiKindEdge>,
    pub story: Vec<AiStep>,
    pub problems: Vec<AiProblem>,
    /// сколько файлов проекта обошли для подсчёта шаблонов путей
    pub walked: u32,
    /// обход остановился на потолке: числа «файлов под шаблон» не меньше показанных
    pub capped: bool,
    /// значение core.hooksPath из .git/config; None - не задано
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hooks_path: Option<String>,
    #[serde(default)]
    pub cached: bool,
}

// ---------------------------------------------------------------------------------------------
// Шаблоны путей (glob)
// ---------------------------------------------------------------------------------------------

/// Шаблон пути: `*` - внутри одной папки, `**` - через папки, `?`, `{a,b}`, `[abc]`.
/// Шаблон без «/» ловит файл в любой папке, как в .gitignore. Регистр не важен (Windows).
#[derive(Clone, Debug)]
pub struct Glob {
    re: Regex,
    prefix: String,
    suffix: String,
}

fn glob_body(chars: &[char], i: &mut usize, stop_at_comma: bool) -> String {
    let mut out = String::new();
    while *i < chars.len() {
        let c = chars[*i];
        match c {
            '*' => {
                let mut j = *i;
                while j < chars.len() && chars[j] == '*' {
                    j += 1;
                }
                if j - *i >= 2 {
                    if chars.get(j) == Some(&'/') {
                        out.push_str("(?:.*/)?");
                        *i = j + 1;
                    } else {
                        out.push_str(".*");
                        *i = j;
                    }
                } else {
                    out.push_str("[^/]*");
                    *i = j;
                }
                continue;
            }
            '?' => out.push_str("[^/]"),
            '{' => {
                *i += 1;
                let mut alts = vec![];
                loop {
                    alts.push(glob_body(chars, i, true));
                    match chars.get(*i) {
                        Some(',') => *i += 1,
                        Some('}') => break,
                        _ => break,
                    }
                }
                out.push_str(&format!("(?:{})", alts.join("|")));
            }
            '}' => {
                if stop_at_comma {
                    return out;
                }
                out.push_str("\\}");
            }
            ',' if stop_at_comma => return out,
            '[' => {
                if let Some(end) = chars[*i..].iter().position(|&x| x == ']') {
                    let class: String = chars[*i..*i + end + 1].iter().collect();
                    out.push_str(&class);
                    *i += end;
                } else {
                    out.push_str("\\[");
                }
            }
            _ => out.push_str(&regex::escape(&c.to_string())),
        }
        *i += 1;
    }
    out
}

impl Glob {
    /// Шаблон путей из правил агента: без «/» ловит файл в любой папке, как в .gitignore.
    pub fn new(pattern: &str) -> Option<Glob> {
        Self::make(pattern, false)
    }

    /// Шаблон признака из ai.json: всегда от корня проекта («CLAUDE.md» - только корневой).
    pub fn anchored(pattern: &str) -> Option<Glob> {
        Self::make(pattern, true)
    }

    fn make(pattern: &str, anchored: bool) -> Option<Glob> {
        let p = pattern.trim().replace('\\', "/").to_lowercase();
        let p = p.trim_start_matches("./").trim_start_matches('/').to_string();
        if p.is_empty() || p.starts_with('!') {
            return None;
        }
        let anywhere = !anchored && !p.contains('/');
        let chars: Vec<char> = p.chars().collect();
        let mut i = 0;
        let mut body = glob_body(&chars, &mut i, false);
        if p.ends_with('/') {
            body.push_str(".*");
        }
        let src = format!("^{}{}$", if anywhere { "(?:.*/)?" } else { "" }, body);
        let re = Regex::new(&src).ok()?;
        let meta = |c: char| matches!(c, '*' | '?' | '{' | '}' | '[' | ']');
        let prefix = if anywhere { String::new() } else { p.chars().take_while(|&c| !meta(c)).collect() };
        let suffix: String = {
            let tail: Vec<char> = p.chars().rev().take_while(|&c| !meta(c)).collect();
            tail.into_iter().rev().collect()
        };
        Some(Glob { re, prefix, suffix })
    }

    /// `rel` - путь относительно корня, через «/», строчными буквами.
    pub fn is_match(&self, rel: &str) -> bool {
        (self.prefix.is_empty() || rel.starts_with(&self.prefix))
            && (self.suffix.is_empty() || rel.ends_with(&self.suffix))
            && self.re.is_match(rel)
    }
}

/// Шаблоны, по которым слежение за файлами узнаёт правку ИИ-части (кладётся в `Kb` при загрузке).
pub fn compile_watch(ai: &Value) -> Vec<Glob> {
    let mut out = vec![];
    for k in ai["kinds"].as_array().into_iter().flatten() {
        for d in k["detect"].as_array().into_iter().flatten() {
            if let Some(g) = d["glob"].as_str().and_then(Glob::anchored) {
                out.push(g);
            }
        }
    }
    out
}

/// Что за событие файла для ИИ-части: 1 - появился или исчез признак (папка `.claude`, файл
/// инструкций в корне): перечитать проект; 3 - правка детали: пересобрать карту; 0 - не про ИИ.
pub fn watch_hit(parts: &[String], kb: &Kb) -> u8 {
    if kb.ai_watch.is_empty() || parts.is_empty() {
        return 0;
    }
    let rel = parts.join("/").to_lowercase();
    let under = |s: &str| {
        let s = s.to_lowercase();
        rel == s || rel.starts_with(&format!("{s}/"))
    };
    if kb.ai["walkSkip"].as_array().into_iter().flatten().filter_map(Value::as_str).any(under) {
        return 0;
    }
    if kb.ai["markers"].as_array().into_iter().flatten().filter_map(Value::as_str).any(|m| rel == m.to_lowercase()) {
        return 1;
    }
    if kb.ai_watch.iter().any(|g| g.is_match(&rel)) {
        return 3;
    }
    0
}

/// Какие признаки ИИ-части лежат в корне проекта (по `markers` из ai.json). Дёшево: без разбора файлов.
pub fn markers(root: &Path, in_root: &dyn Fn(&str) -> bool, kb: &Kb) -> Vec<String> {
    kb.ai["markers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|m| {
            if m.contains('/') {
                let p = root.join(m);
                !is_link(&p) && std::fs::metadata(&p).is_ok()
            } else {
                in_root(m)
            }
        })
        .map(str::to_string)
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Шапка markdown (YAML): небольшой терпимый разбор
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Fv {
    Str(String),
    List(Vec<String>),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fm(pub Vec<(String, Fv)>);

impl Fm {
    pub fn get(&self, key: &str) -> Option<&Fv> {
        self.0.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
    }

    /// Первое из ключей, у которого есть значение, одной строкой (список склеивается через запятую).
    pub fn text(&self, keys: &[String]) -> Option<String> {
        keys.iter().find_map(|k| {
            let s = match self.get(k)? {
                Fv::Str(s) => s.trim().to_string(),
                Fv::List(l) => l.join(", "),
            };
            (!s.is_empty()).then_some(s)
        })
    }

    /// Значения первого подходящего ключа списком: `[a, b]`, строки `- a` и «a, b» дают одно и то же.
    pub fn list(&self, keys: &[String]) -> Vec<String> {
        for k in keys {
            let v = match self.get(k) {
                Some(Fv::List(l)) => l.clone(),
                Some(Fv::Str(s)) => split_list(s),
                None => continue,
            };
            let v: Vec<String> = v.into_iter().map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
            if !v.is_empty() {
                return v;
            }
        }
        vec![]
    }

    pub fn has(&self, key: &str) -> bool {
        match self.get(key) {
            Some(Fv::Str(s)) => !s.trim().is_empty(),
            Some(Fv::List(l)) => !l.is_empty(),
            None => false,
        }
    }
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Значение без кавычек; в двойных понимает `\"`, `\\`, `\n`.
fn unquote(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        let inner = &t[1..t.len() - 1];
        let mut out = String::new();
        let mut it = inner.chars();
        while let Some(c) = it.next() {
            if c == '\\' {
                match it.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(o) => out.push(o),
                    None => {}
                }
            } else {
                out.push(c);
            }
        }
        return out;
    }
    if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') {
        return t[1..t.len() - 1].replace("''", "'");
    }
    t.to_string()
}

/// Отрезает комментарий « # ...» у значения без кавычек.
fn strip_comment(s: &str) -> &str {
    let (mut q, mut prev) = (None::<char>, ' ');
    for (i, c) in s.char_indices() {
        match q {
            Some(open) if c == open => q = None,
            Some(_) => {}
            None if (c == '"' || c == '\'') && prev.is_whitespace() || i == 0 && (c == '"' || c == '\'') => q = Some(c),
            // «#160» в середине фразы - текст, а не комментарий: комментарий YAML отделён пробелом с обеих сторон
            None if c == '#' && prev.is_whitespace() && s[i + 1..].chars().next().is_none_or(char::is_whitespace) => {
                return s[..i].trim_end()
            }
            None => {}
        }
        prev = c;
    }
    s
}

/// Делит «a, b, {c,d}» по запятым вне скобок и кавычек.
pub fn split_list(s: &str) -> Vec<String> {
    let t = s.trim();
    let t = t.strip_prefix('[').and_then(|x| x.strip_suffix(']')).unwrap_or(t);
    let (mut out, mut cur, mut depth, mut q) = (vec![], String::new(), 0i32, None::<char>);
    for c in t.chars() {
        match q {
            Some(open) => {
                if c == open {
                    q = None;
                }
                cur.push(c);
            }
            None => match c {
                '"' | '\'' => {
                    q = Some(c);
                    cur.push(c);
                }
                '{' | '[' | '(' => {
                    depth += 1;
                    cur.push(c);
                }
                '}' | ']' | ')' => {
                    depth -= 1;
                    cur.push(c);
                }
                ',' if depth <= 0 => {
                    out.push(unquote(&cur));
                    cur.clear();
                }
                _ => cur.push(c),
            },
        }
    }
    if !cur.trim().is_empty() {
        out.push(unquote(&cur));
    }
    out.into_iter().filter(|x| !x.is_empty()).collect()
}

/// Ключ строки «key: значение»; None - строка не похожа на пару.
fn key_line(line: &str) -> Option<(&str, &str)> {
    let (k, v) = line.split_once(':')?;
    let k = k.trim();
    if k.is_empty() || !k.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.')) {
        return None;
    }
    // «key:value» без пробела после двоеточия - это уже не пара (адрес, время)
    (v.is_empty() || v.starts_with([' ', '\t'])).then_some((k, v.trim()))
}

fn parse_block(lines: &[&str]) -> Fm {
    let mut out: Vec<(String, Fv)> = vec![];
    let n = lines.len();
    let mut i = 0;
    while i < n {
        let line = lines[i];
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') || indent(line) > 0 {
            i += 1;
            continue;
        }
        let Some((key, rest)) = key_line(line) else {
            i += 1;
            continue;
        };
        i += 1;
        let rest = rest.trim();
        // блочная строка: > или |
        if let Some(first) = rest.chars().next().filter(|c| matches!(c, '>' | '|')) {
            let keep = rest.contains('+');
            let mut body: Vec<&str> = vec![];
            while i < n && (lines[i].trim().is_empty() || indent(lines[i]) > 0) {
                body.push(lines[i]);
                i += 1;
            }
            let min = body.iter().filter(|l| !l.trim().is_empty()).map(|l| indent(l)).min().unwrap_or(0);
            let cut: Vec<String> =
                body.iter().map(|l| if l.len() >= min { l[min..].trim_end().to_string() } else { String::new() }).collect();
            let text = if first == '|' {
                cut.join("\n")
            } else {
                let mut paras: Vec<String> = vec![];
                let mut cur: Vec<&str> = vec![];
                for l in &cut {
                    if l.trim().is_empty() {
                        if !cur.is_empty() {
                            paras.push(cur.join(" "));
                            cur.clear();
                        }
                    } else {
                        cur.push(l.trim());
                    }
                }
                if !cur.is_empty() {
                    paras.push(cur.join(" "));
                }
                paras.join("\n")
            };
            let text = if keep { text } else { text.trim_end().to_string() };
            out.push((key.to_string(), Fv::Str(text)));
            continue;
        }
        // пусто после двоеточия: список строк, вложенный словарь или значение на следующих строках
        if rest.is_empty() || rest.starts_with('#') {
            let mut body: Vec<&str> = vec![];
            while i < n && (lines[i].trim().is_empty() || indent(lines[i]) > 0 || lines[i].trim_start().starts_with("- ")) {
                body.push(lines[i]);
                i += 1;
            }
            let body: Vec<&str> = body.into_iter().filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#')).collect();
            if body.first().is_some_and(|l| l.trim_start().starts_with('-')) {
                let mut items: Vec<String> = vec![];
                for l in &body {
                    let tl = l.trim_start();
                    if let Some(item) = tl.strip_prefix('-') {
                        items.push(unquote(strip_comment(item.trim())));
                    } else if let Some(last) = items.last_mut() {
                        last.push(' ');
                        last.push_str(l.trim());
                    }
                }
                out.push((key.to_string(), Fv::List(items)));
            } else if body.iter().any(|l| key_line(l.trim()).is_some()) {
                // вложенный словарь: ключ есть, значение не разбирается
                out.push((key.to_string(), Fv::Str(String::new())));
            } else {
                out.push((key.to_string(), Fv::Str(body.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" "))));
            }
            continue;
        }
        // встроенный список, возможно на нескольких строках
        if rest.starts_with('[') {
            let mut buf = rest.to_string();
            while !buf.contains(']') && i < n {
                buf.push(' ');
                buf.push_str(lines[i].trim());
                i += 1;
            }
            let end = buf.rfind(']').map(|p| p + 1).unwrap_or(buf.len());
            out.push((key.to_string(), Fv::List(split_list(&buf[..end]))));
            continue;
        }
        // строка в кавычках, возможно на нескольких строках
        if rest.starts_with('"') || rest.starts_with('\'') {
            let q = rest.chars().next().unwrap_or('"');
            let mut buf = rest.to_string();
            let closed = |b: &str| {
                let mut esc = false;
                b.chars().skip(1).any(|c| {
                    if esc {
                        esc = false;
                        false
                    } else if c == '\\' && q == '"' {
                        esc = true;
                        false
                    } else {
                        c == q
                    }
                })
            };
            while !closed(&buf) && i < n {
                buf.push(' ');
                buf.push_str(lines[i].trim());
                i += 1;
            }
            // всё после закрывающей кавычки (комментарий) отбрасывается
            let mut esc = false;
            let end = buf
                .char_indices()
                .skip(1)
                .find(|&(_, c)| {
                    if esc {
                        esc = false;
                        false
                    } else if c == '\\' && q == '"' {
                        esc = true;
                        false
                    } else {
                        c == q
                    }
                })
                .map(|(p, c)| p + c.len_utf8())
                .unwrap_or(buf.len());
            out.push((key.to_string(), Fv::Str(unquote(&buf[..end]))));
            continue;
        }
        // обычная строка; следующие строки с отступом - её продолжение
        let mut val = strip_comment(rest).to_string();
        while i < n
            && indent(lines[i]) > 0
            && !lines[i].trim().is_empty()
            && key_line(lines[i].trim()).is_none()
            && !lines[i].trim_start().starts_with('-')
        {
            val.push(' ');
            val.push_str(strip_comment(lines[i].trim()));
            i += 1;
        }
        out.push((key.to_string(), Fv::Str(val)));
    }
    Fm(out)
}

/// Шапка `--- ... ---` в начале файла и остальной текст. Нет шапки или она не закрыта - пустая шапка и весь текст.
pub fn parse_frontmatter(text: &str) -> (Fm, &str) {
    let t = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut it = t.split_inclusive('\n');
    let Some(first) = it.next() else { return (Fm::default(), t) };
    if first.trim_end() != "---" {
        return (Fm::default(), t);
    }
    let mut pos = first.len();
    let mut block: Vec<&str> = vec![];
    for line in it {
        pos += line.len();
        let tl = line.trim_end();
        if tl == "---" || tl == "..." {
            return (parse_block(&block), &t[pos..]);
        }
        block.push(tl);
    }
    (Fm::default(), t)
}

// ---------------------------------------------------------------------------------------------
// JSON с комментариями и лишними запятыми
// ---------------------------------------------------------------------------------------------

/// JSON настроек как его пишут люди: BOM, `//` и `/* */`, запятая перед `}` и `]`.
pub fn parse_json(text: &str) -> Result<Value, String> {
    let t = text.trim_start_matches('\u{feff}');
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return Ok(v);
    }
    let cleaned = strip_json_extras(t);
    serde_json::from_str(&cleaned).map_err(|e| format!("строка {}, столбец {}", e.line(), e.column()))
}

fn strip_json_extras(t: &str) -> String {
    let c: Vec<char> = t.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(c.len());
    let (mut i, mut in_str, mut esc) = (0, false, false);
    while i < c.len() {
        let ch = c[i];
        if in_str {
            out.push(ch);
            if esc {
                esc = false;
            } else if ch == '\\' {
                esc = true;
            } else if ch == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match (ch, c.get(i + 1)) {
            ('"', _) => {
                in_str = true;
                out.push(ch);
            }
            ('/', Some('/')) => {
                while i < c.len() && c[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            ('/', Some('*')) => {
                i += 2;
                while i + 1 < c.len() && !(c[i] == '*' && c[i + 1] == '/') {
                    i += 1;
                }
                i += 2;
                continue;
            }
            _ => out.push(ch),
        }
        i += 1;
    }
    // запятая перед закрывающей скобкой
    let mut res = String::with_capacity(out.len());
    let (mut in_str, mut esc) = (false, false);
    for (k, &ch) in out.iter().enumerate() {
        if in_str {
            res.push(ch);
            if esc {
                esc = false;
            } else if ch == '\\' {
                esc = true;
            } else if ch == '"' {
                in_str = false;
            }
            continue;
        }
        if ch == '"' {
            in_str = true;
        }
        if ch == ',' && out[k + 1..].iter().find(|x| !x.is_whitespace()).is_some_and(|x| matches!(x, '}' | ']')) {
            continue;
        }
        res.push(ch);
    }
    res
}

// ---------------------------------------------------------------------------------------------
// Тексты: краткое содержание
// ---------------------------------------------------------------------------------------------

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Разметка markdown внутри строки: жирный, ссылки.
fn clean_inline(s: &str) -> String {
    let t = crate::re!(r"\[([^\]]+)\]\([^)]*\)").replace_all(s, "$1");
    collapse_ws(&t.replace("**", "").replace("__", ""))
}

/// Не длиннее `max` знаков: режет по границе предложения, иначе по слову с «…». Короткий текст не меняется.
pub fn cut(s: &str, max: usize) -> String {
    let s = collapse_ws(s);
    if s.chars().count() <= max {
        return s;
    }
    let cs: Vec<(usize, char)> = s.char_indices().collect();
    let head_end = cs.get(max).map(|&(p, _)| p).unwrap_or(s.len());
    let head = &s[..head_end];
    // конец предложения: знак, за которым в исходном тексте идёт пробел
    let mut best = None;
    for (k, &(p, c)) in cs.iter().enumerate().take(max) {
        if matches!(c, '.' | '!' | '?') && cs.get(k + 1).is_none_or(|&(_, n)| n.is_whitespace()) {
            best = Some(p + c.len_utf8());
        }
    }
    if let Some(e) = best.filter(|&e| s[..e].chars().count() * 3 >= max) {
        return s[..e].to_string();
    }
    let cut_at = head.rfind(' ').filter(|&p| p > head.len() / 2).unwrap_or(head.len());
    let base = head[..cut_at].trim_end_matches(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | ':' | '-' | '(' | '/'));
    format!("{base}…")
}

/// Первый осмысленный абзац markdown: без заголовков, комментариев, кодовых блоков, таблиц и линий.
pub fn first_paragraph(body: &str) -> String {
    let (mut para, mut fence, mut html) = (Vec::<String>::new(), false, false);
    for raw in body.lines() {
        let l = raw.trim();
        if html {
            if l.contains("-->") {
                html = false;
            }
            continue;
        }
        if l.starts_with("```") || l.starts_with("~~~") {
            fence = !fence;
            if !para.is_empty() {
                break;
            }
            continue;
        }
        if fence {
            continue;
        }
        if l.starts_with("<!--") {
            html = !l.contains("-->");
            continue;
        }
        let skip = l.starts_with('#')
            || l.starts_with('|')
            || l.starts_with("---")
            || l.starts_with("***")
            || l.starts_with("===")
            || (l.starts_with('<') && l.ends_with('>'));
        if l.is_empty() || skip {
            if !para.is_empty() {
                break;
            }
            continue;
        }
        let l = l.trim_start_matches('>').trim();
        let l = crate::re!(r"^(?:[-*+]|\d+[.)])\s+").replace(l, "");
        para.push(l.to_string());
    }
    clean_inline(&para.join(" "))
}

/// Первый заголовок `# ...`.
fn first_heading(body: &str) -> Option<String> {
    let mut fence = false;
    for l in body.lines() {
        let t = l.trim();
        if t.starts_with("```") {
            fence = !fence;
        }
        if !fence {
            if let Some(h) = t.strip_prefix("# ") {
                return Some(clean_inline(h));
            }
        }
    }
    None
}

/// Первый абзац из блока комментариев в начале скрипта (`//`, `#`, `/* */`, `<# #>`, `REM`, `::`).
pub fn script_header(text: &str) -> String {
    let mut para: Vec<String> = vec![];
    let mut block = false;
    for (n, raw) in text.lines().enumerate() {
        let l = raw.trim();
        if n == 0 && l.starts_with("#!") {
            continue;
        }
        if block {
            if let Some(p) = l.find("*/").or_else(|| l.find("#>")) {
                let rest = l[..p].trim_start_matches('*').trim();
                if !rest.is_empty() {
                    para.push(rest.to_string());
                }
                block = false;
                if !para.is_empty() {
                    break;
                }
                continue;
            }
            let t = l.trim_start_matches('*').trim();
            if t.is_empty() {
                if !para.is_empty() {
                    break;
                }
            } else {
                para.push(t.to_string());
            }
            continue;
        }
        if l.is_empty() {
            if !para.is_empty() {
                break;
            }
            continue;
        }
        if let Some(rest) = l.strip_prefix("/*").or_else(|| l.strip_prefix("<#")) {
            let (inner, closed) = match rest.find("*/").or_else(|| rest.find("#>")) {
                Some(p) => (&rest[..p], true),
                None => (rest, false),
            };
            let inner = inner.trim_start_matches('*').trim();
            if !inner.is_empty() {
                para.push(inner.to_string());
            }
            if closed && !para.is_empty() {
                break;
            }
            block = !closed;
            continue;
        }
        let comment = ["//", "#", "::", "--"]
            .iter()
            .find_map(|p| l.strip_prefix(p))
            .or_else(|| l.strip_prefix("REM ").or_else(|| l.strip_prefix("rem ")));
        match comment {
            Some(c) => {
                let c = c.trim_start_matches(['/', '#', '-', '=', '*', ' ']).trim();
                if c.is_empty() {
                    if !para.is_empty() {
                        break;
                    }
                } else {
                    para.push(c.to_string());
                }
            }
            None => {
                if !para.is_empty() || n > 40 {
                    break;
                }
                // пропускаем служебные строки до комментария: set -e, @echo off, 'use strict'
                if l.len() > 60 {
                    break;
                }
            }
        }
    }
    clean_inline(&para.join(" "))
}

// ---------------------------------------------------------------------------------------------
// Секреты
// ---------------------------------------------------------------------------------------------

const MASK: &str = "•••";

struct Masker {
    key: Option<Regex>,
    value: Option<Regex>,
    inline: Option<Regex>,
    userinfo: Option<Regex>,
}

impl Masker {
    fn new(c: &SecretCfg) -> Masker {
        let re = |p: &str| if p.is_empty() { None } else { crate::util::cached_re(p) };
        Masker { key: re(&c.key), value: re(&c.value), inline: re(&c.inline), userinfo: re(&c.userinfo) }
    }

    fn secret_key(&self, k: &str) -> bool {
        self.key.as_ref().is_some_and(|r| r.is_match(k))
    }

    fn secret_value(&self, v: &str) -> bool {
        let v = v.trim().trim_matches(['"', '\'']);
        self.value.as_ref().is_some_and(|r| r.is_match(v))
    }

    /// Значение переменной окружения: видно, только если ни имя, ни значение не похожи на секрет.
    fn env(&self, key: &str, v: &Value) -> AiEnv {
        let s = match v {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            o => o.to_string(),
        };
        // ссылка на другую переменную (${TOKEN}) значения не раскрывает
        let reference = s.starts_with("${") && s.ends_with('}') || s.starts_with('$') && s.len() > 1 && !s.contains(' ');
        let hide = !reference
            && (self.secret_key(key) || self.secret_value(&s) || s.chars().count() > 80 || s.contains("://") && self.mask_text(&s) != s);
        if s.is_empty() {
            return AiEnv { key: key.to_string(), value: String::new(), masked: false };
        }
        AiEnv { key: key.to_string(), value: if hide { String::new() } else { s }, masked: hide }
    }

    /// Строка команды: пары «имя=значение» с секретным именем, логин:пароль в адресе и длинные токены.
    fn mask_text(&self, s: &str) -> String {
        let mut t = s.to_string();
        if let Some(r) = &self.userinfo {
            t = r.replace_all(&t, |c: &Captures| format!("{}{MASK}@", &c[1])).into_owned();
        }
        if let Some(r) = &self.inline {
            t = r.replace_all(&t, |c: &Captures| format!("{}{MASK}", &c[1])).into_owned();
        }
        let mut hide_next = false;
        let tokens = crate::re!(r"\S+").replace_all(&t, |c: &Captures| {
            let w = &c[0];
            // «--token xxx»: значение идёт следующим словом
            let flag = w.starts_with('-') && !w.contains('=') && self.secret_key(w);
            let was = std::mem::replace(&mut hide_next, flag);
            if was || self.secret_value(w) {
                MASK.to_string()
            } else {
                w.to_string()
            }
        });
        tokens.into_owned()
    }
}

// ---------------------------------------------------------------------------------------------
// Обход дерева
// ---------------------------------------------------------------------------------------------

struct Walk {
    /// пути относительно корня в исходном регистре, по алфавиту
    files: Vec<String>,
    lower: Vec<String>,
    capped: bool,
}

fn walk(root: &Path, skip: &HashSet<String>, skip_paths: &[String], cap: usize) -> Walk {
    let mut files: Vec<String> = vec![];
    let mut stack: Vec<(std::path::PathBuf, String)> = vec![(root.to_path_buf(), String::new())];
    let mut capped = false;
    'all: while let Some((dir, rel)) = stack.pop() {
        for e in ls(&dir) {
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(t) = e.file_type() else { continue };
            let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            if t.is_dir() {
                let low = r.to_lowercase();
                if skip.contains(&name.to_lowercase()) || skip_paths.contains(&low) {
                    continue;
                }
                stack.push((e.path(), r));
            } else if t.is_file() {
                if files.len() >= cap {
                    capped = true;
                    break 'all;
                }
                files.push(r);
            }
        }
    }
    files.sort();
    let lower = files.iter().map(|f| f.to_lowercase()).collect();
    Walk { files, lower, capped }
}

// ---------------------------------------------------------------------------------------------
// Сборка карты
// ---------------------------------------------------------------------------------------------

struct Built {
    items: Vec<AiItem>,
    /// текст файла детали (для поиска упоминаний и следов); пусто, если файла нет
    bodies: Vec<String>,
}

impl Built {
    fn push(&mut self, it: AiItem, body: String) {
        self.items.push(it);
        self.bodies.push(body);
    }

    /// Детали вида (с `start`) по имени, чтобы список не прыгал от порядка обхода папок.
    fn sort_from(&mut self, start: usize) {
        let tail_items = self.items.split_off(start);
        let tail_bodies = self.bodies.split_off(start);
        let mut pairs: Vec<(AiItem, String)> = tail_items.into_iter().zip(tail_bodies).collect();
        pairs.sort_by(|x, y| x.0.name.to_lowercase().cmp(&y.0.name.to_lowercase()).then(x.0.id.cmp(&y.0.id)));
        for (it, body) in pairs {
            self.push(it, body);
        }
    }
}

fn file_name(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

fn dir_of(rel: &str) -> &str {
    rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
}

fn stem(name: &str) -> String {
    let low = name.to_lowercase();
    for suf in [".prompt.md", ".instructions.md", ".agent.md", ".chatmode.md"] {
        if low.ends_with(suf) {
            return name[..name.len() - suf.len()].to_string();
        }
    }
    match name.rfind('.') {
        Some(p) if p > 0 => name[..p].to_string(),
        _ => name.to_string(),
    }
}

fn read_file(root: &Path, rel: &str, max: usize) -> Option<(String, u64, u64, bool)> {
    // секреты окружения не читаются никогда, как бы файл ни попал в список
    if file_name(rel).to_lowercase().starts_with(".env") {
        return None;
    }
    let p = root.join(rel);
    let meta = std::fs::symlink_metadata(&p).ok()?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return None;
    }
    let text = read_head(&p, max)?;
    Some((text, meta.len(), mtime_ms(&p), meta.len() as usize > max))
}

fn pick_mode(det: &Detect, fm: &Fm) -> String {
    for r in &det.modes {
        let hit = match &r.equals {
            Some(eq) => matches!(fm.get(&r.key), Some(Fv::Str(s)) if s.trim().eq_ignore_ascii_case(eq)),
            None => fm.has(&r.key),
        };
        if hit {
            return r.mode.clone();
        }
    }
    det.mode.clone()
}

fn base_item(kind: &KindDef, det: &Detect, rel: &str) -> AiItem {
    AiItem {
        id: format!("{}:{}", kind.id, rel),
        kind: kind.id.clone(),
        file: rel.to_string(),
        eco: det.eco.clone(),
        mode: det.mode.clone(),
        role: det.role.clone(),
        private: det.private,
        ..Default::default()
    }
}

fn name_for(kind: &KindDef, rel: &str, fm: &Fm) -> String {
    if let Some(n) = fm.text(&kind.parse.name) {
        return collapse_ws(&n);
    }
    match kind.parse.name_from.as_str() {
        "dir" => file_name(dir_of(rel)).to_string(),
        "filename" => file_name(rel).to_string(),
        "path" => rel.to_string(),
        _ => stem(file_name(rel)),
    }
}

fn markdown_item(kind: &KindDef, det: &Detect, rel: &str, root: &Path, cfg: &Cfg) -> Option<(AiItem, String)> {
    let (text, size, mtime, truncated) = read_file(root, rel, cfg.limits.file_bytes)?;
    let (fm, body) = parse_frontmatter(&text);
    let mut it = base_item(kind, det, rel);
    it.name = name_for(kind, rel, &fm);
    it.size = size;
    it.mtime = mtime;
    it.truncated = truncated;
    it.mode = pick_mode(det, &fm);
    let desc = fm.text(&kind.parse.desc).map(|d| clean_inline(&d));
    let source = desc.clone().unwrap_or_else(|| first_paragraph(body));
    it.summary = cut(&source, cfg.limits.summary);
    it.desc = if desc.is_some() || !source.is_empty() { cut(&source, cfg.limits.desc) } else { String::new() };
    if it.summary == it.desc {
        it.desc.clear();
    }
    it.tools = fm.list(&kind.parse.tools);
    it.model = fm.text(&kind.parse.model);
    it.paths = fm.list(&kind.parse.paths).into_iter().map(|g| AiGlob { glob: g, files: 0 }).collect();
    if it.summary.is_empty() {
        if let Some(h) = first_heading(body) {
            it.summary = cut(&h, cfg.limits.summary);
        }
    }
    Some((it, text))
}

/// Разбор «слова» команды в путь внутри проекта: убирает кавычки и переменную корня проекта.
fn command_path(word: &str) -> String {
    let mut w = word.trim_matches(['"', '\'', '`', '(', ')', ';']).replace('\\', "/");
    for var in [
        "${CLAUDE_PROJECT_DIR}",
        "$CLAUDE_PROJECT_DIR",
        "%CLAUDE_PROJECT_DIR%",
        "${PWD}",
        "$PWD",
        "$(pwd)",
        "$(git rev-parse --show-toplevel)",
    ] {
        w = w.replace(var, "");
    }
    while let Some(r) = w.strip_prefix("./") {
        w = r.to_string();
    }
    w.trim_start_matches('/').to_string()
}

/// Слова команды с учётом кавычек: `node "$DIR/x.mjs" --a` -> node, $DIR/x.mjs, --a.
fn command_words(cmd: &str) -> Vec<String> {
    let (mut out, mut cur, mut q) = (vec![], String::new(), None::<char>);
    for c in cmd.chars() {
        match q {
            Some(open) if c == open => q = None,
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => q = Some(c),
            None if c.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            None => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

struct Ctx<'a> {
    root: &'a Path,
    cfg: &'a Cfg,
    masker: Masker,
    /// путь строчными буквами -> индекс в walk.files
    index: HashMap<String, usize>,
    walk: &'a Walk,
}

impl Ctx<'_> {
    fn file_of(&self, word: &str) -> Option<&str> {
        let p = command_path(word).to_lowercase();
        self.index.get(&p).map(|&i| self.walk.files[i].as_str())
    }
}

fn hook_items(cx: &Ctx, kind: &KindDef, det: &Detect, rel: &str, b: &mut Built, problems: &mut Vec<AiProblem>) {
    let Some((text, size, mtime, _)) = read_file(cx.root, rel, cx.cfg.limits.file_bytes) else { return };
    let v = match parse_json(&text) {
        Ok(v) => v,
        Err(e) => {
            problems.push(AiProblem { file: rel.to_string(), error: e });
            return;
        }
    };
    let Some(hooks) = v.get("hooks").and_then(Value::as_object) else { return };
    for (event, groups) in hooks {
        for (gi, g) in groups.as_array().into_iter().flatten().enumerate() {
            let matcher = g.get("matcher").and_then(Value::as_str).map(str::to_string);
            for (hi, h) in g.get("hooks").and_then(Value::as_array).into_iter().flatten().enumerate() {
                let typ = h.get("type").and_then(Value::as_str).unwrap_or("command");
                let cmd_raw = h.get("command").and_then(Value::as_str).unwrap_or("");
                let cmd = cx.masker.mask_text(cmd_raw);
                let script = command_words(cmd_raw).iter().find_map(|w| cx.file_of(w).map(str::to_string));
                let mut it = base_item(kind, det, rel);
                it.id = format!("{}:{}#{}/{}/{}", kind.id, rel, event, gi, hi);
                it.event = Some(event.clone());
                it.matcher = matcher.clone();
                it.timeout = h.get("timeout").and_then(Value::as_u64);
                it.size = size;
                it.mtime = mtime;
                it.mode = det.mode.clone();
                it.script = script.clone();
                let head = match (&script, typ) {
                    (Some(s), _) => file_name(s).to_string(),
                    (None, "command") => command_words(&cmd).into_iter().find(|w| !w.starts_with('-')).unwrap_or_default(),
                    (None, other) => other.to_string(),
                };
                it.name = head;
                it.command = (!cmd.is_empty()).then_some(cmd.clone());
                // что делает хук: шапка его скрипта, иначе сам текст подсказки (prompt)
                let mut body = String::new();
                if let Some(s) = &script {
                    if let Some((t, ..)) = read_file(cx.root, s, 16_384) {
                        it.summary = cut(&script_header(&t), cx.cfg.limits.summary);
                        body = t;
                    }
                } else if let Some(p) = h.get("prompt").and_then(Value::as_str) {
                    it.summary = cut(&cx.masker.mask_text(p), cx.cfg.limits.summary);
                }
                if det.private {
                    it.command = None;
                    it.summary.clear();
                }
                b.push(it, body);
            }
        }
    }
}

fn count_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
}

fn permission_item(cx: &Ctx, kind: &KindDef, det: &Detect, rel: &str, b: &mut Built, problems: &mut Vec<AiProblem>) {
    let Some((text, size, mtime, _)) = read_file(cx.root, rel, cx.cfg.limits.file_bytes) else { return };
    let mut it = base_item(kind, det, rel);
    it.name = file_name(rel).to_string();
    it.size = size;
    it.mtime = mtime;
    let v = match parse_json(&text) {
        Ok(v) => v,
        Err(e) => {
            it.broken = Some(e.clone());
            problems.push(AiProblem { file: rel.to_string(), error: e });
            b.push(it, String::new());
            return;
        }
    };
    let perm = v.get("permissions");
    let (allow, deny, ask) = (
        count_list(perm.and_then(|p| p.get("allow"))),
        count_list(perm.and_then(|p| p.get("deny"))),
        count_list(perm.and_then(|p| p.get("ask"))),
    );
    let mut fact = |k: &str, val: String| it.facts.push(AiFact { k: k.to_string(), v: val });
    fact("allow", allow.len().to_string());
    fact("deny", deny.len().to_string());
    fact("ask", ask.len().to_string());
    if let Some(m) = perm.and_then(|p| p.get("defaultMode")).and_then(Value::as_str) {
        fact("defaultMode", m.to_string());
    }
    let hooks_n: usize =
        v.get("hooks").and_then(Value::as_object).map(|o| o.values().filter_map(Value::as_array).map(Vec::len).sum()).unwrap_or(0);
    if hooks_n > 0 {
        fact("hooks", hooks_n.to_string());
    }
    if let Some(m) = v.get("model").and_then(Value::as_str) {
        it.model = Some(m.to_string());
    }
    if let Some(env) = v.get("env").and_then(Value::as_object) {
        if !env.is_empty() {
            it.facts.push(AiFact { k: "env".into(), v: env.len().to_string() });
            if !det.private {
                it.env = env.iter().map(|(k, val)| cx.masker.env(k, val)).collect();
            }
        }
    }
    let keys: Vec<String> = v.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
    it.lists.push(AiList { k: "keys".into(), items: keys });
    if !det.private {
        for (k, l) in [("allow", &allow), ("deny", &deny), ("ask", &ask)] {
            if !l.is_empty() {
                it.lists.push(AiList { k: k.into(), items: l.iter().take(60).map(|s| cut(&cx.masker.mask_text(s), 160)).collect() });
            }
        }
    }
    it.summary = it
        .facts
        .iter()
        .filter(|f| f.v != "0")
        .map(|f| format!("{}: {}", cx.cfg.facts.get(&f.k).unwrap_or(&f.k), f.v))
        .collect::<Vec<_>>()
        .join(", ");
    b.push(it, String::new());
}

fn mcp_items(cx: &Ctx, kind: &KindDef, det: &Detect, rel: &str, b: &mut Built, problems: &mut Vec<AiProblem>) {
    let Some((text, size, mtime, _)) = read_file(cx.root, rel, cx.cfg.limits.file_bytes) else { return };
    let v = match parse_json(&text) {
        Ok(v) => v,
        Err(e) => {
            let mut it = base_item(kind, det, rel);
            it.name = file_name(rel).to_string();
            it.size = size;
            it.mtime = mtime;
            it.broken = Some(e.clone());
            problems.push(AiProblem { file: rel.to_string(), error: e });
            b.push(it, String::new());
            return;
        }
    };
    let servers = v.get("mcpServers").or_else(|| v.get("servers")).and_then(Value::as_object);
    for (name, s) in servers.into_iter().flatten() {
        let mut it = base_item(kind, det, rel);
        it.id = format!("{}:{}#{}", kind.id, rel, name);
        it.name = name.clone();
        it.size = size;
        it.mtime = mtime;
        let command = s.get("command").and_then(Value::as_str);
        let url = s.get("url").and_then(Value::as_str);
        it.transport = s
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| Some(if command.is_some() { "stdio" } else { "http" }.to_string()));
        let mut line = String::new();
        if let Some(c) = command {
            line.push_str(c);
            for a in s.get("args").and_then(Value::as_array).into_iter().flatten() {
                line.push(' ');
                line.push_str(&a.as_str().map(str::to_string).unwrap_or_else(|| a.to_string()));
            }
        } else if let Some(u) = url {
            line.push_str(u);
        }
        let masked = cx.masker.mask_text(&line);
        it.summary = cut(&masked, cx.cfg.limits.summary);
        it.command = (!masked.is_empty()).then_some(masked);
        if let Some(env) = s.get("env").and_then(Value::as_object) {
            it.env = env.iter().map(|(k, val)| cx.masker.env(k, val)).collect();
        }
        // заголовки запросов: видны только имена
        if let Some(h) = s.get("headers").and_then(Value::as_object) {
            it.env.extend(h.iter().map(|(k, val)| AiEnv {
                masked: true,
                value: String::new(),
                key: format!("{k}{}", if val.is_string() { "" } else { "?" }),
            }));
        }
        if s.get("disabled").and_then(Value::as_bool) == Some(true) {
            it.facts.push(AiFact { k: "disabled".into(), v: "1".into() });
        }
        b.push(it, String::new());
    }
    if servers.is_none() {
        let mut it = base_item(kind, det, rel);
        it.name = file_name(rel).to_string();
        it.size = size;
        it.mtime = mtime;
        b.push(it, String::new());
    }
}

fn config_item(cx: &Ctx, kind: &KindDef, det: &Detect, rel: &str, b: &mut Built) {
    let Some((text, size, mtime, truncated)) = read_file(cx.root, rel, cx.cfg.limits.file_bytes) else { return };
    let mut it = base_item(kind, det, rel);
    it.name = file_name(rel).to_string();
    it.size = size;
    it.mtime = mtime;
    it.truncated = truncated;
    it.facts.push(AiFact { k: "lines".into(), v: text.lines().count().to_string() });
    let head = script_header(&text);
    it.summary = cut(&cx.masker.mask_text(&head), cx.cfg.limits.summary);
    b.push(it, text);
}

fn script_item(cx: &Ctx, kind: &KindDef, det: &Detect, rel: &str, b: &mut Built) {
    let name = file_name(rel);
    // служебная папка husky и вложенные файлы хуков не детали
    if rel.contains("/_/") || rel.ends_with("/_") {
        return;
    }
    let Some((text, size, mtime, truncated)) = read_file(cx.root, rel, cx.cfg.limits.file_bytes) else { return };
    let mut it = base_item(kind, det, rel);
    it.name = name.to_string();
    it.size = size;
    it.mtime = mtime;
    it.truncated = truncated;
    it.mode = if kind.events.iter().any(|e| e == name) { det.mode.clone() } else { "helper".into() };
    it.summary = cut(&script_header(&text), cx.cfg.limits.summary);
    b.push(it, text);
}

/// Значение core.hooksPath из .git/config (читается как текст, git не запускается).
fn hooks_path(root: &Path) -> Option<String> {
    let cfg = root.join(".git").join("config");
    let meta = std::fs::symlink_metadata(&cfg).ok()?;
    if !meta.is_file() {
        return None;
    }
    let text = read_head(&cfg, 65_536)?;
    let mut section = String::new();
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            section = t.to_lowercase();
        } else if section.starts_with("[core") {
            if let Some((k, v)) = t.split_once('=') {
                if k.trim().eq_ignore_ascii_case("hookspath") {
                    return Some(v.trim().trim_matches('"').replace('\\', "/"));
                }
            }
        }
    }
    None
}

fn norm_dir(p: &str) -> String {
    p.trim().trim_start_matches("./").trim_end_matches('/').to_lowercase()
}

fn check_role(name_and_file: &str, meta: &Option<Regex>, measure: &Option<Regex>) -> &'static str {
    if meta.as_ref().is_some_and(|r| r.is_match(name_and_file)) {
        "meta"
    } else if measure.as_ref().is_some_and(|r| r.is_match(name_and_file)) {
        "measure"
    } else {
        "tests"
    }
}

fn check_items(cx: &Ctx, kind: &KindDef, project: &Project, b: &mut Built) {
    let re = |p: &str| if p.is_empty() { None } else { crate::util::cached_re(p) };
    let (meta, measure, also) = (re(&kind.meta_re), re(&kind.measure_re), re(&kind.scripts.also_re));
    let max = cx.cfg.limits.items_per_kind;
    let mut seen_files: HashSet<String> = HashSet::new();
    for s in &project.scripts {
        let by_stage = kind.scripts.stages.contains(&s.stage);
        let by_text = also.as_ref().is_some_and(|r| r.is_match(&format!("{} {}", s.name, s.info.summary)));
        if !(by_stage || by_text) || b.items.len() >= max {
            continue;
        }
        let file = s
            .file_rel
            .clone()
            .or_else(|| {
                (!s.virtual_ && matches!(s.source.as_str(), "package.json" | "Makefile" | "justfile" | "Taskfile.yml")).then(|| {
                    if s.where_ == "." {
                        s.source.clone()
                    } else {
                        format!("{}/{}", s.where_, s.source)
                    }
                })
            })
            .unwrap_or_default();
        let det = Detect { eco: String::new(), mode: "run".into(), ..Default::default() };
        let mut it = base_item(kind, &det, &file);
        it.id = format!("{}:script:{}", kind.id, s.id);
        it.name = if s.where_ == "." || s.where_.is_empty() { s.name.clone() } else { format!("{}:{}", s.where_, s.name) };
        it.summary = cut(&s.info.summary, cx.cfg.limits.summary);
        it.role = if by_text && !by_stage {
            check_role(&format!("{} {}", s.name, file), &meta, &measure)
        } else {
            check_role(&format!("{} {}", s.name, file), &meta, &None)
        }
        .to_string();
        it.command = Some(cx.masker.mask_text(&s.cmd));
        it.line = s.line;
        it.facts.push(AiFact { k: "stage".into(), v: s.stage.clone() });
        if s.virtual_ {
            it.facts.push(AiFact { k: "standard".into(), v: "1".into() });
        }
        if let Some(i) = cx.index.get(&file.to_lowercase()) {
            seen_files.insert(cx.walk.lower[*i].clone());
        }
        b.push(it, String::new());
    }
    if kind.ci {
        for c in &project.ci {
            let mut push = |name: &str, stage: &str, desc: &str, line: u32, extra: Vec<AiFact>| {
                if b.items.len() >= max {
                    return;
                }
                let det = Detect { mode: "ci".into(), role: "ci".into(), ..Default::default() };
                let mut it = base_item(kind, &det, &c.file);
                it.id = format!("{}:ci:{}:{}", kind.id, c.file, name);
                it.name = name.to_string();
                it.summary = cut(desc, cx.cfg.limits.summary);
                it.line = (line > 0).then_some(line);
                if !stage.is_empty() {
                    it.facts.push(AiFact { k: "stage".into(), v: stage.to_string() });
                }
                it.facts.extend(extra);
                b.push(it, String::new());
            };
            for st in &c.stages {
                for j in &st.jobs {
                    let mut extra = vec![];
                    if j.manual {
                        extra.push(AiFact { k: "manual".into(), v: "1".into() });
                    }
                    if j.allow_failure {
                        extra.push(AiFact { k: "allowFailure".into(), v: "1".into() });
                    }
                    push(&j.name, &st.name, &j.desc, j.line, extra);
                }
            }
            for j in &c.jobs {
                push(&j.name, "", "", j.line, vec![]);
            }
        }
    }
    // файлы-инструменты по шаблонам из ai.json (.claude/scripts/*): не дубль уже найденных скриптов
    for det in &kind.detect {
        let Some(g) = Glob::anchored(&det.glob) else { continue };
        for (i, low) in cx.walk.lower.iter().enumerate() {
            if !g.is_match(low) || seen_files.contains(low) || b.items.len() >= max {
                continue;
            }
            let rel = &cx.walk.files[i];
            let Some((text, size, mtime, _)) = read_file(cx.root, rel, cx.cfg.limits.file_bytes) else { continue };
            let mut it = base_item(kind, det, rel);
            it.name = file_name(rel).to_string();
            it.size = size;
            it.mtime = mtime;
            it.role = check_role(&format!("{} {}", it.name, rel), &meta, &measure).to_string();
            it.summary = cut(&script_header(&text), cx.cfg.limits.summary);
            b.push(it, text);
        }
    }
}

fn signal_items(cx: &Ctx, kind: &KindDef, b: &mut Built, sources: &[usize]) {
    let re_cap = cx.cfg.limits.hits;
    for s in &kind.signals {
        let det = Detect { mode: "trace".into(), role: s.id.clone(), ..Default::default() };
        let mut it = base_item(kind, &det, &s.dir);
        it.id = format!("{}:{}", kind.id, s.id);
        it.name = s.title.clone();
        it.summary = s.say.clone();
        if !s.dir.is_empty() {
            let p = cx.root.join(&s.dir);
            if is_link(&p) || !p.is_dir() {
                continue;
            }
            it.file = s.dir.clone();
            let names: Vec<String> = ls(&p)
                .iter()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            if s.count_dirs {
                it.facts.push(AiFact { k: "dirs".into(), v: names.len().to_string() });
                let mut names = names;
                names.sort();
                it.lists.push(AiList { k: "dirs".into(), items: names.into_iter().take(40).collect() });
            }
            it.mtime = mtime_ms(&p);
        } else if !s.file.is_empty() {
            let p = cx.root.join(&s.file);
            let Ok(m) = std::fs::symlink_metadata(&p) else { continue };
            if m.file_type().is_symlink() || !m.is_file() {
                continue;
            }
            it.file = s.file.clone();
            it.size = m.len();
            it.mtime = mtime_ms(&p);
        } else if let Some(r) = crate::util::cached_re(&s.scan).filter(|_| !s.scan.is_empty()) {
            let mut files = 0;
            for &k in sources {
                let item = &b.items[k];
                for (n, line) in b.bodies[k].lines().enumerate() {
                    if r.is_match(line) {
                        files += 1;
                        if it.hits.len() < re_cap {
                            it.hits.push(AiHit {
                                file: item.file.clone(),
                                line: n as u32 + 1,
                                text: cut(&cx.masker.mask_text(line.trim()), 200),
                            });
                        }
                        break;
                    }
                }
            }
            if files == 0 {
                continue;
            }
            it.facts.push(AiFact { k: "files".into(), v: files.to_string() });
            it.file = it.hits.first().map(|h| h.file.clone()).unwrap_or_default();
            it.line = it.hits.first().map(|h| h.line);
        }
        b.push(it, String::new());
    }
}

// ---- упоминания -----------------------------------------------------------------------------

fn norm_token(t: &str) -> String {
    let mut s = t
        .trim_matches(|c: char| {
            matches!(c, '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>' | ',' | ';' | ':' | '!' | '?' | '*' | '`' | ' ')
        })
        .trim_end_matches('.')
        .replace('\\', "/");
    while let Some(r) = s.strip_prefix("./") {
        s = r.to_string();
    }
    // «file.md:12» и «file.md#раздел»
    if let Some(p) = s.find('#') {
        s.truncate(p);
    }
    let s = crate::re!(r":\d+(?:-\d+)?$").replace(&s, "").into_owned();
    s.trim_end_matches('/').to_lowercase()
}

fn join_rel(dir: &str, t: &str) -> String {
    let mut parts: Vec<&str> = if dir.is_empty() { vec![] } else { dir.split('/').collect() };
    for seg in t.split('/') {
        match seg {
            ".." => {
                parts.pop();
            }
            "." | "" => {}
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn mention_tokens(text: &str, limit: usize) -> Vec<String> {
    let t: &str = if text.len() > limit { text.get(..limit).unwrap_or(text) } else { text };
    let mut out: Vec<String> = vec![];
    for c in crate::re!(r"`([^`\n]{1,160})`").captures_iter(t) {
        let inner = &c[1];
        out.push(norm_token(inner));
        if inner.contains(char::is_whitespace) {
            out.extend(inner.split_whitespace().map(norm_token));
        }
    }
    for c in crate::re!(r"\]\(([^)\s#]+)(?:#[^)]*)?\)").captures_iter(t) {
        if !c[1].contains("://") {
            out.push(norm_token(&c[1]));
        }
    }
    for c in crate::re!(r"(?:^|\s)@([A-Za-z0-9_./~-]+)").captures_iter(t) {
        out.push(norm_token(&c[1]));
    }
    out.retain(|x| !x.is_empty());
    out.sort();
    out.dedup();
    out
}

/// Пути в тексте скрипта: слова с «/» или точкой, нормализованные.
fn path_tokens(text: &str, limit: usize) -> Vec<String> {
    let t: &str = if text.len() > limit { text.get(..limit).unwrap_or(text) } else { text };
    let mut out: Vec<String> = crate::re!(r#"[A-Za-z0-9_@./\\${}%~()-]{3,}"#)
        .find_iter(t)
        .map(|m| command_path(m.as_str()))
        .map(|w| w.trim_matches(['(', ')']).to_lowercase())
        .filter(|w| w.contains('/') || w.contains('.'))
        .collect();
    out.sort();
    out.dedup();
    out
}

fn link_items(b: &mut Built, kinds: &[KindDef], lim: &Limits) {
    let kind_of: HashMap<&str, &KindDef> = kinds.iter().map(|k| (k.id.as_str(), k)).collect();
    // личность детали -> индексы; неоднозначная (двое отвечают на одно имя) отбрасывается: угадывать нельзя
    let mut ident: HashMap<String, Vec<usize>> = HashMap::new();
    let add = |key: String, i: usize, ident: &mut HashMap<String, Vec<usize>>| {
        if !key.is_empty() {
            let e = ident.entry(key).or_default();
            if !e.contains(&i) {
                e.push(i);
            }
        }
    };
    for (i, it) in b.items.iter().enumerate() {
        let Some(k) = kind_of.get(it.kind.as_str()) else { continue };
        for r in &k.refs {
            match r.as_str() {
                "path" if !it.file.is_empty() => add(it.file.to_lowercase(), i, &mut ident),
                "file" if !it.file.is_empty() && file_name(&it.file).contains('.') => {
                    add(file_name(&it.file).to_lowercase(), i, &mut ident)
                }
                "name" if it.name.chars().count() >= lim.mention_min => add(it.name.to_lowercase(), i, &mut ident),
                // имя с разделителем (verify-gates, lint:api): обычное слово в кавычках не считается ссылкой
                "tag" if it.name.chars().count() >= lim.mention_min && it.name.contains(['-', '_', ':', '.', '/']) => {
                    add(it.name.to_lowercase(), i, &mut ident)
                }
                "slash" if it.name.chars().count() >= lim.mention_min => add(format!("/{}", it.name.to_lowercase()), i, &mut ident),
                "script" => {
                    if let Some(s) = &it.script {
                        add(s.to_lowercase(), i, &mut ident);
                        add(file_name(s).to_lowercase(), i, &mut ident);
                    }
                }
                _ => {}
            }
        }
    }
    let by_file: HashMap<String, Vec<usize>> = {
        let mut m: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, it) in b.items.iter().enumerate() {
            let runnable = matches!(kind_of.get(it.kind.as_str()).map(|k| k.format.as_str()), Some("script" | "checks" | "config"))
                || it.kind == "hooks" && it.unwired;
            if runnable && !it.file.is_empty() {
                m.entry(it.file.to_lowercase()).or_default().push(i);
            }
        }
        m
    };
    let mut edges: Vec<(usize, usize)> = vec![];
    let mut seen: HashSet<(usize, usize)> = HashSet::new();
    let mut link = |from: usize, to: usize, edges: &mut Vec<(usize, usize)>| {
        if from != to && seen.insert((from, to)) {
            edges.push((from, to));
        }
    };
    for i in 0..b.items.len() {
        let it = &b.items[i];
        let dir = dir_of(&it.file).to_string();
        let is_script = matches!(kind_of.get(it.kind.as_str()).map(|k| k.format.as_str()), Some("script" | "checks" | "hooks"));
        if !b.bodies[i].is_empty() {
            // упоминания: обратные кавычки, ссылки markdown, @путь
            for tok in mention_tokens(&b.bodies[i], lim.body_scan) {
                let local = join_rel(&dir, &tok);
                for key in [&tok, &local] {
                    if let Some(t) = ident.get(key.as_str()).filter(|v| v.len() == 1) {
                        link(i, t[0], &mut edges);
                        break;
                    }
                }
            }
            // следы запуска: путь к другому скрипту проекта в тексте скрипта
            if is_script {
                for tok in path_tokens(&b.bodies[i], lim.body_scan) {
                    let local = join_rel(&dir, &tok);
                    for key in [&tok, &local] {
                        if let Some(t) = by_file.get(key.as_str()).filter(|v| v.len() == 1) {
                            link(i, t[0], &mut edges);
                            break;
                        }
                    }
                }
            }
        }
        // запись хука запускает скрипт, который уже является деталью (проверка, хук-скрипт)
        if let Some(p) = &it.script {
            if let Some(t) = by_file.get(&p.to_lowercase()).filter(|v| v.len() == 1) {
                link(i, t[0], &mut edges);
            }
        }
    }
    for (from, to) in edges {
        let (fid, tid) = (b.items[from].id.clone(), b.items[to].id.clone());
        b.items[from].refs_out.push(tid);
        b.items[to].refs_in.push(fid);
    }
}

// ---- рассказ ---------------------------------------------------------------------------------

fn render_parts(names: &[String], more: &str) -> String {
    const SHOWN: usize = 4;
    let mut s = names.iter().take(SHOWN).cloned().collect::<Vec<_>>().join(", ");
    if names.len() > SHOWN {
        s.push(' ');
        s.push_str(&more.replace("{n}", &(names.len() - SHOWN).to_string()));
    }
    s
}

fn build_story(items: &[AiItem], cfg: &Cfg) -> Vec<AiStep> {
    let mut out = vec![];
    if items.is_empty() {
        return out;
    }
    for st in &cfg.story {
        let mut parts: Vec<usize> = vec![];
        for c in &st.any {
            let re = (!c.pat.is_empty()).then(|| crate::util::cached_re(&c.pat)).flatten();
            for (i, it) in items.iter().enumerate() {
                if it.kind != c.kind || (!c.mode.is_empty() && it.mode != c.mode) || (!c.role.is_empty() && it.role != c.role) {
                    continue;
                }
                if !c.pat.is_empty() && !re.as_ref().is_some_and(|r| r.is_match(&format!("{} {} {}", it.name, it.summary, it.desc))) {
                    continue;
                }
                if !parts.contains(&i) {
                    parts.push(i);
                }
            }
        }
        if parts.is_empty() && !st.always {
            continue;
        }
        let mut names: Vec<String> = vec![];
        for &i in &parts {
            let n = items[i].name.clone();
            if !names.contains(&n) {
                names.push(n);
            }
        }
        out.push(AiStep {
            id: st.id.clone(),
            title: st.title.clone(),
            text: st.say.replace("{parts}", &render_parts(&names, &cfg.story_more)).replace("{n}", &parts.len().to_string()),
            parts: parts.iter().take(12).map(|&i| items[i].id.clone()).collect(),
        });
    }
    out
}

// ---- главная функция ---------------------------------------------------------------------------

/// Собирает карту ИИ-части проекта. Ничего не запускает и ничего не пишет.
pub fn build(root: &Path, project: &Project, kb: &Kb) -> AiMap {
    let t0 = Instant::now();
    let cfg = Cfg::load(kb);
    let mut map = AiMap { path: project.path.clone(), built_at: now_ms(), ..Default::default() };
    if cfg.kinds.is_empty() || crate::util::is_unc(&root.to_string_lossy()) {
        return map;
    }
    let skip: HashSet<String> = kb.str_list("skipDirs").into_iter().map(|s| s.to_lowercase()).collect();
    let skip_paths: Vec<String> = cfg.walk_skip.iter().map(|s| s.to_lowercase()).collect();
    let ignore: HashSet<String> = cfg.ignore_files.iter().map(|s| s.to_lowercase()).collect();
    let mut w = walk(root, &skip, &skip_paths, cfg.limits.walk_files);
    // заглушки папок (.gitkeep) не детали и в счёт файлов не идут
    let keep: Vec<bool> = w.files.iter().map(|f| !ignore.contains(&file_name(f).to_lowercase())).collect();
    let mut k = keep.iter();
    w.files.retain(|_| *k.next().unwrap_or(&true));
    let mut k = keep.iter();
    w.lower.retain(|_| *k.next().unwrap_or(&true));
    let index: HashMap<String, usize> = w.lower.iter().enumerate().map(|(i, l)| (l.clone(), i)).collect();
    let cx = Ctx { root, cfg: &cfg, masker: Masker::new(&cfg.secrets), index, walk: &w };
    let mut b = Built { items: vec![], bodies: vec![] };
    let mut problems: Vec<AiProblem> = vec![];
    let mut wired: HashSet<String> = HashSet::new();

    for kind in &cfg.kinds {
        let start = b.items.len();
        match kind.format.as_str() {
            "checks" => check_items(&cx, kind, project, &mut b),
            "signals" => {
                let sources: Vec<usize> =
                    (0..b.items.len()).filter(|&i| !b.bodies[i].is_empty() && kind.scan_kinds.contains(&b.items[i].kind)).collect();
                signal_items(&cx, kind, &mut b, &sources);
            }
            _ => {
                // записи настроек (хуки) раньше одиночных скриптов: нужно знать, какие скрипты подключены
                let mut dets: Vec<&Detect> = kind.detect.iter().collect();
                dets.sort_by_key(|d| u8::from(d.format == "hookscript"));
                for det in dets {
                    let Some(g) = Glob::anchored(&det.glob) else { continue };
                    let fmt = if det.format.is_empty() { kind.format.as_str() } else { det.format.as_str() };
                    for (i, low) in w.lower.iter().enumerate() {
                        if b.items.len() - start >= cfg.limits.items_per_kind {
                            break;
                        }
                        if !g.is_match(low) {
                            continue;
                        }
                        let rel = w.files[i].as_str();
                        let before = b.items.len();
                        match fmt {
                            "hooks" => {
                                hook_items(&cx, kind, det, rel, &mut b, &mut problems);
                                wired.extend(b.items[before..].iter().filter_map(|it| it.script.as_ref().map(|s| s.to_lowercase())));
                            }
                            "hookscript" => {
                                if wired.contains(low) {
                                    continue;
                                }
                                if let Some((text, size, mtime, truncated)) = read_file(root, rel, cfg.limits.file_bytes) {
                                    let mut it = base_item(kind, det, rel);
                                    it.name = file_name(rel).to_string();
                                    it.size = size;
                                    it.mtime = mtime;
                                    it.truncated = truncated;
                                    it.unwired = true;
                                    it.summary = cut(&script_header(&text), cfg.limits.summary);
                                    b.push(it, text);
                                }
                            }
                            "permissions" => permission_item(&cx, kind, det, rel, &mut b, &mut problems),
                            "mcp" => mcp_items(&cx, kind, det, rel, &mut b, &mut problems),
                            "config" => config_item(&cx, kind, det, rel, &mut b),
                            "script" => script_item(&cx, kind, det, rel, &mut b),
                            _ => {
                                if let Some((it, body)) = markdown_item(kind, det, rel, root, &cfg) {
                                    b.push(it, body);
                                }
                            }
                        }
                    }
                }
            }
        }
        b.sort_from(start);
    }
    drop(cx);

    // шаблоны путей правил против настоящего дерева
    for it in b.items.iter_mut().filter(|it| !it.paths.is_empty()) {
        let globs: Vec<Option<Glob>> = it.paths.iter().map(|p| Glob::new(&p.glob)).collect();
        let mut counts = vec![0u32; globs.len()];
        let mut any = 0u32;
        for low in &w.lower {
            let mut hit = false;
            for (k, g) in globs.iter().enumerate() {
                if g.as_ref().is_some_and(|g| g.is_match(low)) {
                    counts[k] += 1;
                    hit = true;
                }
            }
            any += u32::from(hit);
        }
        for (p, n) in it.paths.iter_mut().zip(counts) {
            p.files = n;
        }
        it.files_matched = any;
    }

    // git видит папку хуков только через core.hooksPath
    let hp = hooks_path(root);
    let git_known = root.join(".git").is_dir();
    for it in b.items.iter_mut().filter(|it| it.kind == "githooks" && it.eco == "git") {
        if git_known {
            it.active = Some(hp.as_deref().is_some_and(|h| norm_dir(h) == dir_of(&it.file).to_lowercase()));
        }
    }

    link_items(&mut b, &cfg.kinds, &cfg.limits);

    let order: HashMap<&str, usize> = cfg.kinds.iter().enumerate().map(|(i, k)| (k.id.as_str(), i)).collect();
    let kind_at: HashMap<&str, &str> = b.items.iter().map(|it| (it.id.as_str(), it.kind.as_str())).collect();
    let mut edge_n: BTreeMap<(usize, usize), u32> = BTreeMap::new();
    for it in &b.items {
        for t in &it.refs_out {
            let (Some(&a), Some(&bb)) = (order.get(it.kind.as_str()), kind_at.get(t.as_str()).and_then(|k| order.get(k))) else { continue };
            if a != bb {
                *edge_n.entry((a.min(bb), a.max(bb))).or_default() += 1;
            }
        }
    }
    map.kind_edges =
        edge_n.into_iter().map(|((a, bb), n)| AiKindEdge { from: cfg.kinds[a].id.clone(), to: cfg.kinds[bb].id.clone(), n }).collect();
    map.kinds = cfg
        .kinds
        .iter()
        .map(|k| {
            let mut st = AiKindStat { id: k.id.clone(), ..Default::default() };
            for it in b.items.iter().filter(|it| it.kind == k.id) {
                st.count += 1;
                *st.modes.entry(it.mode.clone()).or_default() += 1;
                if !it.role.is_empty() {
                    *st.roles.entry(it.role.clone()).or_default() += 1;
                }
            }
            st
        })
        .collect();
    map.story = build_story(&b.items, &cfg);
    map.problems = problems;
    map.walked = w.files.len() as u32;
    map.capped = w.capped;
    map.hooks_path = hp;
    map.items = b.items;
    map.fp = key_of(&serde_json::to_string(&(&map.items, &map.kind_edges, &map.story, &map.problems)).unwrap_or_default());
    map.ms = t0.elapsed().as_millis() as u64;
    map
}

/// Название вида из ai.json (для терминала и сообщений).
pub fn kind_title(kb: &Kb, id: &str) -> String {
    kb.ai["kinds"].as_array().into_iter().flatten().find(|k| k["id"] == id).and_then(|k| k["title"].as_str()).unwrap_or(id).to_string()
}

/// Подпись режима загрузки детали («всегда», «по путям») из ai.json.
pub fn mode_label(kb: &Kb, kind: &str, mode: &str) -> String {
    kb.ai["kinds"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|k| k["id"] == kind)
        .and_then(|k| k["modes"][mode]["label"].as_str())
        .unwrap_or(mode)
        .to_string()
}
