//! Статус CI через glab / gh. Только по запросу (без опроса в фоне) и только к доверенным хостам:
//! адрес из чужого .git/config не должен получать ваш токен.
use crate::model::Remote;
use crate::{git, paths, runner};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: u64,
    pub status: String,
    pub r#ref: String,
    pub sha: String,
    pub url: String,
    pub created_at: String,
    /// начало и последнее обновление: по ним окно оценивает, сколько ещё идти пайплайну
    pub started_at: String,
    pub updated_at: String,
    pub source: String,
    pub name: String,
    pub title: String,
}

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: u64,
    pub name: String,
    pub stage: String,
    pub status: String,
    pub url: String,
    pub duration: Option<u64>,
    /// когда джоба началась (для оценки «сколько ещё»)
    pub started_at: Option<String>,
    pub allow_failure: bool,
    pub failure_reason: Option<String>,
    pub runner: Option<String>,
    pub step: Option<String>,
    pub failed_step: Option<String>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct StageJobs {
    pub name: String,
    pub jobs: Vec<Job>,
}

fn norm(s: &str) -> String {
    match s.to_lowercase().as_str() {
        "success" | "passed" => "success",
        "failed" | "failure" | "timed_out" | "startup_failure" => "failed",
        "canceled" | "cancelled" => "canceled",
        "skipped" | "neutral" | "stale" => "skipped",
        "manual" | "action_required" => "manual",
        "running" | "in_progress" => "running",
        _ => "pending",
    }
    .into()
}

/// Куда и как спрашивать: (github|gitlab, хост, путь репозитория).
/// Хост - из списка доверенных, путь - проверенный `git::valid_slug`: «evil.example/o/r» с хостом
/// github.com иначе ушёл бы в gh как ХОСТ/владелец/репозиторий, то есть токен - на чужой сервер.
fn target(r: &Remote) -> Result<(&'static str, String, String), String> {
    let host = r.host.clone().unwrap_or_default();
    if !r.trusted {
        return Err(format!("Хост «{host}» не в списке доверенных. Добавьте его в настройках, если это ваш сервер CI"));
    }
    let slug = r.slug.clone().unwrap_or_default();
    if !git::valid_host(&host) || !git::valid_slug(&host, &slug) {
        return Err(format!("Адрес репозитория «{}» не похож на «владелец/репозиторий» - CI не спрашиваю", r.url));
    }
    Ok((if git::is_github(&host) { "github" } else { "gitlab" }, host, slug))
}

/// glab и gh запускаются из папки данных Пульта, а не из проекта: иначе они читали бы
/// чужой .git/config (и звали бы git с ним), а хост выбирали бы по его remote.
fn call(bin: &str, args: Vec<String>, host: &str, timeout: u64) -> Result<String, String> {
    let mut argv = vec![bin.to_string()];
    argv.extend(args);
    let envs: [(&str, &str); 2] =
        if bin == "gh" { [("GH_HOST", host), ("GH_PROMPT_DISABLED", "1")] } else { [("GITLAB_HOST", host), ("GL_HOST", host)] };
    let c = runner::capture_env(&argv, &paths::data_dir(), Duration::from_secs(timeout), &envs).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("Не найдена программа «{bin}». Установите её и войдите: {bin} auth login")
        } else {
            e.to_string()
        }
    })?;
    if c.timed_out {
        return Err(format!("{bin} не ответил за {timeout} с - сеть или сервер медленные"));
    }
    if c.code != 0 {
        let msg = format!("{}\n{}", c.err, c.out);
        let low = msg.to_lowercase();
        return Err(if low.contains("401") || low.contains("unauthorized") {
            format!("{bin}: сервер не принял токен (401). Войдите заново: {bin} auth login")
        } else if low.contains("not logged") || low.contains("auth login") || low.contains("no token") {
            format!("{bin} не авторизован. Выполните: {bin} auth login")
        } else if low.contains("404") || low.contains("not found") {
            format!("{bin}: не найдено (404). Возможно, нет доступа к репозиторию или CI выключен")
        } else if low.contains("rate limit") || low.contains("429") {
            format!("{bin}: превышен лимит запросов. Подождите пару минут")
        } else if low.contains("could not resolve") || low.contains("connection") || low.contains("tls") {
            format!("{bin}: нет связи с сервером. Проверьте сеть или прокси")
        } else {
            msg.trim().lines().take(4).collect::<Vec<_>>().join("\n")
        });
    }
    Ok(c.out)
}

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

