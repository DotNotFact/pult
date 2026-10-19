//! База знаний: JSON из knowledge/. Читается с диска (можно править без пересборки),
//! а если папки нет - из копии, вшитой в exe при сборке.
use crate::paths;
use include_dir::{include_dir, Dir};
use regex::{Regex, RegexBuilder};
use serde::Deserialize;
use serde_json::Value;
use std::sync::{Arc, RwLock};

static EMBEDDED: Dir = include_dir!("$CARGO_MANIFEST_DIR/../knowledge");

#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Desc {
    pub say: Option<String>,
    pub part: Option<String>,
    #[serde(default)]
    pub fx: Vec<String>,
    #[serde(default)]
    pub fx_remove: Vec<String>,
    pub long: Option<bool>,
    pub url: Option<bool>,
    pub danger: Option<bool>,
    /// «что происходит внутри» именно у этой подкоманды (главнее how программы)
    #[serde(default)]
    pub how: Vec<String>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct SubRule {
    #[serde(rename = "match")]
    pub pat: String,
    #[serde(flatten)]
    pub d: Desc,
    /// компилируется при первом использовании - большинство программ проекту не нужны
    #[serde(skip)]
    pub re_cell: std::sync::OnceLock<Option<SubRe>>,
}

/// Собранное правило подкоманды. Правила пишутся в синтаксисе JS: почти все - обычные выражения,
/// их собирает крейт regex; с заглядыванием вперёд или назад - fancy-regex. Через fancy-regex
/// обычное правило стоило 70 КБ кучи и 0,8 мс сборки, напрямую - 26 КБ и 0,4 мс
/// (замер в docs/optimizations.md). Совпадения те же: fancy-regex сам отдаёт такие выражения regex.
#[derive(Clone, Debug)]
pub enum SubRe {
    Plain(Regex),
    Fancy(fancy_regex::Regex),
}

impl SubRe {
    pub fn build(pat: &str) -> Option<SubRe> {
        let p = format!("(?i){pat}");
        if let Ok(r) = Regex::new(&p) {
            return Some(SubRe::Plain(r));
        }
        fancy_regex::RegexBuilder::new(&p).backtrack_limit(100_000).build().ok().map(SubRe::Fancy)
    }

    pub fn is_match(&self, s: &str) -> bool {
        match self {
            SubRe::Plain(r) => r.is_match(s),
            // перебор упёрся в лимит - считаем, что не совпало
            SubRe::Fancy(r) => r.is_match(s).unwrap_or(false),
        }
    }

    /// Где кончается первое совпадение.
    pub fn find_end(&self, s: &str) -> Option<usize> {
        match self {
            SubRe::Plain(r) => r.find(s).map(|m| m.end()),
            SubRe::Fancy(r) => r.find(s).ok().flatten().map(|m| m.end()),
        }
    }
}

