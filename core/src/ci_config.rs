//! Разбор файлов CI без YAML-библиотеки: нужны только этапы, джобы и их связи.
use crate::kb::Kb;
use crate::model::{CiConfig, CiJobCfg, CiStageCfg, GhJobCfg};
use crate::util::read_text;
use std::collections::HashMap;
use std::path::Path;

const RESERVED: [&str; 10] =
    ["stages", "variables", "default", "include", "workflow", "image", "services", "before_script", "after_script", "cache"];

/// Ключ верхнего уровня: «test:api:», «build job:», «"x y":», «image: node:20».
fn top_key(line: &str) -> Option<(String, String)> {
    if line.is_empty() || line.starts_with([' ', '\t', '#', '-']) {
        return None;
    }
    let unq = |s: &str| s.trim().trim_matches('"').trim_matches('\'').to_string();
    let no_comment = match line.find(" #") {
        Some(i) => &line[..i],
        None => line,
    };
    let t = no_comment.trim_end();
    if let Some(k) = t.strip_suffix(':') {
        return Some((unq(k), String::new()));
    }
    // «ключ: значение» - ключ до первого «: »
    let i = t.find(": ")?;
    Some((unq(&t[..i]), t[i + 2..].trim().to_string()))
}

struct Block {
    name: String,
    rest: String,
    comment: String,
    lines: Vec<String>,
    line: u32,
}