fn s(v: &Value) -> String {
    v.as_str().map(str::to_string).unwrap_or_default()
}

pub fn runs(_dir: &Path, r: &Remote, git_ref: Option<&str>) -> Result<Vec<Run>, String> {
    let (kind, host, slug) = target(r)?;
    let repo = format!("{host}/{slug}");
    let json: Value = match kind {
        "gitlab" => {
            let mut path = format!("projects/{}/pipelines?per_page=20", enc(&slug));
            if let Some(rf) = git_ref {
                path.push_str(&format!("&ref={}", enc(rf)));
            }
            serde_json::from_str(&call("glab", vec!["api".into(), path, "--hostname".into(), host.clone()], &host, 25)?)
                .map_err(|e| e.to_string())?
        }
        _ => {
            let mut a: Vec<String> = [
                "run",
                "list",
                "--repo",
                &repo,
                "--limit",
                "20",
                "--json",
                "databaseId,status,conclusion,workflowName,headBranch,headSha,createdAt,startedAt,updatedAt,url,event,displayTitle",
            ]
            .iter()
            .map(|x| x.to_string())
            .collect();
            if let Some(rf) = git_ref {
                a.extend(["--branch".into(), rf.to_string()]);
            }
            serde_json::from_str(&call("gh", a, &host, 25)?).map_err(|e| e.to_string())?
        }
    };
    let gh = kind == "github";
    Ok(json
        .as_array()
        .map(|a| {
            a.iter()
                .map(|p| {
                    if gh {
                        let st = if s(&p["status"]) == "completed" { norm(&s(&p["conclusion"])) } else { norm(&s(&p["status"])) };
                        Run {
                            id: p["databaseId"].as_u64().unwrap_or(0),
                            status: st,
                            r#ref: s(&p["headBranch"]),
                            sha: s(&p["headSha"]).chars().take(8).collect(),
                            url: s(&p["url"]),
                            created_at: s(&p["createdAt"]),
                            started_at: s(&p["startedAt"]),
                            updated_at: s(&p["updatedAt"]),
                            source: s(&p["event"]),
                            name: s(&p["workflowName"]),
                            title: s(&p["displayTitle"]),
                        }
                    } else {
                        Run {
                            id: p["id"].as_u64().unwrap_or(0),
                            status: norm(&s(&p["status"])),
                            r#ref: s(&p["ref"]),
                            sha: s(&p["sha"]).chars().take(8).collect(),
                            url: s(&p["web_url"]),
                            created_at: s(&p["created_at"]),
                            started_at: s(&p["started_at"]),
                            updated_at: s(&p["updated_at"]),
                            source: s(&p["source"]),
                            ..Default::default()
                        }
                    }
                })
                .collect()
        })
        .unwrap_or_default())
}

