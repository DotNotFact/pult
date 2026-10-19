//! Разбор папки проекта. Форматы файлов (package.json, Makefile...) разбирает код,
//! а всё, что касается смысла, - этапы, стандартные команды, метки технологий - берётся из базы знаний.
use crate::explain::{self, Ctx};
use crate::kb::{self, Kb};
use crate::model::*;
use crate::util::{display_argv, id_of, is_unc, ls, mtime_ms, now_ms, read_head, read_text, rel};
use crate::{ci_config, git, runner, topics};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

#[derive(Default, Clone)]
pub struct ScanOpts {
    pub trusted_hosts: Vec<String>,
}

/// Запись папки без ссылок. Тип берётся из самой записи каталога: отдельный запрос к диску
/// на каждый файл (`is_file`) стоил на большом проекте десятки миллисекунд.
struct Ent {
    name: String,
    path: PathBuf,
    dir: bool,
    file: bool,
}

/// Папка, прочитанная один раз за разбор. Разборщики сначала смотрят сюда, а файл открывают,
/// только если он есть: попытка открыть отсутствующий `justfile` - это обращение к диску.
struct Listing {
    /// записи без ссылок (symlink, junction), в порядке чтения папки
    entries: Vec<Ent>,
    /// имена всех записей, ссылок тоже, строчными: «есть ли файл» решается так же, как раньше
    /// решала попытка его открыть (на Windows имена без учёта регистра, ссылку открывал read_text)
    names: HashSet<String>,
}

impl Listing {
    fn read(dir: &Path) -> Listing {
        let mut entries = Vec::new();
        let mut names = HashSet::new();
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            names.insert(name.to_lowercase());
            let Ok(t) = e.file_type() else { continue };
            if t.is_symlink() {
                continue;
            }
            entries.push(Ent { path: e.path(), name, dir: t.is_dir(), file: t.is_file() });
        }
        Listing { entries, names }
    }

    fn has(&self, name: &str) -> bool {
        self.names.contains(&name.to_lowercase())
    }

    /// Записи, отсортированные по имени (как `sort_by_key(file_name)` у `ls`).
    fn sorted(&self) -> Vec<&Ent> {
        let mut v: Vec<&Ent> = self.entries.iter().collect();
        v.sort_by(|a, b| a.path.file_name().cmp(&b.path.file_name()));
        v
    }
}

struct Scan<'a> {
    root: PathBuf,
    kb: &'a Kb,
    /// прочитанные папки: каждая читается один раз за разбор
    dirs: HashMap<PathBuf, Arc<Listing>>,
    pm: String,
    scripts: Vec<Script>,
    stacks: BTreeSet<String>,
    deps: BTreeSet<String>,
    found: BTreeSet<String>,
    warnings: Vec<String>,
    name: Option<String>,
    version: Option<String>,
    description: Option<String>,
    engines: Option<String>,
    compose: Option<Vec<String>>,
}

/// Уточнения к скрипту, собранному `Scan::mk`: группа, этап, «стандартная команда».
impl Script {
    fn in_group(mut self, group: impl Into<String>) -> Self {
        self.group = group.into();
        self
    }
    /// Этап задан явно (база знаний или папка файла), а не угадан по имени.
    fn at_stage(mut self, stage: Option<&str>) -> Self {
        if let Some(s) = stage {
            self.stage = s.to_string();
        }
        self
    }
    /// Стандартная команда по манифесту, а не скрипт автора: имя не делает её опасной.
    fn standard(mut self) -> Self {
        self.virtual_ = true;
        self.danger = false;
        self
    }
}

