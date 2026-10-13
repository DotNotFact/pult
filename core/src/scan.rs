//! Разбор папки проекта. Форматы файлов (package.json, Makefile...) разбирает код,
//! а всё, что касается смысла, - этапы, стандартные команды, метки технологий - берётся из базы знаний.
use crate::explain::{self, Ctx};
use crate::kb::{self, Kb};
use crate::model::*;
use crate::util::{display_argv, id_of, is_unc, ls, mtime_ms, now_ms, read_head, read_text, rel};
use crate::{ci_config, git, runner, topics};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[derive(Default, Clone)]
pub struct ScanOpts {
    pub trusted_hosts: Vec<String>,
}

struct Scan<'a> {
    root: PathBuf,
    kb: &'a Kb,
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

impl Scan<'_> {
    fn mk(&self, dir: &Path, source: &str, name: &str, argv: Vec<String>, group: Option<String>, raw: Vec<String>, stage: Option<&str>, virtual_: bool) -> Script {
        let where_ = rel(&self.root, dir);
        let hint = if raw.is_empty() { display_argv(&argv) } else { raw.join(" ") };
        Script {
            id: id_of(&[&self.root.to_string_lossy(), &where_, source, name]),
            name: name.into(),
            cmd: display_argv(&argv),
            tool: argv.first().map(|a| explain::word_of(a)),
            argv,
            group: group.unwrap_or_else(|| source.to_string()),
            where_,
            source: source.into(),
            stage: stage.map(str::to_string).unwrap_or_else(|| self.kb.classify(name, &hint)),
            danger: !virtual_ && self.kb.is_danger_name(name),
            virtual_,
            raw,
            ..Default::default()
        }
    }

    fn label_for(&self, dir: &Path, file: &str) -> String {
        let r = rel(&self.root, dir);
        if r == "." { file.to_string() } else { r }
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
        let p = dir.join("package.json");
        let Some(t) = read_text(&p) else { return };
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
            for (name, v) in obj {
                let raw = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
                let line = t.lines().position(|l| l.contains(&format!("\"{name}\""))).map(|i| i as u32 + 1);
                let mut s = self.mk(dir, "package.json", name, vec![self.pm.clone(), "run".into(), name.clone()], Some(label.clone()), vec![raw.clone()], None, false);
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
            let Some(t) = read_text(&dir.join(f)) else { continue };
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
                let desc = rest.split_once("##").map(|x| x.1.trim().to_string()).or_else(|| if i > 0 { lines[i - 1].strip_prefix('#').map(|c| c.trim_start_matches('#').trim().to_string()) } else { None }).unwrap_or_default();
                let recipe: Vec<String> = lines[i + 1..].iter().take_while(|l| l.starts_with('\t')).map(|l| l.trim().trim_start_matches(['@', '-']).to_string()).take(6).collect();
                let mut s = self.mk(dir, f, &name, vec!["make".into(), name.clone()], None, recipe, None, false);
                s.desc = desc;
                s.line = Some(i as u32 + 1);
                self.scripts.push(s);
            }
            return;
        }
    }

    fn justfile(&mut self, dir: &Path) {
        let Some(t) = read_text(&dir.join("justfile")).or_else(|| read_text(&dir.join("Justfile"))) else { return };
        self.stacks.insert("just".into());
        let re = crate::re!(r"^@?([A-Za-z_][\w-]*)(?:\s+[^:=]*)?:([^=]|$)");
        let lines: Vec<&str> = t.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            let Some(m) = re.captures(l) else { continue };
            let name = m[1].to_string();
            let desc = if i > 0 { lines[i - 1].strip_prefix('#').map(|c| c.trim().to_string()).unwrap_or_default() } else { String::new() };
            let recipe: Vec<String> = lines[i + 1..].iter().take_while(|l| l.starts_with([' ', '\t']) && !l.trim().is_empty()).map(|l| l.trim().trim_start_matches('@').to_string()).take(6).collect();
            let mut s = self.mk(dir, "justfile", &name, vec!["just".into(), name.clone()], None, recipe, None, false);
            s.desc = desc;
            s.line = Some(i as u32 + 1);
            self.scripts.push(s);
        }
    }

    fn taskfile(&mut self, dir: &Path) {
        for f in ["Taskfile.yml", "Taskfile.yaml"] {
            let Some(t) = read_text(&dir.join(f)) else { continue };
            self.stacks.insert("Taskfile".into());
            let lines: Vec<&str> = t.lines().collect();
            let Some(start) = lines.iter().position(|l| l.trim_end() == "tasks:") else { return };
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
                    let mut s = self.mk(&dir.to_path_buf(), f, &name, vec!["task".into(), name.clone()], None, vec![], None, false);
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
        let Some(t) = read_text(&dir.join("pyproject.toml")) else { return };
        if dir == self.root {
            let get = |k: &str| crate::util::cached_re(&format!(r#"(?m)^\s*{k}\s*=\s*"([^"]+)""#)).and_then(|r| r.captures(&t).map(|c| c[1].to_string()));
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
                    let mut s = self.mk(dir, "pyproject.toml", &m[1], vec![m[1].to_string()], Some(label.clone()), vec![], None, false);
                    s.desc = format!("Точка входа: {}", &m[2]);
                    self.scripts.push(s);
                }
            }
        }
        if let Some(c) = crate::re!(r"(?s)dependencies\s*=\s*\[(.*?)\]").captures(&t) {
            for m in crate::re!(r#"["']([A-Za-z0-9_.\-]+)"#).captures_iter(&c[1]) {
                self.add_stack_dep(&m[1]);
            }
        }
    }

    fn composer_scripts(&mut self, dir: &Path) {
        let Some(t) = read_text(&dir.join("composer.json")) else { return };
        let Ok(j) = serde_json::from_str::<Value>(&t) else { return };
        if let Some(o) = j["scripts"].as_object() {
            for (n, v) in o {
                let raw = match v {
                    Value::Array(a) => a.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(" && "),
                    other => other.as_str().unwrap_or("").to_string(),
                };
                let mut s = self.mk(dir, "composer.json", n, vec!["composer".into(), "run-script".into(), n.clone()], None, vec![raw.clone()], None, false);
                s.desc = raw;
                self.scripts.push(s);
            }
        }
    }

    // ---------- зависимости других языков (для меток и уроков) ----------
    fn other_deps(&mut self, dir: &Path) {
        if let Some(t) = read_text(&dir.join("Cargo.toml")) {
            let mut on = false;
            for l in t.lines() {
                let l = l.trim();
                if l.starts_with('[') {
                    on = l.contains("dependencies");
                    continue;
                }
                if on {
                    if let Some((k, _)) = l.split_once('=') {
                        let k = k.trim();
                        if !k.is_empty() && !k.starts_with('#') {
                            self.add_stack_dep(k);
                        }
                    }
                }
            }
        }
        for e in ls(dir) {
            let n = e.file_name().to_string_lossy().to_string();
            let low = n.to_lowercase();
            if low.ends_with(".csproj") || low.ends_with(".fsproj") {
                if let Some(t) = read_head(&e.path(), 400_000) {
                    for m in crate::re!(r#"PackageReference\s+Include="([^"]+)""#).captures_iter(&t) {
                        self.add_stack_dep(&m[1]);
                    }
                }
            }
        }
        if let Some(t) = read_text(&dir.join("requirements.txt")) {
            for l in t.lines() {
                if let Some(m) = crate::re!(r"^\s*([A-Za-z0-9_.\-]+)").captures(l) {
                    self.add_stack_dep(&m[1]);
                }
            }
        }
        if let Some(t) = read_text(&dir.join("go.mod")) {
            for m in crate::re!(r"(?m)^\s*(?:require\s+)?([\w.\-]+/[\w.\-/]+)\s+v").captures_iter(&t) {
                let last = m[1].rsplit('/').next().unwrap_or("").to_string();
                self.add_stack_dep(&last);
            }
        }
    }

    // ---------- стандартные команды по манифестам (из knowledge/manifests.json) ----------
    fn manifests(&mut self, dir: &Path) {
        let kinds = self.kb.manifests["kinds"].as_array().cloned().unwrap_or_default();
        let names: Vec<String> = ls(dir).iter().filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
        for k in kinds {
            let id = k["id"].as_str().unwrap_or("");
            let strs = |v: &Value| v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect::<Vec<_>>()).unwrap_or_default();
            let mut hits: Vec<String> = Vec::new();
            for f in strs(&k["files"]) {
                if names.iter().any(|n| n == &f) {
                    hits.push(f);
                    break;
                }
            }
            if hits.is_empty() {
                if let (Some(f), Some(text)) = (k["alsoIfContains"]["file"].as_str(), k["alsoIfContains"]["text"].as_str()) {
                    if read_text(&dir.join(f)).is_some_and(|t| t.contains(text)) {
                        hits.push(f.to_string());
                    }
                }
            }
            let mut by_ext = false;
            if hits.is_empty() {
                for exts in [strs(&k["ext"]), strs(&k["fallbackExt"])] {
                    let found: Vec<String> = names.iter().filter(|n| exts.iter().any(|e| n.to_lowercase().ends_with(e.as_str()))).cloned().collect();
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
                if !read_text(&dir.join(&hits[0])).is_some_and(|t| t.contains(text)) {
                    continue;
                }
            }
            if k["skipWorkspaceMembers"].as_bool() == Some(true) && dir != self.root {
                if read_text(&self.root.join(&hits[0])).is_some_and(|t| t.contains("[workspace]")) {
                    continue;
                }
            }
            if let Some(s) = k["stack"].as_str() {
                self.stacks.insert(s.to_string());
            }
            let bin = strs(&k["wrapper"]).into_iter().find(|w| names.iter().any(|n| n == w)).map(|w| dir.join(w).to_string_lossy().into_owned()).or_else(|| k["bin"].as_str().map(str::to_string)).unwrap_or_default();
            let services = if id == "compose" { compose_services(&dir.join(&hits[0])) } else { vec![] };
            if id == "compose" && dir == self.root {
                self.compose = Some(services.clone());
            }
            let desc = k["desc"].as_str().map(|d| d.replace("{services}", &services.iter().take(12).cloned().collect::<Vec<_>>().join(", "))).filter(|_| !services.is_empty()).unwrap_or_default();
            for file in &hits {
                let is_project = by_ext && !file.to_lowercase().ends_with(".sln") && !file.to_lowercase().ends_with(".slnx");
                let group = if by_ext { if rel(&self.root, dir) == "." { file.clone() } else { format!("{}/{}", rel(&self.root, dir), file) } } else { self.label_for(dir, file) };
                for c in k["commands"].as_array().cloned().unwrap_or_default() {
                    if c["projectOnly"].as_bool() == Some(true) && !is_project {
                        continue;
                    }
                    let argv: Vec<String> = strs(&c["argv"]).into_iter().map(|a| a.replace("{file}", file).replace("{bin}", &bin)).collect();
                    let name = c["name"].as_str().unwrap_or("").to_string();
                    let mut s = self.mk(dir, file, &name, argv.clone(), Some(group.clone()), vec![display_argv(&argv)], c["stage"].as_str(), true);
                    s.desc = desc.clone();
                    self.scripts.push(s);
                }
            }
        }
    }

    // ---------- файлы-скрипты ----------
    fn script_files(&mut self) {
        let dirs = self.kb.str_list("scriptDirs");
        let skip = self.kb.str_list("skipDirs");
        let runners = self.kb.manifests["scriptFiles"].clone();
        let bash = runner::git_bash().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "bash".into());
        let mut count = 0;
        let mut stack = vec![(self.root.clone(), 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            let mut entries = ls(&dir);
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                if count > 200 {
                    return;
                }
                let p = e.path();
                let name = e.file_name().to_string_lossy().to_string();
                if p.is_dir() {
                    if depth < 3 && !skip.contains(&name) && (depth > 0 || dirs.contains(&name)) {
                        stack.push((p, depth + 1));
                    }
                    continue;
                }
                let ext = p.extension().map(|x| format!(".{}", x.to_string_lossy().to_lowercase())).unwrap_or_default();
                let Some(tpl) = runners[&ext].as_array() else { continue };
                if ext == ".py" && depth == 0 {
                    continue;
                }
                count += 1;
                let full = p.to_string_lossy().into_owned();
                let argv: Vec<String> = tpl.iter().filter_map(|v| v.as_str()).map(|a| a.replace("{file}", &full).replace("{bash}", &bash)).collect();
                let r = rel(&self.root, &dir);
                let stem = name.rsplit_once('.').map(|x| x.0).unwrap_or(&name).to_string();
                let stage = self.kb.classify(&stem, &r);
                let src = rel(&self.root, &p);
                let mut s = self.mk(&dir, &src, &name, argv, Some(if r == "." { "файлы в корне".into() } else { format!("файлы: {r}") }), vec![], Some(&stage), false);
                s.file_rel = Some(src);
                self.scripts.push(s);
            }
        }
    }

    fn git_scripts(&mut self) {
        for c in self.kb.manifests["git"].as_array().cloned().unwrap_or_default() {
            let argv: Vec<String> = c["argv"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
            let root = self.root.clone();
            let s = self.mk(&root, "git", c["name"].as_str().unwrap_or(""), argv.clone(), Some("git".into()), vec![display_argv(&argv)], c["stage"].as_str(), true);
            self.scripts.push(s);
        }
    }
}

