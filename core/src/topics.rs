//! Какие уроки нужны этому проекту. Признаки - в поле detect каждого урока (knowledge/topics/*.json).
use crate::kb::Kb;
use crate::model::{Project, TopicRef};
use std::collections::{BTreeSet, HashSet};
use std::path::Path;

fn strs(v: &serde_json::Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default()
}

/// `root_names` - имена записей корня строчными (их уже прочитал разбор): «есть ли в корне файл»
/// решается по ним, без запроса к диску на каждый признак каждого урока.
pub fn detect(p: &Project, found: &BTreeSet<String>, root_names: &HashSet<String>, kb: &Kb) -> Vec<TopicRef> {
    let root = Path::new(&p.path);
    let at_root = |f: &str| if f.contains('/') { root.join(f).exists() } else { root_names.contains(&f.to_lowercase()) };
    let bins: BTreeSet<String> =
        p.scripts.iter().filter_map(|s| s.tool.clone()).chain(p.scripts.iter().flat_map(|s| s.info.tools.iter().cloned())).collect();
    let topics_used: BTreeSet<String> = p.scripts.iter().flat_map(|s| s.info.topics.iter().cloned()).collect();
    let mut out = Vec::new();
    for t in &kb.topics {
        let d = &t["detect"];
        let id = t["id"].as_str().unwrap_or("").to_string();
        let mut why: Option<String> = None;
        for f in strs(&d["files"]) {
            let hit = if f.starts_with('.') && !f.contains('/') && f.len() <= 8 && !at_root(&f) {
                found.iter().find(|x| x.to_lowercase().ends_with(&f.to_lowercase())).cloned()
            } else if at_root(&f) {
                Some(f.clone())
            } else {
                found.iter().find(|x| x.rsplit('/').next() == Some(f.as_str())).cloned()
            };
            if let Some(h) = hit {
                why = Some(format!("в проекте есть {h}"));
                break;
            }
        }
        if why.is_none() {
            if let Some(dep) = strs(&d["deps"]).into_iter().find(|x| p.deps.contains(&x.to_lowercase())) {
                why = Some(format!("проект использует {dep}"));
            }
        }
        if why.is_none() {
            if let Some(b) = strs(&d["bins"]).into_iter().find(|x| bins.contains(&x.to_lowercase())) {
                why = Some(format!("скрипты вызывают {b}"));
            }
        }
        if why.is_none() && topics_used.contains(&id) {
            why = Some("встречается в скриптах".into());
        }
        if why.is_none() {
            if let Some(s) = strs(&d["stacks"]).into_iter().find(|x| p.stacks.contains(x)) {
                why = Some(format!("технология {s}"));
            }
        }
        if why.is_none() && d["always"].as_bool() == Some(true) {
            why = Some("нужно любому проекту".into());
        }
        if let Some(w) = why {
            out.push(TopicRef {
                id,
                title: t["title"].as_str().unwrap_or("").into(),
                color: t["color"].as_str().unwrap_or("#888888").into(),
                icon: t["icon"].as_str().unwrap_or("book").into(),
                summary: t["summary"].as_str().unwrap_or("").into(),
                why: w,
            });
        }
    }
    // kb.topics уже стоят в порядке knowledge/lessons.json
    out
}