impl Scan<'_> {
    /// Папка из кэша разбора; первая просьба читает её с диска.
    fn listing(&mut self, dir: &Path) -> Arc<Listing> {
        if let Some(l) = self.dirs.get(dir) {
            return l.clone();
        }
        let l = Arc::new(Listing::read(dir));
        self.dirs.insert(dir.to_path_buf(), l.clone());
        l
    }

    /// Текст файла из папки, если он там есть. Путь с разделителем читается как раньше.
    fn read(&mut self, dir: &Path, name: &str) -> Option<String> {
        if name.contains(['/', '\\']) || self.listing(dir).has(name) {
            read_text(&dir.join(name))
        } else {
            None
        }
    }

    /// Скрипт автора: группа - файл-источник, этап угадан по имени и тексту команды.
    fn mk(&self, dir: &Path, source: &str, name: &str, argv: Vec<String>, raw: Vec<String>) -> Script {
        let where_ = rel(&self.root, dir);
        let hint = if raw.is_empty() { display_argv(&argv) } else { raw.join(" ") };
        Script {
            id: id_of(&[&self.root.to_string_lossy(), &where_, source, name]),
            name: name.into(),
            cmd: display_argv(&argv),
            // путь (обёртка gradlew, сам файл .bat) - это программа целиком: её ищут по пути, а не
            // по имени в PATH, иначе «нет программы gradlew» и кнопка запуска выключена
            tool: argv.first().map(|a| if a.contains(['/', '\\']) { a.clone() } else { explain::word_of(a) }),
            argv,
            group: source.to_string(),
            where_,
            source: source.into(),
            stage: self.kb.classify(name, &hint),
            danger: self.kb.is_danger_name(name),
            virtual_: false,
            raw,
            ..Default::default()
        }
    }

    fn label_for(&self, dir: &Path, file: &str) -> String {
        let r = rel(&self.root, dir);
        if r == "." {
            file.to_string()
        } else {
            r
        }
    }

    fn add_stack_dep(&mut self, dep: &str) {
        let d = dep.to_lowercase();
        if let Some(l) = self.kb.stacks["deps"][&d].as_str() {
            self.stacks.insert(l.to_string());
        }
        self.deps.insert(d);
    }

    // ---------- package.json ----------
    fn package_json(&mut self, dir: &Path) {
        let Some(t) = self.read(dir, "package.json") else { return };
        let j: Value = match serde_json::from_str(&t) {
            Ok(j) => j,
            Err(e) => {
                self.warnings.push(format!("{}/package.json не читается как JSON: {e}", rel(&self.root, dir)));
                return;
            }
        };
        if dir == self.root {
            self.name = j["name"].as_str().map(str::to_string);
            self.version = j["version"].as_str().map(str::to_string);
            self.description = j["description"].as_str().map(str::to_string);
            self.engines = j["engines"]["node"].as_str().map(str::to_string);
        }
        let label = self.label_for(dir, "package.json");
        if let Some(obj) = j["scripts"].as_object() {
            // строка скрипта - ключ «"имя":» внутри scripts, а не первое упоминание имени в файле
            // («"keywords": ["build"]» или «"name": "test"» выше скриптов давали чужую строку)
            let scripts_at = t.lines().position(|l| l.contains("\"scripts\"")).unwrap_or(0);
            for (name, v) in obj {
                let raw = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
                let key = format!("\"{name}\"");
                let line = t
                    .lines()
                    .enumerate()
                    .skip(scripts_at)
                    .find(|(_, l)| l.split_once(&key).is_some_and(|(_, after)| after.trim_start().starts_with(':')))
                    .map(|(i, _)| i as u32 + 1);
                let mut s = self
                    .mk(dir, "package.json", name, vec![self.pm.clone(), "run".into(), name.clone()], vec![raw.clone()])
                    .in_group(label.clone());
                s.desc = raw.chars().take(220).collect();
                s.line = line;
                self.scripts.push(s);
            }
        }
        for sect in ["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"] {
            if let Some(o) = j[sect].as_object() {
                for k in o.keys() {
                    self.add_stack_dep(k);
                }
            }
        }
    }

    // ---------- Makefile / justfile / Taskfile ----------
    fn makefile(&mut self, dir: &Path) {
        for f in ["Makefile", "makefile", "GNUmakefile"] {
            let Some(t) = self.read(dir, f) else { continue };
            self.stacks.insert("Make".into());
            let re = crate::re!(r"^([A-Za-z0-9][A-Za-z0-9_.\-/]*)\s*:([^=].*)?$");
            let lines: Vec<&str> = t.lines().collect();
            for (i, l) in lines.iter().enumerate() {
                let Some(m) = re.captures(l) else { continue };
                let name = m[1].to_string();
                if name.starts_with('.') {
                    continue;
                }
                let rest = m.get(2).map(|x| x.as_str()).unwrap_or("");
                let desc = rest
                    .split_once("##")
                    .map(|x| x.1.trim().to_string())
                    .or_else(|| {
                        if i > 0 {
                            lines[i - 1].strip_prefix('#').map(|c| c.trim_start_matches('#').trim().to_string())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();
                let recipe: Vec<String> = lines[i + 1..]
                    .iter()
                    .take_while(|l| l.starts_with('\t'))
                    .map(|l| l.trim().trim_start_matches(['@', '-']).to_string())
                    .take(6)
                    .collect();
                let mut s = self.mk(dir, f, &name, vec!["make".into(), name.clone()], recipe);
                s.desc = desc;
                s.line = Some(i as u32 + 1);
                self.scripts.push(s);
            }
            return;
        }
    }

    fn justfile(&mut self, dir: &Path) {
        let Some(t) = self.read(dir, "justfile").or_else(|| self.read(dir, "Justfile")) else {
            return;
        };
        self.stacks.insert("just".into());
        let re = crate::re!(r"^@?([A-Za-z_][\w-]*)(?:\s+[^:=]*)?:([^=]|$)");
        let lines: Vec<&str> = t.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            let Some(m) = re.captures(l) else { continue };
            let name = m[1].to_string();
            let desc = if i > 0 { lines[i - 1].strip_prefix('#').map(|c| c.trim().to_string()).unwrap_or_default() } else { String::new() };
            let recipe: Vec<String> = lines[i + 1..]
                .iter()
                .take_while(|l| l.starts_with([' ', '\t']) && !l.trim().is_empty())
                .map(|l| l.trim().trim_start_matches('@').to_string())
                .take(6)
                .collect();
            let mut s = self.mk(dir, "justfile", &name, vec!["just".into(), name.clone()], recipe);
            s.desc = desc;
            s.line = Some(i as u32 + 1);
            self.scripts.push(s);
        }
    }

    fn taskfile(&mut self, dir: &Path) {
        for f in ["Taskfile.yml", "Taskfile.yaml"] {
            let Some(t) = self.read(dir, f) else { continue };
            self.stacks.insert("Taskfile".into());
            let lines: Vec<&str> = t.lines().collect();
            let Some(start) = lines.iter().position(|l| l.trim_end() == "tasks:") else {
                return;
            };
            let mut cur: Option<Script> = None;
            for (i, l) in lines.iter().enumerate().skip(start + 1) {
                if !l.starts_with(' ') && !l.trim().is_empty() {
                    break;
                }
                let ind = l.len() - l.trim_start().len();
                let tl = l.trim();
                if ind == 2 && tl.ends_with(':') {
                    if let Some(s) = cur.take() {
                        self.scripts.push(s);
                    }
                    let name = tl.trim_end_matches(':').to_string();
                    let mut s = self.mk(dir, f, &name, vec!["task".into(), name.clone()], vec![]);
                    s.line = Some(i as u32 + 1);
                    cur = Some(s);
                } else if let Some(s) = cur.as_mut() {
                    if let Some(d) = tl.strip_prefix("desc:") {
                        s.desc = d.trim().trim_matches(['"', '\'']).to_string();
                    } else if ind >= 6 && tl.starts_with("- ") && s.raw.len() < 6 {
                        s.raw.push(tl[2..].trim_start_matches("cmd:").trim().trim_matches(['"', '\'']).to_string());
                    }
                }
            }
            if let Some(s) = cur.take() {
                self.scripts.push(s);
            }
            return;
        }
    }

    fn pyproject_scripts(&mut self, dir: &Path) {
        let Some(t) = self.read(dir, "pyproject.toml") else { return };
        if dir == self.root {
            let get = |k: &str| {
                crate::util::cached_re(&format!(r#"(?m)^\s*{k}\s*=\s*"([^"]+)""#)).and_then(|r| r.captures(&t).map(|c| c[1].to_string()))
            };
            if self.name.is_none() {
                self.name = get("name");
            }
            if self.description.is_none() {
                self.description = get("description");
            }
        }
        if let Some(c) = crate::re!(r"(?s)\[(?:project|tool\.poetry)\.scripts\](.*?)(?:\n\[|\z)").captures(&t) {
            let label = self.label_for(dir, "pyproject.toml");
            for l in c[1].lines() {
                if let Some(m) = crate::re!(r#"^\s*([\w.-]+)\s*=\s*"([^"]+)""#).captures(l) {
                    let mut s = self.mk(dir, "pyproject.toml", &m[1], vec![m[1].to_string()], vec![]).in_group(label.clone());
                    s.desc = format!("Точка входа: {}", &m[2]);
                    self.scripts.push(s);
                }
            }
        }
        for d in pyproject_deps(&t) {
            self.add_stack_dep(&d);
        }
    }

    fn composer_scripts(&mut self, dir: &Path) {
        let Some(t) = self.read(dir, "composer.json") else { return };
        let Ok(j) = serde_json::from_str::<Value>(&t) else { return };
        if let Some(o) = j["scripts"].as_object() {
            for (n, v) in o {
                let raw = match v {
                    Value::Array(a) => a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" && "),
                    other => other.as_str().unwrap_or("").to_string(),
                };
                let mut s = self.mk(dir, "composer.json", n, vec!["composer".into(), "run-script".into(), n.clone()], vec![raw.clone()]);
                s.desc = raw;
                self.scripts.push(s);
            }
        }
    }

    // ---------- зависимости других языков (для меток и уроков) ----------
    fn other_deps(&mut self, dir: &Path) {
        if let Some(t) = self.read(dir, "Cargo.toml") {
            if dir == self.root {
                // имя, версия и описание - из [package] или общего [workspace.package]
                let mut sec = false;
                for l in t.lines().map(str::trim) {
                    if l.starts_with('[') {
                        // «[ package ] # комментарий» - тоже [package]
                        let h: String = l.split('#').next().unwrap_or("").chars().filter(|c| !c.is_whitespace()).collect();
                        sec = h == "[package]" || h == "[workspace.package]";
                        continue;
                    }
                    if !sec {
                        continue;
                    }
                    // строка TOML: "..." с экранированием до первой неэкранированной кавычки или '...' как есть
                    let re = crate::re!(r#"^(name|version|description)\s*=\s*(?:"((?:[^"\\]|\\.)*)"|'([^']*)')"#);
                    if let Some(c) = re.captures(l) {
                        let v = match (c.get(2), c.get(3)) {
                            (Some(b), _) => b.as_str().replace("\\\"", "\"").replace("\\\\", "\\"),
                            (_, Some(lit)) => lit.as_str().to_string(),
                            _ => continue,
                        };
                        if v.is_empty() {
                            continue;
                        }
                        let v = Some(v);
                        match &c[1] {
                            "name" if self.name.is_none() => self.name = v,
                            "version" if self.version.is_none() => self.version = v,
                            "description" if self.description.is_none() => self.description = v,
                            _ => {}
                        }
                    }
                }
            }
            let mut on = false;
            for l in t.lines() {
                let l = l.trim();
                if l.starts_with('[') {
                    let h = l.split('#').next().unwrap_or("").trim().trim_matches(['[', ']']);
                    // «[dependencies.serde]» - таблица одной зависимости: её имя в заголовке,
                    // а ключи внутри (version, features) - не зависимости
                    match h.rsplit_once("dependencies.") {
                        Some((_, name)) if !name.is_empty() => {
                            self.add_stack_dep(name.trim_matches(['"', '\'']));
                            on = false;
                        }
                        _ => on = h.ends_with("dependencies"),
                    }
                    continue;
                }
                if on {
                    if let Some((k, _)) = l.split_once('=') {
                        let k = k.trim().trim_matches(['"', '\'']);
                        if !k.is_empty() && !k.starts_with('#') {
                            self.add_stack_dep(k);
                        }
                    }
                }
            }
        }
        let list = self.listing(dir);
        for e in &list.entries {
            let low = e.name.to_lowercase();
            // пакеты .NET: в проекте и в общем Directory.Packages.props (центральные версии);
            // Include не обязательно первый атрибут: «<PackageReference Version="1" Include="x" />»
            if low.ends_with(".csproj") || low.ends_with(".fsproj") || low == "directory.packages.props" {
                if let Some(t) = read_head(&e.path, 400_000) {
                    for m in crate::re!(r#"<Package(?:Reference|Version)\b[^>]*?\sInclude="([^"]+)""#).captures_iter(&t) {
                        self.add_stack_dep(&m[1]);
                    }
                }
            }
        }
        if let Some(t) = self.read(dir, "requirements.txt") {
            for l in t.lines() {
                if let Some(m) = crate::re!(r"^\s*([A-Za-z0-9_.\-]+)").captures(l) {
                    self.add_stack_dep(&m[1]);
                }
            }
        }
        if let Some(t) = self.read(dir, "go.mod") {
            for m in crate::re!(r"(?m)^\s*(?:require\s+)?([\w.\-]+/[\w.\-/]+)\s+v").captures_iter(&t) {
                let last = m[1].rsplit('/').next().unwrap_or("").to_string();
                self.add_stack_dep(&last);
            }
        }
    }

    // ---------- стандартные команды по манифестам (из knowledge/manifests.json) ----------
    fn manifests(&mut self, dir: &Path) {
        // база живёт дольше разбора: правила читаются по ссылке, без копии на каждую папку
        let kb = self.kb;
        let kinds = kb.manifests["kinds"].as_array().map(Vec::as_slice).unwrap_or_default();
        let list = self.listing(dir);
        let names: Vec<&str> = list.entries.iter().filter(|e| e.file).map(|e| e.name.as_str()).collect();
        let root = self.root.clone();
        for k in kinds {
            let id = k["id"].as_str().unwrap_or("");
            let strs = |v: &Value| {
                v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect::<Vec<_>>()).unwrap_or_default()
            };
            let mut hits: Vec<String> = Vec::new();
            for f in strs(&k["files"]) {
                if names.iter().any(|n| *n == f) {
                    hits.push(f);
                    break;
                }
            }
            if hits.is_empty() {
                if let (Some(f), Some(text)) = (k["alsoIfContains"]["file"].as_str(), k["alsoIfContains"]["text"].as_str()) {
                    if self.read(dir, f).is_some_and(|t| t.contains(text)) {
                        hits.push(f.to_string());
                    }
                }
            }
            let mut by_ext = false;
            if hits.is_empty() {
                for exts in [strs(&k["ext"]), strs(&k["fallbackExt"])] {
                    let found: Vec<String> = names
                        .iter()
                        .filter(|n| exts.iter().any(|e| n.to_lowercase().ends_with(e.as_str())))
                        .map(|n| n.to_string())
                        .collect();
                    if !found.is_empty() {
                        hits = found;
                        by_ext = true;
                        break;
                    }
                }
            }
            if hits.is_empty() {
                continue;
            }
            if let Some(text) = k["contains"].as_str() {
                if !self.read(dir, &hits[0]).is_some_and(|t| t.contains(text)) {
                    continue;
                }
            }
            // член workspace: стандартные команды уже есть у корня
            if k["skipWorkspaceMembers"].as_bool() == Some(true)
                && dir != root
                && self.read(&root, &hits[0]).is_some_and(|t| t.contains("[workspace]"))
            {
                continue;
            }
            if let Some(s) = k["stack"].as_str() {
                self.stacks.insert(s.to_string());
            }
            let bin = strs(&k["wrapper"])
                .into_iter()
                .find(|w| names.iter().any(|n| n == w))
                .map(|w| dir.join(w).to_string_lossy().into_owned())
                .or_else(|| k["bin"].as_str().map(str::to_string))
                .unwrap_or_default();
            let services = if id == "compose" { compose_services(&dir.join(&hits[0])) } else { vec![] };
            if id == "compose" && dir == self.root {
                self.compose = Some(services.clone());
            }
            let desc = k["desc"]
                .as_str()
                .filter(|_| !services.is_empty())
                .map(|d| d.replace("{services}", &services[..services.len().min(12)].join(", ")))
                .unwrap_or_default();
            let solution_exts = strs(&k["ext"]);
            let is_solution = |f: &str| solution_exts.iter().any(|e| f.to_lowercase().ends_with(e.as_str()));
            // проект внутри решения (оно рядом выше по папкам): сборку, тесты и восстановление
            // делает решение, у проекта остаются только свои команды (dotnet run)
            let member = by_ext
                && k["solutionMembers"].as_str() == Some("projectOnly")
                && !hits.iter().any(|f| is_solution(f))
                && self.solution_above(dir, &solution_exts);
            for file in &hits {
                let is_project = by_ext && !is_solution(file);
                let group = if by_ext {
                    if rel(&self.root, dir) == "." {
                        file.clone()
                    } else {
                        format!("{}/{}", rel(&self.root, dir), file)
                    }
                } else {
                    self.label_for(dir, file)
                };
                for c in k["commands"].as_array().into_iter().flatten() {
                    let project_only = c["projectOnly"].as_bool() == Some(true);
                    if (project_only && !is_project) || (member && !project_only) {
                        continue;
                    }
                    // «только у программы»: dotnet run у библиотеки или проекта тестов не запустится
                    if let Some(need) = c["ifFileContains"].as_array() {
                        let text = self.read(dir, file).unwrap_or_default().to_lowercase();
                        if !need.iter().filter_map(Value::as_str).any(|n| text.contains(&n.to_lowercase())) {
                            continue;
                        }
                    }
                    let argv: Vec<String> =
                        strs(&c["argv"]).into_iter().map(|a| a.replace("{file}", file).replace("{bin}", &bin)).collect();
                    let name = c["name"].as_str().unwrap_or("").to_string();
                    let shown = vec![display_argv(&argv)];
                    let mut s = self.mk(dir, file, &name, argv, shown).in_group(group.clone()).at_stage(c["stage"].as_str()).standard();
                    s.desc = desc.clone();
                    self.scripts.push(s);
                }
            }
        }
    }

    /// Есть ли решение (файл с расширением из `exts`) в папках выше `dir` до корня проекта включительно.
    fn solution_above(&mut self, dir: &Path, exts: &[String]) -> bool {
        let root = self.root.clone();
        let mut cur = dir.parent();
        while let Some(d) = cur.filter(|d| d.starts_with(&root)) {
            let list = self.listing(d);
            if list.entries.iter().any(|e| e.file && exts.iter().any(|x| e.name.to_lowercase().ends_with(x.as_str()))) {
                return true;
            }
            if d == root {
                break;
            }
            cur = d.parent();
        }
        false
    }

    // ---------- файлы-скрипты ----------
    fn script_files(&mut self) {
        let dirs = self.kb.str_list("scriptDirs");
        let skip = self.kb.str_list("skipDirs");
        let kb = self.kb;
        let runners = &kb.manifests["scriptFiles"];
        let bash = runner::git_bash().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "bash".into());
        let mut count = 0;
        let mut stack = vec![(self.root.clone(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            let list = self.listing(&dir);
            for e in list.sorted() {
                if count > 200 {
                    return;
                }
                let p = e.path.clone();
                let name = e.name.clone();
                if e.dir {
                    if depth < 3 && !skip.contains(&name) && (depth > 0 || dirs.contains(&name)) {
                        stack.push((p, depth + 1));
                    }
                    continue;
                }
                let ext = p.extension().map(|x| format!(".{}", x.to_string_lossy().to_lowercase())).unwrap_or_default();
                let Some(tpl) = runners[&ext].as_array() else { continue };
                if (ext == ".py" && depth == 0) || skip_file(kb, &name) {
                    continue;
                }
                count += 1;
                let full = p.to_string_lossy().into_owned();
                let argv: Vec<String> =
                    tpl.iter().filter_map(|v| v.as_str()).map(|a| a.replace("{file}", &full).replace("{bash}", &bash)).collect();
                let r = rel(&self.root, &dir);
                let stem = name.rsplit_once('.').map(|x| x.0).unwrap_or(&name).to_string();
                let stage = self.kb.classify(&stem, &r);
                let src = rel(&self.root, &p);
                let mut s = self
                    .mk(&dir, &src, &name, argv, vec![])
                    .in_group(if r == "." { "файлы в корне".into() } else { format!("файлы: {r}") })
                    .at_stage(Some(&stage));
                s.file_rel = Some(src);
                self.scripts.push(s);
            }
        }
    }

    fn git_scripts(&mut self) {
        let kb = self.kb;
        for c in kb.manifests["git"].as_array().into_iter().flatten() {
            let argv: Vec<String> =
                c["argv"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
            let root = self.root.clone();
            let s = self
                .mk(&root, "git", c["name"].as_str().unwrap_or(""), argv.clone(), vec![display_argv(&argv)])
                .in_group("git")
                .at_stage(c["stage"].as_str())
                .standard();
            self.scripts.push(s);
        }
    }
}

/// Зависимости из pyproject.toml: списки PEP 621 (`dependencies`, группы
/// `[project.optional-dependencies]` и `[dependency-groups]`) и таблицы Poetry
/// (`[tool.poetry.dependencies]`, `[tool.poetry.group.<имя>.dependencies]`).
/// Конец списка - «]» вне кавычек: `"uvicorn[standard]>=0.30"` список не обрывает.
fn pyproject_deps(t: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut section = String::new();
    let mut depth = 0usize;
    let name = crate::re!(r#"^\s*["']([A-Za-z0-9][A-Za-z0-9_.\-]*)"#);
    for l in t.lines() {
        let line = l.split(" #").next().unwrap_or(l).trim();
        if depth == 0 {
            if let Some(h) = line.strip_prefix('[').filter(|_| !line.contains('=')) {
                section = h.trim_end_matches(']').trim_matches(['[', ']', ' ']).to_string();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else { continue };
            let key = key.trim().trim_matches(['"', '\'']);
            let poetry = section == "tool.poetry.dependencies"
                || section == "tool.poetry.dev-dependencies"
                || (section.starts_with("tool.poetry.group.") && section.ends_with(".dependencies"));
            if poetry {
                if key != "python" && !key.is_empty() {
                    out.push(key.to_string());
                }
                continue;
            }
            let list = (section == "project" && key == "dependencies")
                || section == "project.optional-dependencies"
                || section == "dependency-groups";
            if !list {
                continue;
            }
            let Some(rest) = value.trim().strip_prefix('[') else { continue };
            depth = 1;
            scan_list(rest, &mut depth, &mut out, name);
        } else {
            scan_list(line, &mut depth, &mut out, name);
        }
    }
    out
}

/// Строки внутри списка TOML: имя пакета из каждой строки в кавычках; `depth` - вложенность «[]» вне кавычек.
fn scan_list(s: &str, depth: &mut usize, out: &mut Vec<String>, name: &regex::Regex) {
    let mut q: Option<char> = None;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match (q, c) {
            (Some(qc), _) if c == qc => q = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => {
                q = Some(c);
                start = i;
            }
            (None, '[') => *depth += 1,
            (None, ']') => {
                *depth -= 1;
                if *depth == 0 {
                    return;
                }
            }
            _ => {}
        }
        if q.is_some() && i == start {
            if let Some(m) = name.captures(&s[i..]) {
                out.push(m[1].to_string());
            }
        }
    }
}

/// Служебный файл с расширением скрипта (`__init__.py`): его не запускают сами (manifests.scriptFilesSkip).
fn skip_file(kb: &Kb, name: &str) -> bool {
    kb.manifests["scriptFilesSkip"]["names"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|n| n.as_str().is_some_and(|n| n.eq_ignore_ascii_case(name)))
}

fn compose_services(file: &Path) -> Vec<String> {
    let Some(t) = read_text(file) else { return vec![] };
    let lines: Vec<&str> = t.lines().collect();
    let Some(i) = lines.iter().position(|l| l.trim_end() == "services:") else {
        return vec![];
    };
    let mut out = Vec::new();
    for l in &lines[i + 1..] {
        if !l.starts_with(' ') && !l.trim().is_empty() {
            break;
        }
        let ind = l.len() - l.trim_start().len();
        let t = l.trim();
        if ind == 2 && t.ends_with(':') && !t.starts_with('#') {
            out.push(t.trim_end_matches(':').to_string());
        }
    }
    out
}

impl Scan<'_> {
    fn candidate_dirs(&mut self, deep: bool) -> Vec<PathBuf> {
        let skip = self.kb.str_list("skipDirs");
        let groups = self.kb.str_list("groupDirs");
        let root = self.root.clone();
        let mut out = vec![root.clone()];
        let top = self.listing(&root);
        for e in top.sorted() {
            if !e.dir || skip.contains(&e.name) || e.name.starts_with('.') {
                continue;
            }
            out.push(e.path.clone());
            if deep || groups.contains(&e.name) {
                let sub = self.listing(&e.path);
                for e2 in sub.sorted() {
                    if e2.dir && !skip.contains(&e2.name) && !e2.name.starts_with('.') {
                        out.push(e2.path.clone());
                    }
                }
            }
        }
        out.truncate(160);
        out
    }
}

fn readme(root: &Path, list: &Listing) -> (Option<String>, Option<String>) {
    let t = ["README.md", "Readme.md", "readme.md", "README.rst", "README.txt", "README"]
        .iter()
        .filter(|f| list.has(f))
        .find_map(|f| read_head(&root.join(f), 20_000));
    let Some(t) = t else { return (None, None) };
    let mut title = None;
    let mut para: Vec<String> = Vec::new();
    let mut code = false;
    let skip = crate::re!(r"^(#|!\[|\[!\[|<|---|===|>|\||[-*]\s|\d+\.\s)");
    for l in t.lines() {
        let s = l.trim();
        if s.starts_with("```") {
            code = !code;
            if !para.is_empty() {
                break;
            }
            continue;
        }
        if code {
            continue;
        }
        if title.is_none() && s.starts_with("# ") {
            title = Some(s.trim_start_matches('#').trim().replace(['`', '*', '_'], ""));
            continue;
        }
        if s.is_empty() {
            if !para.is_empty() {
                break;
            }
            continue;
        }
        if skip.is_match(s) {
            if !para.is_empty() {
                break;
            }
            continue;
        }
        para.push(s.to_string());
        if para.join(" ").chars().count() > 400 {
            break;
        }
    }
    let link = crate::re!(r"\[([^\]]+)\]\([^)]+\)");
    let ex = link.replace_all(&para.join(" "), "$1").replace(['`', '*'], "").replace(['\u{2014}', '\u{2013}'], "-");
    let ex = if ex.chars().count() > 400 { format!("{}…", ex.chars().take(397).collect::<String>()) } else { ex };
    (title, (!ex.is_empty()).then_some(ex))
}

fn docs(list: &Listing, kb: &Kb) -> Vec<Doc> {
    let exts = kb.str_list("docExtensions");
    let mut out = Vec::new();
    for e in list.sorted() {
        let n = e.name.clone();
        let low = n.to_lowercase();
        if e.file {
            let (stem, ext) = match low.rfind('.') {
                Some(i) if i > 0 => (&low[..i], &low[i..]),
                _ => (low.as_str(), ""),
            };
            if let Some(r) = kb.doc_files.iter().find(|r| r.name == stem) {
                // открывать программой по умолчанию можно только текст: readme.js или license.bat - это не документы
                out.push(Doc {
                    rel: n.clone(),
                    name: n,
                    kind: "file".into(),
                    hint: r.hint.clone(),
                    count: 0,
                    openable: exts.iter().any(|x| x == ext),
                });
            }
        } else if let Some(r) = kb.doc_dirs.iter().find(|r| r.name == low) {
            let count = ls(&e.path).iter().filter(|x| x.file_type().is_ok_and(|t| t.is_file())).count() as u32;
            out.push(Doc { rel: n.clone(), name: format!("{n}/"), kind: "dir".into(), hint: r.hint.clone(), count, openable: true });
        }
    }
    out
}

fn node_version() -> Option<String> {
    static V: OnceLock<Option<String>> = OnceLock::new();
    V.get_or_init(|| {
        runner::capture(&["node".into(), "--version".into()], Path::new("."), Duration::from_secs(5))
            .ok()
            .filter(|c| c.code == 0)
            .map(|c| c.out.trim().trim_start_matches('v').to_string())
    })
    .clone()
}

fn health(sc: &Scan, list: &Listing, scripts: &[Script]) -> Health {
    let root = &sc.root;
    // .env только проверяется на наличие (файл, не ссылка); остальное - по списку папки, без запросов к диску
    let names: Vec<&str> = list.entries.iter().filter(|e| e.file).map(|e| e.name.as_str()).collect();
    let mut h = Health {
        env: names.iter().any(|n| *n == ".env" || *n == ".env.local"),
        env_example: names.iter().any(|n| [".env.example", ".env.sample", ".env.template", ".env.dist"].contains(n)),
        docker: list.has("Dockerfile") || sc.compose.is_some(),
        hooks: list.has(".githooks") || list.has(".husky") || list.has("lefthook.yml"),
        agent: list.has("CLAUDE.md") || list.has("AGENTS.md") || list.has(".claude"),
        ..Default::default()
    };
    if list.has("package.json") {
        let nvm = [".nvmrc", ".node-version"]
            .iter()
            .filter(|f| list.has(f))
            .find_map(|f| read_head(&root.join(f), 200))
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let want = nvm.or_else(|| sc.engines.clone());
        let have = node_version();
        let ok = want.as_ref().zip(have.as_ref()).and_then(|(w, hv)| {
            let major: u32 = crate::re!(r"(\d+)").captures(w)?[1].parse().ok()?;
            let hmaj: u32 = hv.split('.').next()?.parse().ok()?;
            // «>=20» - не ниже; «20», «v20.11», «^20», «20.x» - та же основная версия
            Some(if w.contains(">=") || w.contains('>') { hmaj >= major } else { hmaj == major })
        });
        h.node = Some(NodeHealth { want, have, ok });
        let nm = root.join("node_modules");
        h.deps = Some(if !nm.exists() {
            "missing".into()
        } else {
            let lock =
                ["pnpm-lock.yaml", "package-lock.json", "yarn.lock", "bun.lock"].iter().map(|f| mtime_ms(&root.join(f))).max().unwrap_or(0);
            let marker = NODE_MARKERS.iter().map(|f| mtime_ms(&nm.join(f))).max().unwrap_or(0);
            if lock > 0 && marker > 0 && lock > marker + 2000 {
                "stale".into()
            } else {
                "ok".into()
            }
        });
        h.install_script = scripts
            .iter()
            .find(|s| s.where_ == "." && (s.info.fx.iter().any(|f| f == "install") || s.name.contains("install")) && s.stage == "setup")
            .map(|s| s.id.clone());
    }
    if sc.stacks.contains("Python") {
        h.venv = Some(root.join(".venv").exists() || root.join("venv").exists());
    }
    h
}

/// Метки установки в node_modules: по их времени видно, когда ставили зависимости.
const NODE_MARKERS: [&str; 4] = [".modules.yaml", ".package-lock.json", ".yarn-state.yml", ".yarn-integrity"];

/// Файлы, которые разборщики этого модуля читают по имени сами (в нижнем регистре). Остальное,
/// за чем стоит следить, - из базы знаний: stacks.files, manifests.kinds, менеджеры пакетов.
const PARSED: [&str; 15] = [
    "makefile",
    "gnumakefile",
    "taskfile.yaml",
    "readme.md",
    "readme.rst",
    "readme.txt",
    "readme",
    ".nvmrc",
    "jenkinsfile",
    ".env",
    ".env.local",
    ".env.example",
    ".env.sample",
    ".env.template",
    ".env.dist",
];

/// Git внутри .git: эти файлы меняются при коммите, переключении ветки, fetch, слиянии.
const GIT_STATE: [&str; 6] = ["HEAD", "index", "FETCH_HEAD", "MERGE_HEAD", "ORIG_HEAD", "packed-refs"];

/// Что за событие файла в проекте: 1 - перечитать проект, 2 - обновить git, 0 - не важно.
/// Правила те же, что у разбора: манифесты из базы знаний, файлы-скрипты с расширением из
/// manifests.scriptFiles (в корне и в папках heuristics.scriptDirs), workflows GitHub.
pub fn watch_kind(root: &Path, path: &Path, kb: &Kb) -> u8 {
    let Ok(r) = path.strip_prefix(root) else { return 0 };
    let parts: Vec<String> = r.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
    let Some(first) = parts.first() else { return 0 };
    // именно папка .git, а не .gitlab-ci.yml или .githooks
    if first == ".git" {
        let rest = parts[1..].join("/");
        let hit = GIT_STATE.contains(&rest.as_str()) || rest.starts_with("refs/") || rest.starts_with("rebase-");
        return if hit { 2 } else { 0 };
    }
    let name = parts.last().cloned().unwrap_or_default();
    if first == "node_modules" {
        return u8::from(parts.len() == 2 && NODE_MARKERS.contains(&name.as_str()));
    }
    let skip = kb.str_list("skipDirs");
    if parts.iter().any(|c| skip.contains(c)) {
        return 0;
    }
    // ИИ-часть: .claude, инструкции, правила, хуки. 1 - признак в корне, 3 - правка детали
    let ai = crate::ai::watch_hit(&parts, kb);
    if ai > 0 {
        return ai;
    }
    let low = name.to_lowercase();
    if parts.len() >= 3 && first == ".github" && parts[1] == "workflows" {
        return 1;
    }
    let strs =
        |v: &Value| -> Vec<String> { v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_lowercase)).collect() };
    // имя или расширение (ключ с точкой в начале) - как в detect: «.csproj», «.gitlab-ci.yml»
    let by_name = |key: &str| {
        let k = key.to_lowercase();
        if k.starts_with('.') {
            low.ends_with(&k)
        } else {
            low == k
        }
    };
    let manifest = PARSED.contains(&low.as_str())
        || kb.stacks["files"].as_object().into_iter().flatten().any(|(k, _)| by_name(k))
        || kb.stacks["packageManagers"]["order"].as_array().into_iter().flatten().any(|p| p[0].as_str().is_some_and(by_name))
        || kb.manifests["kinds"].as_array().into_iter().flatten().any(|k| {
            strs(&k["files"]).contains(&low)
                || strs(&k["ext"]).iter().chain(strs(&k["fallbackExt"]).iter()).any(|e| low.ends_with(e.as_str()))
        });
    if manifest {
        return 1;
    }
    // файлы-скрипты: так же, как их ищет script_files
    let ext = Path::new(&low).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    if ext.is_empty() || kb.manifests["scriptFiles"].get(&ext).and_then(Value::as_array).is_none() || skip_file(kb, &name) {
        return 0;
    }
    let depth = parts.len() - 1;
    let script = match depth {
        0 => ext != ".py",
        1..=3 => kb.str_list("scriptDirs").contains(first),
        _ => false,
    };
    u8::from(script)
}

pub fn scan(root: &Path, opts: &ScanOpts) -> Result<Project, String> {
    scan_timed(root, opts).map(|x| x.0)
}

/// Части разбора и сколько каждая заняла - для замера (`examples/perf.rs`).
pub type Phases = Vec<(&'static str, Duration)>;

/// То же, что `scan`, плюс время каждой части разбора.
pub fn scan_timed(root: &Path, opts: &ScanOpts) -> Result<(Project, Phases), String> {
    let t0 = Instant::now();
    let mut phases: Phases = Vec::new();
    let mut lap = Instant::now();
    let mut mark = |name: &'static str| {
        phases.push((name, lap.elapsed()));
        lap = Instant::now();
    };
    if is_unc(&root.to_string_lossy()) {
        return Err("Сетевые папки (\\\\сервер\\...) не открываю: обращение к ним отдаёт данные входа Windows".into());
    }
    let meta = std::fs::metadata(root).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "Папка не найдена".to_string(),
        std::io::ErrorKind::PermissionDenied => "Нет доступа к папке".to_string(),
        _ => e.to_string(),
    })?;
    if !meta.is_dir() {
        return Err("Это файл, а не папка".into());
    }
    let kb = kb::get();
    mark("kb");
    // git - самая долгая часть (несколько процессов git по 40-60 мс на запуск в Windows) и ни от
    // чего в разборе не зависит: идёт в своём потоке, пока разбираются файлы и объяснения.
    // Проверки доверия внутри git::info те же: поток ничего не меняет в том, когда git зовут.
    let git_job = {
        let (r, hosts) = (root.to_path_buf(), opts.trusted_hosts.clone());
        std::thread::spawn(move || {
            let t = Instant::now();
            (git::info(&r, &hosts), t.elapsed())
        })
    };
    let root_list = Arc::new(Listing::read(root));
    // node --version (десятки миллисекунд на запуск) нужен только в конце, для health: спрашиваем заранее
    if root_list.has("package.json") {
        std::thread::spawn(|| {
            node_version();
        });
    }
    let order = kb.stacks["packageManagers"]["order"].as_array().map(Vec::as_slice).unwrap_or_default();
    let pm = order
        .iter()
        .find_map(|p| p[0].as_str().filter(|f| root_list.has(f)).and(p[1].as_str()))
        .or(kb.stacks["packageManagers"]["default"].as_str())
        .unwrap_or("npm")
        .to_string();
    let mut sc = Scan {
        root: root.to_path_buf(),
        kb: &kb,
        dirs: HashMap::from([(root.to_path_buf(), root_list.clone())]),
        pm,
        scripts: vec![],
        stacks: BTreeSet::new(),
        deps: BTreeSet::new(),
        found: BTreeSet::new(),
        warnings: vec![],
        name: None,
        version: None,
        description: None,
        engines: None,
        compose: None,
    };

    let detect = |sc: &mut Scan, d: &Path| {
        let list = sc.listing(d);
        for e in list.entries.iter().filter(|e| e.file) {
            let n = &e.name;
            if let Some(l) = kb.stacks["files"][n].as_str() {
                sc.stacks.insert(l.to_string());
            }
            let low = n.to_lowercase();
            for (ext, l) in kb.stacks["files"].as_object().into_iter().flatten() {
                if ext.starts_with('.') && low.ends_with(ext.as_str()) {
                    if let Some(l) = l.as_str() {
                        sc.stacks.insert(l.to_string());
                    }
                }
            }
            sc.found.insert(rel(&sc.root, &e.path));
        }
        sc.package_json(d);
        sc.makefile(d);
        sc.justfile(d);
        sc.taskfile(d);
        sc.pyproject_scripts(d);
        sc.composer_scripts(d);
        sc.other_deps(d);
        sc.manifests(d);
    };
    let first = sc.candidate_dirs(false);
    for d in &first {
        detect(&mut sc, d);
    }
    if sc.scripts.is_empty() && sc.stacks.is_empty() {
        for d in sc.candidate_dirs(true) {
            if !first.contains(&d) {
                detect(&mut sc, &d);
            }
        }
    }
    mark("manifests");
    sc.script_files();
    mark("script_files");
    let ci = ci_config::detect(root, &kb);
    mark("ci_config");
    let docs = docs(&root_list, &kb);
    let (rtitle, rdesc) = readme(root, &root_list);
    mark("docs");

    // порядок: npm-скрипты, Makefile и т.п., стандартные команды, файлы, git; корень раньше вложенных
    let rank = |s: &Script| -> u8 {
        if s.source == "package.json" {
            0
        } else if s.virtual_ && s.source != "git" {
            2
        } else if s.file_rel.is_some() {
            3
        } else if s.source == "git" {
            4
        } else {
            1
        }
    };
    let mut seen = BTreeSet::new();
    let mut scripts: Vec<(usize, Script)> =
        std::mem::take(&mut sc.scripts).into_iter().filter(|s| seen.insert(s.id.clone())).enumerate().collect();
    scripts.sort_by(|(i, a), (j, b)| {
        rank(a)
            .cmp(&rank(b))
            .then((a.where_ != ".").cmp(&(b.where_ != ".")))
            .then(if rank(a) == 3 { std::cmp::Ordering::Equal } else { a.where_.cmp(&b.where_) })
            .then(i.cmp(j))
    });
    let mut scripts: Vec<Script> = scripts.into_iter().map(|x| x.1).collect();

    // объяснения - параллельно, по кускам
    let mut pkg: HashMap<String, HashMap<String, String>> = HashMap::new();
    for s in scripts.iter().filter(|s| s.source == "package.json") {
        pkg.entry(s.where_.clone()).or_default().insert(s.name.clone(), s.raw.first().cloned().unwrap_or_default());
    }
    let ctx = Ctx { root, kb: &kb, pkg_scripts: &pkg, depth: 0 };
    explain_all(&mut scripts, root, &ctx);
    mark("explain");

    let (gi, git_took) = git_job.join().unwrap_or((None, Duration::ZERO));
    mark("git_wait");
    // git pull и другие из базы знаний запустили бы фильтры и хуки недоверенного репозитория.
    // Скрипты git в порядке показа всегда последние (ранг 4), поэтому их можно объяснить и
    // дописать в конец после того, как git ответил: порядок тот же, что при общей сортировке.
    if gi.as_ref().is_some_and(|g| !g.untrusted) {
        sc.git_scripts();
        let mut extra: Vec<Script> = std::mem::take(&mut sc.scripts).into_iter().filter(|s| seen.insert(s.id.clone())).collect();
        explain_all(&mut extra, root, &ctx);
        scripts.extend(extra);
    }
    mark("explain_git");

    let health = health(&sc, &root_list, &scripts);
    mark("health");
    let mut project = Project {
        path: root.to_string_lossy().into_owned(),
        name: sc.name.clone().or(rtitle).unwrap_or_else(|| root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
        folder: root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        version: sc.version.clone().unwrap_or_default(),
        description: sc.description.clone().or(rdesc).unwrap_or_default().replace(['\u{2014}', '\u{2013}'], "-"),
        package_manager: sc.pm.clone(),
        stacks: sc.stacks.iter().cloned().collect(),
        scripts,
        git: gi,
        ci,
        docs,
        health,
        warnings: sc.warnings.clone(),
        topics: vec![],
        ai_markers: crate::ai::markers(root, &|m| root_list.has(m), &kb),
        deps: sc.deps.iter().cloned().collect(),
        scanned_at: now_ms(),
        scan_ms: 0,
        cached: false,
        favorites: vec![],
    };
    project.topics = topics::detect(&project, &sc.found, &root_list.names, &kb);
    mark("topics");
    phases.push(("git_thread", git_took));
    project.scan_ms = t0.elapsed().as_millis() as u64;
    Ok((project, phases))
}

/// Объяснения - параллельно, по кускам. Этап «other» уточняется по найденным эффектам.
fn explain_all(scripts: &mut [Script], root: &Path, ctx: &Ctx) {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8);
    let chunk = scripts.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        for part in scripts.chunks_mut(chunk) {
            s.spawn(move || {
                for sc in part {
                    let cwd = root.join(if sc.where_ == "." { "" } else { &sc.where_ });
                    sc.info = explain::explain(sc, &cwd, ctx);
                    if sc.info.fx.iter().any(|f| f == "danger") {
                        sc.danger = true;
                    }
                    if sc.stage == "other" {
                        let fx = &sc.info.fx;
                        sc.stage = if sc.info.long {
                            "dev".into()
                        } else if fx.iter().any(|f| f == "install") {
                            "setup".into()
                        } else if fx.iter().any(|f| f == "docker" || f == "db") {
                            "ops".into()
                        // «только читает» у файла-скрипта - это умолчание интерпретатора (node, python), а не
                        // знание о файле: create-admin.js проверкой от этого не становится
                        } else if fx.iter().all(|f| f == "read")
                            && sc.info.known
                            && !fx.is_empty()
                            && sc.info.source != "file"
                            && !sc.info.fx_default
                        {
                            "check".into()
                        } else {
                            "other".into()
                        };
                    }
                    sc.raw.clear();
                }
            });
        }
    });
}