fn compose_services(file: &Path) -> Vec<String> {
    let Some(t) = read_text(file) else { return vec![] };
    let lines: Vec<&str> = t.lines().collect();
    let Some(i) = lines.iter().position(|l| l.trim_end() == "services:") else { return vec![] };
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

fn candidate_dirs(root: &Path, kb: &Kb, deep: bool) -> Vec<PathBuf> {
    let skip = kb.str_list("skipDirs");
    let groups = kb.str_list("groupDirs");
    let mut out = vec![root.to_path_buf()];
    let mut top = ls(root);
    top.sort_by_key(|e| e.file_name());
    for e in top {
        let n = e.file_name().to_string_lossy().to_string();
        if !e.path().is_dir() || skip.contains(&n) || n.starts_with('.') {
            continue;
        }
        out.push(e.path());
        if deep || groups.contains(&n) {
            let mut sub = ls(&e.path());
            sub.sort_by_key(|e| e.file_name());
            for e2 in sub {
                let n2 = e2.file_name().to_string_lossy().to_string();
                if e2.path().is_dir() && !skip.contains(&n2) && !n2.starts_with('.') {
                    out.push(e2.path());
                }
            }
        }
    }
    out.truncate(160);
    out
}

fn readme(root: &Path) -> (Option<String>, Option<String>) {
    let t = ["README.md", "Readme.md", "readme.md", "README.rst", "README.txt", "README"].iter().find_map(|f| read_head(&root.join(f), 20_000));
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
        if para.join(" ").len() > 400 {
            break;
        }
    }
    let link = crate::re!(r"\[([^\]]+)\]\([^)]+\)");
    let ex = link.replace_all(&para.join(" "), "$1").replace(['`', '*'], "").replace(['\u{2014}', '\u{2013}'], "-");
    let ex = if ex.chars().count() > 400 { format!("{}…", ex.chars().take(397).collect::<String>()) } else { ex };
    (title, (!ex.is_empty()).then_some(ex))
}