impl SubRule {
    pub fn re(&self) -> Option<&SubRe> {
        self.re_cell.get_or_init(|| SubRe::build(&self.pat)).as_ref()
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct FlagRule {
    pub flag: String,
    #[serde(default)]
    pub alias: Vec<String>,
    #[serde(default)]
    pub text: String,
    #[serde(flatten)]
    pub d: Desc,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub topic: String,
    pub bins: Vec<String>,
    #[serde(default)]
    pub about: String,
    #[serde(default)]
    pub how: Vec<String>,
    #[serde(default)]
    pub default: Desc,
    #[serde(default)]
    pub sub: Vec<SubRule>,
    #[serde(default)]
    pub flags: Vec<FlagRule>,
    #[serde(default)]
    pub help: Option<HelpSpec>,
    #[serde(default)]
    pub runs_file: bool,
}

/// Как спросить у программы справку: ключ («--help») или шаблоны по порядку
/// («{bin} {sub} -h», «{bin} help {sub}»). {bin} - слова программы, {sub} - её подкоманда из команды скрипта.
#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum HelpSpec {
    One(String),
    Many(Vec<String>),
}

impl Tool {
    /// Шаблоны справки по порядку. Просто ключ значит «сначала справка подкоманды, потом общая»;
    /// у интерпретаторов (runsFile) слово после программы - файл, а не подкоманда, - только общая.
    pub fn help_patterns(&self) -> Vec<String> {
        match &self.help {
            None => vec![],
            Some(HelpSpec::Many(v)) => v.clone(),
            Some(HelpSpec::One(f)) if f.contains("{bin}") => vec![f.clone()],
            Some(HelpSpec::One(f)) if self.runs_file => vec![format!("{{bin}} {f}")],
            Some(HelpSpec::One(f)) => vec![format!("{{bin}} {{sub}} {f}"), format!("{{bin}} {f}")],
        }
    }
}

#[derive(Deserialize, Clone, Debug, Default, serde::Serialize)]
pub struct Stage {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub hint: String,
    #[serde(default)]
    pub learn: String,
    #[serde(default, skip_serializing)]
    pub words: Vec<String>,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct DocRule {
    pub name: String,
    pub hint: String,
}

pub struct Kb {
    pub tools: Vec<Tool>,
    /// (слова команды, индекс программы), длинные префиксы первыми
    pub bins: Vec<(Vec<String>, usize)>,
    pub stages: Vec<Stage>,
    stage_res: Vec<(String, Regex)>,
    pub effects: Value,
    pub stacks: Value,
    pub doc_files: Vec<DocRule>,
    pub doc_dirs: Vec<DocRule>,
    pub heur: Value,
    pub manifests: Value,
    pub glossary: Value,
    pub icons: Value,
    pub ui: Value,
    /// шаблоны популярных сценариев (knowledge/scenarios.json)
    pub scenarios: Value,
    /// ИИ-часть проекта: виды деталей, слои, рассказ (knowledge/ai.json)
    pub ai: Value,
    /// шаблоны путей из ai.json, по которым слежение узнаёт правку ИИ-части
    pub ai_watch: Vec<crate::ai::Glob>,
    pub topics: Vec<Value>,
    pub wrappers: Vec<Regex>,
    pub skip_seg: Option<Regex>,
    pub file_fx: Vec<(String, Regex)>,
    pub danger_name: Option<Regex>,
    /// имена, которые похожи на опасные, но только пишут заметки или проверяют (release-notes, deploy-preview)
    pub danger_except: Option<Regex>,
    pub errors: Vec<String>,
    pub from_disk: bool,
}

fn re(s: &str) -> Option<Regex> {
    RegexBuilder::new(s).case_insensitive(true).multi_line(true).size_limit(1 << 22).build().ok()
}

fn load_text(rel: &str) -> Option<(String, bool)> {
    let p = paths::knowledge_dir().join(rel);
    if let Ok(t) = std::fs::read_to_string(&p) {
        return Some((t.trim_start_matches('\u{feff}').to_string(), true));
    }
    EMBEDDED.get_file(rel).and_then(|f| f.contents_utf8()).map(|t| (t.to_string(), false))
}

fn list(sub: &str) -> Vec<String> {
    let dir = paths::knowledge_dir().join(sub);
    let mut out: Vec<String> = std::fs::read_dir(&dir)
        .map(|r| {
            r.flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.ends_with(".json"))
                .map(|n| format!("{sub}/{n}"))
                .collect()
        })
        .unwrap_or_default();
    if out.is_empty() {
        if let Some(d) = EMBEDDED.get_dir(sub) {
            out = d.files().filter_map(|f| f.path().to_str().map(|s| s.replace('\\', "/"))).filter(|n| n.ends_with(".json")).collect();
        }
    }
    out.sort();
    out
}

fn json(rel: &str, errors: &mut Vec<String>) -> Value {
    match load_text(rel) {
        Some((t, _)) => serde_json::from_str(&t).unwrap_or_else(|e| {
            errors.push(format!("{rel}: {e}"));
            Value::Null
        }),
        None => Value::Null,
    }
}

impl Kb {
    pub fn load() -> Kb {
        let mut errors = Vec::new();
        let from_disk = paths::knowledge_dir().is_dir();
        let mut tools: Vec<Tool> = Vec::new();
        for f in list("tools") {
            #[derive(Deserialize)]
            struct File {
                tools: Vec<Tool>,
            }
            let Some((t, _)) = load_text(&f) else { continue };
            match serde_json::from_str::<File>(&t) {
                Ok(file) => tools.extend(file.tools),
                Err(e) => errors.push(format!("{f}: {e}")),
            }
        }
        let mut bins = Vec::new();
        for (i, t) in tools.iter().enumerate() {
            for b in &t.bins {
                bins.push((b.to_lowercase().split_whitespace().map(str::to_string).collect::<Vec<_>>(), i));
            }
        }
        bins.sort_by_key(|b| std::cmp::Reverse(b.0.len()));

        let st = json("stages.json", &mut errors);
        let stages: Vec<Stage> = serde_json::from_value(st.get("stages").cloned().unwrap_or(Value::Null)).unwrap_or_default();
        let mut by_pri: Vec<&Stage> = stages.iter().filter(|s| !s.words.is_empty()).collect();
        by_pri.sort_by_key(|s| s.priority);
        let stage_res = by_pri
            .iter()
            .filter_map(|s| {
                let alts = s.words.iter().map(|w| regex::escape(w)).collect::<Vec<_>>().join("|");
                re(&format!(r"(?:^|[:\-_ ./])(?:{alts})(?:$|[:\-_ ./])")).map(|r| (s.id.clone(), r))
            })
            .collect();

        let heur = json("heuristics.json", &mut errors);
        let wrappers =
            heur["wrappers"]["re"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().and_then(re)).collect()).unwrap_or_default();
        let skip_seg = heur["skipSegments"]["re"].as_str().and_then(re);
        let file_fx = heur["fileFx"]
            .as_object()
            .map(|o| o.iter().filter(|(k, _)| *k != "_").filter_map(|(k, v)| v.as_str().and_then(re).map(|r| (k.clone(), r))).collect())
            .unwrap_or_default();
        let danger_name = heur["danger"]["re"].as_str().and_then(re);
        let danger_except = heur["danger"]["except"].as_str().and_then(re);

