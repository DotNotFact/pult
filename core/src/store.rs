//! Настройки, недавние проекты, избранное, история запусков и кэш - JSON в data/.
//! Запись атомарная: во временный файл, затем переименование.
use crate::model::Project;
use crate::paths;
use crate::util::key_of;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub confirm_danger: bool,
    pub auto_refresh: bool,
    pub ci_autoload: bool,
    pub colors: bool,
    pub notify: bool,
    /// режим обучения: подробные пояснения, подсказки к терминам, «что внутри»
    pub learn: bool,
    /// минимальный интерфейс: без подсказок и пояснений, плотнее - для тех, кто уже привык (противоположность learn)
    pub minimal: bool,
    /// хосты CI, которым можно отдавать запросы (кроме gitlab.com и github.com)
    pub ci_hosts: Vec<String>,
    /// крестик окна прячет его в трей, а не закрывает Пульт (идущие запуски и сценарии продолжаются)
    pub close_to_tray: bool,
    /// что показывает виджет (узкое окно), по порядку: runs, quick, scenarios, ci, git
    pub widget: Vec<String>,
    /// виджет поверх всех окон
    pub widget_on_top: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            confirm_danger: true,
            auto_refresh: true,
            ci_autoload: true,
            colors: true,
            notify: true,
            learn: false,
            minimal: false,
            ci_hosts: vec![],
            close_to_tray: false,
            widget: ["runs", "quick", "scenarios", "ci"].map(String::from).to_vec(),
            widget_on_top: false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Recent {
    pub path: String,
    pub name: String,
    pub at: u64,
    pub pinned: bool,
    #[serde(skip_deserializing)]
    pub exists: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub recent: Vec<Recent>,
    pub last: Option<String>,
    pub favorites: HashMap<String, Vec<String>>,
    pub settings: Settings,
    /// корни репозиториев, которым пользователь доверил запуск git несмотря на опасные настройки.
    /// Не в Settings: их меняет команда settings из окна, а доверие ставится только кнопкой
    pub trusted_repos: Vec<String>,
    /// сценарии - свои цепочки команд, запускаются из окна и из трея
    pub scenarios: Vec<Scenario>,
}