pub fn jobs(_dir: &Path, r: &Remote, id: u64) -> Result<Vec<StageJobs>, String> {
    let (kind, host, slug) = target(r)?;
    if kind == "gitlab" {
        let v: Value = serde_json::from_str(&call(
            "glab",
            vec![
                "api".into(),
                format!("projects/{}/pipelines/{id}/jobs?per_page=100&include_retried=false", enc(&slug)),
                "--hostname".into(),
                host.clone(),
            ],
            &host,
            25,
        )?)
        .map_err(|e| e.to_string())?;
        let mut jobs: Vec<Job> = v
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|j| Job {
                        id: j["id"].as_u64().unwrap_or(0),
                        name: s(&j["name"]),
                        stage: s(&j["stage"]),
                        status: norm(&s(&j["status"])),
                        url: s(&j["web_url"]),
                        duration: j["duration"].as_f64().map(|d| d.round() as u64),
                        started_at: j["started_at"].as_str().map(str::to_string),
                        allow_failure: j["allow_failure"].as_bool().unwrap_or(false),
                        failure_reason: j["failure_reason"].as_str().map(str::to_string),
                        runner: j["runner"]["description"].as_str().map(str::to_string),
                        ..Default::default()
                    })
                    .collect()
            })
            .unwrap_or_default();
        jobs.sort_by_key(|j| j.id);
        let mut order: Vec<String> = Vec::new();
        for j in &jobs {
            if !order.contains(&j.stage) {
                order.push(j.stage.clone());
            }
        }
        return Ok(order
            .into_iter()
            .map(|st| StageJobs { jobs: jobs.iter().filter(|j| j.stage == st).cloned().collect(), name: st })
            .collect());
    }
    let v: Value = serde_json::from_str(&call(
        "gh",
        vec![
            "run".into(),
            "view".into(),
            id.to_string(),
            "--repo".into(),
            format!("{host}/{slug}"),
            "--json".into(),
            "jobs,workflowName".into(),
        ],
        &host,
        25,
    )?)
    .map_err(|e| e.to_string())?;
    let secs = |a: &Value, b: &Value| -> Option<u64> {
        let p = |x: &Value| {
            x.as_str().and_then(|s| s.get(11..19)).and_then(|t| {
                let v: Vec<u64> = t.split(':').filter_map(|n| n.parse().ok()).collect();
                (v.len() == 3).then(|| v[0] * 3600 + v[1] * 60 + v[2])
            })
        };
        Some(p(b)?.saturating_sub(p(a)?))
    };
    let name = s(&v["workflowName"]);
    let jobs = v["jobs"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| Job {
                    id: x["databaseId"].as_u64().unwrap_or(0),
                    name: s(&x["name"]),
                    stage: name.clone(),
                    status: if s(&x["status"]) == "completed" { norm(&s(&x["conclusion"])) } else { norm(&s(&x["status"])) },
                    url: s(&x["url"]),
                    duration: secs(&x["startedAt"], &x["completedAt"]),
                    started_at: x["startedAt"].as_str().map(str::to_string),
                    step: x["steps"].as_array().and_then(|st| st.iter().find(|q| q["status"] == "in_progress")).map(|q| s(&q["name"])),
                    failed_step: x["steps"]
                        .as_array()
                        .and_then(|st| st.iter().find(|q| q["conclusion"] == "failure"))
                        .map(|q| s(&q["name"])),
                    ..Default::default()
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(vec![StageJobs { name, jobs }])
}

pub fn job_log(_dir: &Path, r: &Remote, job: u64) -> Result<String, String> {
    let (kind, host, slug) = target(r)?;
    let mut text = if kind == "gitlab" {
        let t = call(
            "glab",
            vec!["api".into(), format!("projects/{}/jobs/{job}/trace", enc(&slug)), "--hostname".into(), host.clone()],
            &host,
            40,
        )?;
        let t = crate::re!(r"section_(?:start|end):\d+:[\w.\-]+(?:\[[^\]]*\])?\r?(?:\x1b\[0K)?").replace_all(&t, "").into_owned();
        // метки времени новых раннеров: «2026-…Z 00O+текст»; «+» - продолжение строки
        let ts = crate::re!(r"^\d{4}-\d\d-\d\dT[\d:.]+Z [0-9a-f]{2}[OE](\+?)(.*)$");
        if t.lines().take(5).any(|l| ts.is_match(l)) {
            let mut out: Vec<String> = Vec::new();
            for l in t.split('\n') {
                match ts.captures(l) {
                    Some(c) if !c[1].is_empty() && !out.is_empty() => {
                        let last = out.len() - 1;
                        out[last].push_str(&c[2]);
                    }
                    Some(c) => out.push(c[2].strip_prefix(' ').unwrap_or(&c[2]).to_string()),
                    None => out.push(l.to_string()),
                }
            }
            out.join("\n")
        } else {
            t
        }
    } else {
        let t = call(
            "gh",
            vec!["run".into(), "view".into(), "--job".into(), job.to_string(), "--log".into(), "--repo".into(), format!("{host}/{slug}")],
            &host,
            40,
        )?;
        t.lines()
            .map(|l| {
                let p: Vec<&str> = l.splitn(3, '\t').collect();
                if p.len() == 3 {
                    p[2].split_once(' ').map(|x| x.1).unwrap_or(p[2]).to_string()
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    const MAX: usize = 2_000_000;
    if text.len() > MAX {
        let cut = text.len() - MAX;
        let cut = (cut..text.len()).find(|i| text.is_char_boundary(*i)).unwrap_or(cut);
        text = format!("… начало журнала обрезано …\n{}", &text[cut..]);
    }
    Ok(text)
}