        let docs = json("docs.json", &mut errors);
        let doc_files = serde_json::from_value(docs["files"].clone()).unwrap_or_default();
        let doc_dirs = serde_json::from_value(docs["dirs"].clone()).unwrap_or_default();

        // порядок уроков - из lessons.json (от основ к инфраструктуре); урок не из списка - в конец, по id
        let lessons: Vec<String> = json("lessons.json", &mut errors)["order"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        let mut topics: Vec<Value> = list("topics").iter().map(|f| json(f, &mut errors)).filter(|v| v.is_object()).collect();
        let rank = |t: &Value| {
            let id = t["id"].as_str().unwrap_or("");
            (lessons.iter().position(|l| l == id).unwrap_or(usize::MAX), id.to_string())
        };
        topics.sort_by_key(rank);

        let ai = json("ai.json", &mut errors);
        let ai_watch = crate::ai::compile_watch(&ai);
        Kb {
            tools,
            bins,
            stages,
            stage_res,
            effects: json("effects.json", &mut errors),
            stacks: json("stacks.json", &mut errors),
            doc_files,
            doc_dirs,
            glossary: json("glossary.json", &mut errors),
            icons: json("icons.json", &mut errors),
            ui: json("ui.json", &mut errors),
            scenarios: json("scenarios.json", &mut errors),
            ai,
            ai_watch,
            topics,
            wrappers,
            skip_seg,
            file_fx,
            danger_name,
            danger_except,
            heur,
            manifests: json("manifests.json", &mut errors),
            errors,
            from_disk,
        }
    }

    /// Этап скрипта по имени, потом по тексту команды.
    pub fn classify(&self, name: &str, hint: &str) -> String {
        for (id, r) in &self.stage_res {
            if r.is_match(name) {
                return id.clone();
            }
        }
        let head: String = hint.chars().take(80).collect();
        for (id, r) in &self.stage_res {
            if r.is_match(&head) {
                return id.clone();
            }
        }
        "other".into()
    }

    pub fn is_danger_name(&self, name: &str) -> bool {
        self.danger_name.as_ref().is_some_and(|r| r.is_match(name)) && !self.danger_except.as_ref().is_some_and(|r| r.is_match(name))
    }

    /// Программа по началу команды: самое длинное совпадение слов.
    pub fn match_tool(&self, words: &[String]) -> Option<(usize, usize)> {
        for (bw, i) in &self.bins {
            if words.len() >= bw.len() && bw.iter().zip(words).all(|(a, b)| a == b) {
                return Some((*i, bw.len()));
            }
        }
        None
    }

    pub fn tool(&self, id: &str) -> Option<&Tool> {
        self.tools.iter().find(|t| t.id == id)
    }

    pub fn str_list(&self, key: &str) -> Vec<String> {
        self.heur[key].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default()
    }

    pub fn topic(&self, id: &str) -> Option<&Value> {
        self.topics.iter().find(|t| t["id"] == id)
    }

    /// Всё, что нужно интерфейсу, одним объектом (кроме уроков целиком).
    pub fn client_bundle(&self) -> Value {
        // Map с preserve_order: интерфейс получает уроки в порядке lessons.json
        let mut topics = serde_json::Map::new();
        for t in &self.topics {
            if let Some(id) = t["id"].as_str() {
                topics.insert(id.into(), serde_json::json!({ "id": id, "title": t["title"], "color": t["color"], "icon": t["icon"], "summary": t["summary"], "level": t["level"] }));
            }
        }
        let tools: Vec<Value> =
            self.tools.iter().map(|t| serde_json::json!({ "id": t.id, "name": t.name, "topic": t.topic, "about": t.about })).collect();
        // термины из уроков - для подсказок в тексте (режим обучения)
        let mut terms: Vec<Value> = Vec::new();
        for t in &self.topics {
            for s in t["sections"].as_array().into_iter().flatten().filter(|s| s["type"] == "terms") {
                for x in s["items"].as_array().into_iter().flatten() {
                    terms.push(serde_json::json!({ "term": x["term"], "text": x["text"], "forms": x["forms"], "topic": t["id"] }));
                }
            }
        }
        serde_json::json!({
            "effects": self.effects, "stages": self.stages, "glossary": self.glossary, "icons": self.icons, "ui": self.ui, "scenarios": self.scenarios, "ai": self.ai,
            "topics": topics, "tools": tools, "topicTerms": terms, "console": self.heur["consoleErrors"], "errors": self.errors, "fromDisk": self.from_disk,
        })
    }
}

static KB: RwLock<Option<Arc<Kb>>> = RwLock::new(None);

pub fn get() -> Arc<Kb> {
    if let Some(k) = KB.read().ok().and_then(|g| g.clone()) {
        return k;
    }
    reload()
}

pub fn reload() -> Arc<Kb> {
    let k = Arc::new(Kb::load());
    if let Ok(mut g) = KB.write() {
        *g = Some(k.clone());
    }
    k
}
