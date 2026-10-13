//! Данные, которые ядро отдаёт окну и терминалу. Имена полей - camelCase, как в интерфейсе.
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    pub text: String,
    pub note: String,
    /// bin | sub | flag | arg | file | script
    pub kind: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct FlagInfo {
    pub flag: String,
    pub text: String,
    #[serde(default)]
    pub used: bool,
    /// kb | file | help
    #[serde(default)]
    pub from: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    pub summary: String,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub header: String,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub flags: Vec<FlagInfo>,
    #[serde(default)]
    pub fx: Vec<String>,
    #[serde(default)]
    pub long: bool,
    #[serde(default)]
    pub url: bool,
    #[serde(default)]
    pub known: bool,
    #[serde(default)]
    pub hook: Option<String>,
    #[serde(default)]
    pub refs: Vec<String>,
    #[serde(default)]
    pub fx_from_file: bool,
    /// какие программы участвуют (id из базы знаний)
    #[serde(default)]
    pub tools: Vec<String>,
    /// уроки, которые объясняют эти программы
    #[serde(default)]
    pub topics: Vec<String>,
    /// что происходит внутри (для режима обучения)
    #[serde(default)]
    pub how: Vec<String>,
    /// команда по частям
    #[serde(default)]
    pub parts: Vec<Part>,
    /// что это за программа
    #[serde(default)]
    pub about: String,
    /// откуда взято описание: kb | file | package | unknown
    #[serde(default)]
    pub source: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Script {
    pub id: String,
    pub name: String,
    /// программа и аргументы - запускается без оболочки
    pub argv: Vec<String>,
    /// как команду показать человеку
    pub cmd: String,
    #[serde(rename = "where")]
    pub where_: String,
    pub source: String,
    pub group: String,
    #[serde(default)]
    pub desc: String,
    /// текст команды для объяснения (package.json, рецепт Makefile)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub raw: Vec<String>,
    #[serde(default)]
    pub file_rel: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
    pub stage: String,
    #[serde(default)]
    pub danger: bool,
    #[serde(default)]
    pub tool: Option<String>,
    #[serde(default, rename = "virtual")]
    pub virtual_: bool,
    #[serde(default)]
    pub info: Info,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GitFile {
    pub path: String,
    pub code: String,
    pub label: String,
    pub staged: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Remote {
    pub name: String,
    pub url: String,
    pub host: Option<String>,
    pub slug: Option<String>,
    pub dup: bool,
    /// хост разрешён для запросов CI (heuristics.ciHosts или настройки)
    #[serde(default)]
    pub trusted: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Commit {
    pub hash: String,
    pub subject: String,
    pub author: String,
    pub date: String,
    pub refs: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GitInfo {
    pub toplevel: String,
    /// путь проекта относительно корня репозитория ("" - совпадают)
    pub prefix: String,
    pub branch: String,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub gone: bool,
    pub dirty: u32,
    pub files: Vec<GitFile>,
    pub stash: u32,
    pub tag: Option<String>,
    pub fetched_at: Option<u64>,
    pub op: Option<String>,
    pub remotes: Vec<Remote>,
    pub commits: Vec<Commit>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct CiJobCfg {
    pub name: String,
    pub stage: String,
    pub manual: bool,
    pub allow_failure: bool,
    pub needs: Vec<String>,
    pub desc: String,
    pub line: u32,
    pub only_main: bool,
    pub only_mr: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct CiStageCfg {
    pub name: String,
    pub sid: String,
    pub jobs: Vec<CiJobCfg>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct GhJobCfg {
    pub id: String,
    pub name: String,
    pub needs: Vec<String>,
    pub sid: String,
    pub line: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct CiConfig {
    pub kind: String,
    pub file: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub stages: Vec<CiStageCfg>,
    #[serde(default)]
    pub jobs: Vec<GhJobCfg>,
    #[serde(default)]
    pub triggers: Vec<String>,
    #[serde(default)]
    pub includes: Vec<String>,
    #[serde(default)]
    pub job_count: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Doc {
    pub name: String,
    pub rel: String,
    pub kind: String,
    pub hint: String,
    pub count: u32,
    /// можно открыть программой по умолчанию (расширение из белого списка)
    pub openable: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct NodeHealth {
    pub want: Option<String>,
    pub have: Option<String>,
    pub ok: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    pub env: bool,
    pub env_example: bool,
    pub docker: bool,
    pub hooks: bool,
    pub agent: bool,
    pub node: Option<NodeHealth>,
    /// ok | stale | missing
    pub deps: Option<String>,
    pub install_script: Option<String>,
    pub venv: Option<bool>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct TopicRef {
    pub id: String,
    pub title: String,
    pub color: String,
    pub icon: String,
    pub summary: String,
    /// почему урок показан: «найден Dockerfile»
    pub why: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub path: String,
    pub name: String,
    pub folder: String,
    pub version: String,
    pub description: String,
    pub package_manager: String,
    pub stacks: Vec<String>,
    pub scripts: Vec<Script>,
    pub git: Option<GitInfo>,
    pub ci: Vec<CiConfig>,
    pub docs: Vec<Doc>,
    pub health: Health,
    pub warnings: Vec<String>,
    pub topics: Vec<TopicRef>,
    /// зависимости (для определения уроков и меток)
    pub deps: Vec<String>,
    pub scanned_at: u64,
    pub scan_ms: u64,
    #[serde(default)]
    pub cached: bool,
    #[serde(default)]
    pub favorites: Vec<String>,
}