pub fn parse_gitlab(text: &str, kb: &Kb) -> CiConfig {
    let mut stages: Vec<String> = Vec::new();
    let mut blocks: Vec<Block> = Vec::new();
    let mut comment: Vec<String> = Vec::new();
    for (i, l) in text.lines().enumerate() {
        if let Some((k, rest)) = top_key(l) {
            blocks.push(Block { name: k, rest, comment: comment.join(" ").trim().to_string(), lines: vec![], line: i as u32 + 1 });
            comment.clear();
        } else if let Some(c) = l.strip_prefix('#') {
            let c = c.trim();
            if !c.chars().all(|ch| "-=# ".contains(ch)) {
                comment.push(c.to_string());
                if comment.len() > 4 {
                    comment.remove(0);
                }
            }
        } else if let Some(b) = blocks.last_mut() {
            b.lines.push(l.to_string());
            if l.trim().is_empty() {
                comment.clear();
            }
        }
    }
    struct J {
        stage: Option<String>,
        ext: Vec<String>,
        manual: bool,
        allow: bool,
        needs: Vec<String>,
        desc: String,
        line: u32,
        hidden: bool,
        only_main: bool,
        only_mr: bool,
    }
    let re_stage = crate::re!(r#"stage:\s*["']?([\w.\- ]+?)["']?\s*$"#);
    let mut info: Vec<(String, J)> = Vec::new();
    // якоря YAML: «.base: &base» - слияние «<<: *base» в джобе работает как extends
    let anchors: HashMap<String, String> = blocks
        .iter()
        .filter_map(|b| b.rest.strip_prefix('&').map(|a| (a.split_whitespace().next().unwrap_or("").to_string(), b.name.clone())))
        .collect();
    for b in &blocks {
        if b.name == "stages" {
            stages = if b.rest.starts_with('[') {
                b.rest
                    .trim_matches(['[', ']'])
                    .split(',')
                    .map(|s| s.trim().trim_matches(['"', '\'']).to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            } else {
                b.lines
                    .iter()
                    .filter_map(|l| strip_comment(l).trim().strip_prefix("- ").map(|s| s.trim().trim_matches(['"', '\'']).to_string()))
                    .collect()
            };
            continue;
        }
        if RESERVED.contains(&b.name.as_str()) {
            continue;
        }
        let body: Vec<&String> = b.lines.iter().filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#')).collect();
        if body.is_empty() {
            continue;
        }
        let ind = |l: &str| l.len() - l.trim_start().len();
        let base = body.iter().map(|l| ind(l)).min().unwrap_or(0);
        let direct: Vec<&&String> = body.iter().filter(|l| ind(l) == base).collect();
        let key = |k: &str| direct.iter().find(|l| l.trim_start().starts_with(&format!("{k}:"))).map(|s| s.to_string());
        if !["script", "trigger", "extends", "stage", "needs", "run", "<<"].iter().any(|k| key(k).is_some()) {
            continue;
        }
        let stage = key("stage").and_then(|l| re_stage.captures(strip_comment(&l).trim()).map(|c| c[1].trim().to_string()));
        let mut ext = Vec::new();
        if let Some(e) = key("extends") {
            let v = strip_comment(&e).trim_start().trim_start_matches("extends:").trim();
            if !v.is_empty() {
                ext = v
                    .trim_matches(['[', ']'])
                    .split(',')
                    .map(|s| s.trim().trim_matches(['"', '\'']).to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            } else if let Some(pos) = body.iter().position(|l| l.trim_start().starts_with("extends:")) {
                for l in &body[pos + 1..] {
                    match l.trim().strip_prefix("- ") {
                        Some(x) => ext.push(strip_comment(x).trim().trim_matches(['"', '\'']).to_string()),
                        None => break,
                    }
                }
            }
        }
        if let Some(m) = key("<<") {
            for a in crate::re!(r"\*([\w.\-]+)").captures_iter(&m) {
                if let Some(n) = anchors.get(&a[1]) {
                    ext.push(n.clone());
                }
            }
        }
        let mut needs = Vec::new();
        if let Some(n) = key("needs") {
            let v = n.trim_start().trim_start_matches("needs:").trim();
            if v.starts_with('[') {
                needs = v
                    .trim_matches(['[', ']'])
                    .split(',')
                    .map(|s| s.trim().trim_matches(['"', '\'']).to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
            } else if v.is_empty() {
                // списком: «- lint» или «- job: lint» (под элементом бывают свои ключи - их пропускаем)
                if let Some(pos) = body.iter().position(|l| ind(l) == base && l.trim_start().starts_with("needs:")) {
                    for l in body[pos + 1..].iter().take_while(|l| ind(l) > base) {
                        if let Some(x) = l.trim().strip_prefix("- ") {
                            let x = x.trim();
                            let x = x.strip_prefix("job:").unwrap_or(x).trim().trim_matches(['"', '\'']);
                            // «- project: ...», «- pipeline: ...» - чужой проект, не джоба этого файла
                            if !x.is_empty() && !x.contains(": ") {
                                needs.push(x.to_string());
                            }
                        }
                    }
                }
            }
        }
        let all = body.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n");
        let only_main = all.contains("$CI_COMMIT_BRANCH == $CI_DEFAULT_BRANCH")
            || crate::re!(r#"\$CI_COMMIT_BRANCH\s*==\s*["']main["']"#).is_match(&all);
        info.push((
            b.name.clone(),
            J {
                stage,
                ext,
                manual: all.contains("when: manual"),
                allow: all.contains("allow_failure: true"),
                needs,
                desc: b.comment.clone(),
                line: b.line,
                hidden: b.name.starts_with('.'),
                only_main,
                only_mr: all.contains("merge_request_event") && !only_main,
            },
        ));
    }
    let idx: HashMap<String, usize> = info.iter().enumerate().map(|(i, (n, _))| (n.clone(), i)).collect();
    fn resolve(i: usize, info: &[(String, J)], idx: &HashMap<String, usize>, depth: u8) -> Option<String> {
        let j = &info[i].1;
        if j.stage.is_some() {
            return j.stage.clone();
        }
        if depth > 5 {
            return None;
        }
        j.ext.iter().find_map(|e| idx.get(e).and_then(|&k| resolve(k, info, idx, depth + 1)))
    }
    let jobs: Vec<CiJobCfg> = info
        .iter()
        .enumerate()
        .filter(|(_, (_, j))| !j.hidden)
        .map(|(i, (n, j))| CiJobCfg {
            name: n.clone(),
            stage: resolve(i, &info, &idx, 0).unwrap_or_else(|| "test".into()),
            manual: j.manual,
            allow_failure: j.allow,
            needs: j.needs.clone(),
            desc: j.desc.clone(),
            line: j.line,
            only_main: j.only_main,
            only_mr: j.only_mr,
        })
        .collect();
    let order = if stages.is_empty() {
        vec![".pre".to_string(), "build".into(), "test".into(), "deploy".into(), ".post".into()]
    } else {
        stages.clone()
    };
    let mut names = order.clone();
    for j in &jobs {
        if !names.contains(&j.stage) {
            names.push(j.stage.clone());
        }
    }
    let job_count = jobs.len() as u32;
    let stages = names
        .into_iter()
        .map(|s| CiStageCfg { sid: kb.classify(&s, ""), jobs: jobs.iter().filter(|j| j.stage == s).cloned().collect(), name: s })
        .filter(|s| !s.jobs.is_empty() || (stages.contains(&s.name)))
        .collect();
    let includes = crate::re!(r#"(?m)^\s*-?\s*local:\s*["']?([^"'\s]+)"#).captures_iter(text).map(|c| c[1].to_string()).collect();
    CiConfig { kind: "gitlab".into(), file: ".gitlab-ci.yml".into(), stages, includes, job_count, ..Default::default() }
}

/// Строка YAML без комментария в конце («stage: build # сборка» -> «stage: build»).
fn strip_comment(l: &str) -> &str {
    match l.find(" #") {
        Some(i) => l[..i].trim_end(),
        None => l.trim_end(),
    }
}

/// Отступ первой значимой строки после `from` (глубже `parent`); None - блок пуст.
fn child_indent(lines: &[&str], from: usize, parent: usize) -> Option<usize> {
    for l in lines.iter().skip(from) {
        let t = l.trim_start();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let ind = l.len() - t.len();
        return (ind > parent).then_some(ind);
    }
    None
}

pub fn parse_github(text: &str, file: &str, kb: &Kb) -> CiConfig {
    let lines: Vec<&str> = text.lines().collect();
    let unq = |s: &str| s.trim().trim_matches(['"', '\'']).to_string();
    let name = lines
        .iter()
        .find_map(|l| strip_comment(l).strip_prefix("name:"))
        .map(unq)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| file.rsplit('/').next().unwrap_or(file).to_string());
    let mut triggers = Vec::new();
    let top = |l: &&str, k: &str| {
        let t = strip_comment(l);
        t == format!("{k}:") || t.starts_with(&format!("{k}: ")) || t.starts_with(&format!("\"{k}\":")) || t.starts_with(&format!("'{k}':"))
    };
    if let Some(oi) = lines.iter().position(|l| top(l, "on")) {
        let inline = strip_comment(lines[oi]).split_once(':').map(|x| x.1.trim()).unwrap_or("");
        if !inline.is_empty() {
            triggers.extend(inline.trim_matches(['[', ']']).split(',').map(unq).filter(|s| !s.is_empty()));
        } else if let Some(ci) = child_indent(&lines, oi + 1, 0) {
            for l in &lines[oi + 1..] {
                if !l.starts_with([' ', '\t']) && !l.trim().is_empty() {
                    break;
                }
                let t = strip_comment(l).trim_start();
                if l.len() - l.trim_start().len() != ci || t.is_empty() || t.starts_with('#') {
                    continue;
                }
                if let Some(k) = t.strip_prefix("- ") {
                    triggers.push(unq(k));
                } else if let Some((k, _)) = t.split_once(':') {
                    triggers.push(unq(k));
                }
            }
        }
    }
    let mut jobs: Vec<GhJobCfg> = Vec::new();
    // отступы бывают любые (2 и 4 пробела): уровень джоб и их ключей берётся из самого файла
    if let Some(ji) = lines.iter().position(|l| strip_comment(l) == "jobs:") {
        let job_ind = child_indent(&lines, ji + 1, 0).unwrap_or(2);
        let mut key_ind: Option<usize> = None;
        // «needs:» без значения - дальше идёт список «- build»
        let mut needs_list = false;
        for (k, l) in lines.iter().enumerate().skip(ji + 1) {
            if !l.starts_with([' ', '\t']) && !l.trim().is_empty() {
                break;
            }
            let t = strip_comment(l).trim_start();
            let ind = l.len() - l.trim_start().len();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            if ind == job_ind && t.ends_with(':') {
                needs_list = false;
                let id = unq(t.trim_end_matches(':'));
                jobs.push(GhJobCfg { name: id.clone(), id, needs: vec![], sid: String::new(), line: k as u32 + 1 });
                key_ind = child_indent(&lines, k + 1, job_ind);
                continue;
            }
            if key_ind.is_some_and(|ki| ind <= ki) {
                needs_list = false;
            }
            if Some(ind) == key_ind {
                if let Some(j) = jobs.last_mut() {
                    if let Some(n) = t.strip_prefix("name:") {
                        j.name = unq(n);
                    } else if let Some(n) = t.strip_prefix("needs:") {
                        j.needs = n.trim().trim_matches(['[', ']']).split(',').map(unq).filter(|s| !s.is_empty()).collect();
                        needs_list = n.trim().is_empty();
                    }
                }
            } else if needs_list {
                if let (Some(j), Some(x)) = (jobs.last_mut(), t.strip_prefix("- ")) {
                    j.needs.push(unq(x));
                }
            }
        }
    }
    for j in &mut jobs {
        j.sid = kb.classify(&format!("{} {}", j.id, j.name), "");
    }
    let job_count = jobs.len() as u32;
    CiConfig { kind: "github".into(), file: file.into(), name, triggers, jobs, job_count, ..Default::default() }
}

pub fn detect(root: &Path, kb: &Kb) -> Vec<CiConfig> {
    let mut out = Vec::new();
    if let Some(t) = read_text(&root.join(".gitlab-ci.yml")) {
        out.push(parse_gitlab(&t, kb));
    }
    let wf = root.join(".github").join("workflows");
    let mut files: Vec<_> =
        crate::util::ls(&wf).into_iter().filter(|e| e.path().extension().is_some_and(|x| x == "yml" || x == "yaml")).collect();
    files.sort_by_key(|e| e.file_name());
    for e in files {
        if let Some(t) = read_text(&e.path()) {
            out.push(parse_github(&t, &format!(".github/workflows/{}", e.file_name().to_string_lossy()), kb));
        }
    }
    if let Some(t) = read_text(&root.join("Jenkinsfile")) {
        let re = crate::re!(r#"stage\s*\(\s*['"]([^'"]+)['"]"#);
        let st: Vec<CiStageCfg> =
            re.captures_iter(&t).map(|c| CiStageCfg { name: c[1].to_string(), sid: kb.classify(&c[1], ""), jobs: vec![] }).collect();
        out.push(CiConfig {
            kind: "jenkins".into(),
            file: "Jenkinsfile".into(),
            job_count: st.len() as u32,
            stages: st,
            ..Default::default()
        });
    }
    for f in [
        "azure-pipelines.yml",
        ".circleci/config.yml",
        "bitbucket-pipelines.yml",
        ".travis.yml",
        "appveyor.yml",
        ".drone.yml",
        ".woodpecker.yml",
    ] {
        if root.join(f).exists() {
            out.push(CiConfig { kind: "other".into(), file: f.into(), ..Default::default() });
        }
    }
    out
}