/// Шаг сценария: скрипт проекта и дополнительные аргументы. Имя хранится рядом с id:
/// id может смениться после правки проекта, тогда шаг находится по имени.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Step {
    pub script_id: String,
    pub name: String,
    pub args: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Scenario {
    pub id: String,
    pub name: String,
    /// папка проекта
    pub project: String,
    pub steps: Vec<Step>,
    /// упавший шаг останавливает цепочку
    pub stop_on_error: bool,
    /// показывать в меню значка в трее
    pub in_tray: bool,
}
impl Default for Scenario {
    fn default() -> Self {
        Self { id: String::new(), name: String::new(), project: String::new(), steps: vec![], stop_on_error: true, in_tray: true }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct HistEntry {
    pub id: String,
    pub project: String,
    pub script_id: String,
    pub title: String,
    pub cmd: String,
    #[serde(rename = "where")]
    pub where_: String,
    pub args: String,
    pub status: String,
    pub code: Option<i32>,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub kind: String,
    pub tail: String,
}

fn file(name: &str) -> PathBuf {
    paths::data_dir().join(name)
}

pub(crate) fn read_json<T: for<'a> Deserialize<'a> + Default>(p: &Path) -> T {
    std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub(crate) fn write_json<T: Serialize>(p: &Path, v: &T) {
    let tmp = p.with_extension("tmp");
    if let Ok(t) = serde_json::to_string(v) {
        if std::fs::write(&tmp, t).is_ok() {
            let _ = std::fs::rename(&tmp, p);
        }
    }
}

static LOCK: Mutex<()> = Mutex::new(());

pub fn config() -> Config {
    let _g = LOCK.lock();
    read_json(&file("config.json"))
}

pub fn update_config(f: impl FnOnce(&mut Config)) -> Config {
    let _g = LOCK.lock();
    let mut c: Config = read_json(&file("config.json"));
    f(&mut c);
    write_json(&file("config.json"), &c);
    c
}

pub fn trusted_hosts() -> Vec<String> {
    let kb = crate::kb::get();
    let mut v = kb.str_list("ciHosts");
    v.extend(config().settings.ci_hosts);
    v
}

fn repo_key(work: &Path) -> String {
    let s = work.to_string_lossy().replace('/', "\\");
    s.trim_end_matches('\\').to_lowercase()
}

/// Пользователь доверил этот репозиторий (корень рабочей папки).
pub(crate) fn repo_trusted(work: &Path) -> bool {
    let k = repo_key(work);
    config().trusted_repos.iter().any(|r| repo_key(Path::new(r)) == k)
}

/// Доверять репозиторию: git для него запускается, даже если его настройки запускают программы.
pub fn trust_repo(work: &Path) {
    let k = repo_key(work);
    update_config(|c| {
        if !c.trusted_repos.iter().any(|r| repo_key(Path::new(r)) == k) {
            c.trusted_repos.push(work.to_string_lossy().into_owned());
        }
    });
}

/// Добавить или заменить сценарий (по id). Пустой id - новый сценарий.
pub fn save_scenario(mut s: Scenario) -> Scenario {
    if s.id.is_empty() {
        s.id = key_of(&format!("{}{}{}", s.project, s.name, crate::util::now_ms()));
    }
    s.name = s.name.trim().to_string();
    let out = s.clone();
    update_config(|c| match c.scenarios.iter_mut().find(|x| x.id == s.id) {
        Some(x) => *x = s,
        None => c.scenarios.push(s),
    });
    out
}

pub fn delete_scenario(id: &str) {
    update_config(|c| c.scenarios.retain(|x| x.id != id));
}

pub fn touch_recent(path: &str, name: &str) {
    update_config(|c| {
        let pinned = c.recent.iter().find(|r| r.path == path).is_some_and(|r| r.pinned);
        c.recent.retain(|r| r.path != path);
        c.recent.insert(0, Recent { path: path.into(), name: name.into(), at: crate::util::now_ms(), pinned, exists: true });
        c.recent.truncate(30);
        c.last = Some(path.into());
    });
}

// ---------- история ----------
pub fn history() -> Vec<HistEntry> {
    let _g = LOCK.lock();
    read_json(&file("history.json"))
}

pub fn push_history(e: HistEntry) {
    let _g = LOCK.lock();
    let mut h: Vec<HistEntry> = read_json(&file("history.json"));
    h.push(e);
    if h.len() > 600 {
        h.drain(..h.len() - 500);
    }
    write_json(&file("history.json"), &h);
}

pub fn clear_history(project: &str) {
    let _g = LOCK.lock();
    let mut h: Vec<HistEntry> = read_json(&file("history.json"));
    h.retain(|x| !x.project.eq_ignore_ascii_case(project));
    write_json(&file("history.json"), &h);
}

// ---------- кэш разбора ----------
fn scan_file(root: &str) -> PathBuf {
    paths::data_dir().join("cache").join(format!("scan-{}.json", key_of(root)))
}

pub fn cached_scan(root: &str) -> Option<Project> {
    let t = std::fs::read_to_string(scan_file(root)).ok()?;
    let p: Project = serde_json::from_str(&t).ok()?;
    p.path.eq_ignore_ascii_case(root).then_some(p)
}

pub fn save_scan(p: &Project) {
    write_json(&scan_file(&p.path), p);
}

// ---------- кэш карты ИИ-части ----------
fn ai_file(root: &str) -> PathBuf {
    paths::data_dir().join("cache").join(format!("ai-{}.json", key_of(root)))
}

pub fn cached_ai(root: &str) -> Option<crate::ai::AiMap> {
    let t = std::fs::read_to_string(ai_file(root)).ok()?;
    let m: crate::ai::AiMap = serde_json::from_str(&t).ok()?;
    m.path.eq_ignore_ascii_case(root).then_some(m)
}

pub fn save_ai(m: &crate::ai::AiMap) {
    write_json(&ai_file(&m.path), m);
}

// ---------- кэш CI ----------
pub fn ci_cache_get(key: &str) -> Option<(u64, Value)> {
    let m: HashMap<String, (u64, Value)> = read_json(&file("cache/ci.json"));
    m.get(key).cloned()
}

pub fn ci_cache_put(key: &str, v: &Value) {
    let _g = LOCK.lock();
    let p = file("cache/ci.json");
    let mut m: HashMap<String, (u64, Value)> = read_json(&p);
    m.insert(key.into(), (crate::util::now_ms(), v.clone()));
    if m.len() > 300 {
        let mut keys: Vec<(String, u64)> = m.iter().map(|(k, v)| (k.clone(), v.0)).collect();
        keys.sort_by_key(|x| x.1);
        for (k, _) in keys.into_iter().take(m.len() - 200) {
            m.remove(&k);
        }
    }
    write_json(&p, &m);
}