fn docs(root: &Path, kb: &Kb) -> Vec<Doc> {
    let exts = kb.str_list("docExtensions");
    let mut out = Vec::new();
    let mut top = ls(root);
    top.sort_by_key(|e| e.file_name());
    for e in &top {
        let n = e.file_name().to_string_lossy().to_string();
        let low = n.to_lowercase();
        if e.path().is_file() {
            let (stem, ext) = match low.rfind('.') {
                Some(i) if i > 0 => (&low[..i], &low[i..]),
                _ => (low.as_str(), ""),
            };
            if let Some(r) = kb.doc_files.iter().find(|r| r.name == stem) {
                // открывать программой по умолчанию можно только текст: readme.js или license.bat - это не документы
                out.push(Doc { rel: n.clone(), name: n, kind: "file".into(), hint: r.hint.clone(), count: 0, openable: exts.iter().any(|x| x == ext) });
            }
        } else if let Some(r) = kb.doc_dirs.iter().find(|r| r.name == low) {
            let count = ls(&e.path()).iter().filter(|x| x.path().is_file()).count() as u32;
            out.push(Doc { rel: n.clone(), name: format!("{n}/"), kind: "dir".into(), hint: r.hint.clone(), count, openable: true });
        }
    }
    out
}

fn node_version() -> Option<String> {
    static V: OnceLock<Option<String>> = OnceLock::new();
    V.get_or_init(|| runner::capture(&["node".into(), "--version".into()], Path::new("."), Duration::from_secs(5)).ok().filter(|c| c.code == 0).map(|c| c.out.trim().trim_start_matches('v').to_string())).clone()
}

