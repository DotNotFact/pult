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
    /// хосты CI, которым можно отдавать запросы (кроме gitlab.com и github.com)
    pub ci_hosts: Vec<String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self { confirm_danger: true, auto_refresh: true, ci_autoload: true, colors: true, notify: true, learn: false, ci_hosts: vec![] }
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

pub fn read_json<T: for<'a> Deserialize<'a> + Default>(p: &Path) -> T {
    std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn write_json<T: Serialize>(p: &Path, v: &T) {
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
    (p.path.eq_ignore_ascii_case(root) && !p.scripts.is_empty() || p.path.eq_ignore_ascii_case(root)).then_some(p)
}

pub fn save_scan(p: &Project) {
    write_json(&scan_file(&p.path), p);
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