fn health(sc: &Scan, scripts: &[Script]) -> Health {
    let root = &sc.root;
    let names: Vec<String> = ls(root).iter().filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
    let mut h = Health {
        env: names.iter().any(|n| n == ".env" || n == ".env.local"),
        env_example: names.iter().any(|n| [".env.example", ".env.sample", ".env.template", ".env.dist"].contains(&n.as_str())),
        docker: root.join("Dockerfile").exists() || sc.compose.is_some(),
        hooks: root.join(".githooks").exists() || root.join(".husky").exists() || root.join("lefthook.yml").exists(),
        agent: root.join("CLAUDE.md").exists() || root.join("AGENTS.md").exists() || root.join(".claude").exists(),
        ..Default::default()
    };
    if root.join("package.json").exists() {
        let nvm = read_head(&root.join(".nvmrc"), 200).or_else(|| read_head(&root.join(".node-version"), 200)).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
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
            let lock = ["pnpm-lock.yaml", "package-lock.json", "yarn.lock", "bun.lock"].iter().map(|f| mtime_ms(&root.join(f))).max().unwrap_or(0);
            let marker = [".modules.yaml", ".package-lock.json", ".yarn-state.yml", ".yarn-integrity"].iter().map(|f| mtime_ms(&nm.join(f))).max().unwrap_or(0);
            if lock > 0 && marker > 0 && lock > marker + 2000 { "stale".into() } else { "ok".into() }
        });
        h.install_script = scripts.iter().find(|s| s.where_ == "." && (s.info.fx.iter().any(|f| f == "install") || s.name.contains("install")) && s.stage == "setup").map(|s| s.id.clone());
    }
    if sc.stacks.contains("Python") {
        h.venv = Some(root.join(".venv").exists() || root.join("venv").exists());
    }
    h
}

pub fn scan(root: &Path, opts: &ScanOpts) -> Result<Project, String> {
    let t0 = Instant::now();
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
    let order = kb.stacks["packageManagers"]["order"].as_array().cloned().unwrap_or_default();
    let pm = order.iter().find_map(|p| p[0].as_str().filter(|f| root.join(f).exists()).and(p[1].as_str())).or(kb.stacks["packageManagers"]["default"].as_str()).unwrap_or("npm").to_string();
    let mut sc = Scan { root: root.to_path_buf(), kb: &kb, pm, scripts: vec![], stacks: BTreeSet::new(), deps: BTreeSet::new(), found: BTreeSet::new(), warnings: vec![], name: None, version: None, description: None, engines: None, compose: None };

    let detect = |sc: &mut Scan, d: &Path| {
        for e in ls(d) {
            if e.path().is_file() {
                let n = e.file_name().to_string_lossy().to_string();
                if let Some(l) = kb.stacks["files"][&n].as_str() {
                    sc.stacks.insert(l.to_string());
                }
                for (ext, l) in kb.stacks["files"].as_object().into_iter().flatten() {
                    if ext.starts_with('.') && n.to_lowercase().ends_with(ext.as_str()) {
                        if let Some(l) = l.as_str() {
                            sc.stacks.insert(l.to_string());
                        }
                    }
                }
                sc.found.insert(rel(&sc.root, &e.path()));
            }
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
    let first = candidate_dirs(root, &kb, false);
    for d in &first {
        detect(&mut sc, d);
    }
    if sc.scripts.is_empty() && sc.stacks.is_empty() {
        for d in candidate_dirs(root, &kb, true) {
            if !first.contains(&d) {
                detect(&mut sc, &d);
            }
        }
    }
    sc.script_files();
    let gi = git::info(root, &opts.trusted_hosts);
    if gi.is_some() {
        sc.git_scripts();
    }
    let ci = ci_config::detect(root, &kb);
    let docs = docs(root, &kb);
    let (rtitle, rdesc) = readme(root);

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
    let mut scripts: Vec<(usize, Script)> = std::mem::take(&mut sc.scripts).into_iter().filter(|s| seen.insert(s.id.clone())).enumerate().collect();
    scripts.sort_by(|(i, a), (j, b)| {
        rank(a).cmp(&rank(b)).then((a.where_ != ".").cmp(&(b.where_ != "."))).then(if rank(a) == 3 { std::cmp::Ordering::Equal } else { a.where_.cmp(&b.where_) }).then(i.cmp(j))
    });
    let mut scripts: Vec<Script> = scripts.into_iter().map(|x| x.1).collect();

    // объяснения - параллельно, по кускам
    let mut pkg: HashMap<String, HashMap<String, String>> = HashMap::new();
    for s in scripts.iter().filter(|s| s.source == "package.json") {
        pkg.entry(s.where_.clone()).or_default().insert(s.name.clone(), s.raw.first().cloned().unwrap_or_default());
    }
    let ctx = Ctx { root, kb: &kb, pkg_scripts: &pkg, depth: 0 };
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8);
    let chunk = scripts.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        for part in scripts.chunks_mut(chunk) {
            let ctx = &ctx;
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
                        } else if fx.iter().all(|f| f == "read") && sc.info.known && !fx.is_empty() {
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

    let health = health(&sc, &scripts);
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
        deps: sc.deps.iter().cloned().collect(),
        scanned_at: now_ms(),
        scan_ms: 0,
        cached: false,
        favorites: vec![],
    };
    project.topics = topics::detect(&project, &sc.found, &kb);
    project.scan_ms = t0.elapsed().as_millis() as u64;
    Ok(project)
}
