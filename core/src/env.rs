//! Окружение: какие программы есть, жив ли Docker.
use crate::runner;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Docker {
    /// up | down | absent
    pub state: String,
    pub version: Option<String>,
    pub running: Option<u32>,
    pub error: Option<String>,
    pub ms: u64,
}

/// Для каждого имени программы - путь или None.
pub fn tools<'a>(names: impl IntoIterator<Item = &'a str>) -> BTreeMap<String, Option<String>> {
    names.into_iter().map(|n| (n.to_string(), runner::which(n).map(|p| p.to_string_lossy().into_owned()))).collect()
}

static DOCKER: Mutex<Option<(Instant, Docker)>> = Mutex::new(None);

pub fn docker(force: bool) -> Docker {
    if !force {
        if let Some((at, d)) = DOCKER.lock().ok().and_then(|g| g.clone()) {
            if at.elapsed() < Duration::from_secs(60) {
                return d;
            }
        }
    }
    let d = if runner::which("docker").is_none() {
        Docker { state: "absent".into(), ..Default::default() }
    } else {
        let t0 = Instant::now();
        let argv: Vec<String> = ["docker", "info", "--format", "{{.ServerVersion}}|{{.ContainersRunning}}"].iter().map(|s| s.to_string()).collect();
        match runner::capture(&argv, Path::new("."), Duration::from_secs(6)) {
            Ok(c) if c.code == 0 && !c.timed_out => {
                let mut it = c.out.trim().split('|');
                Docker { state: "up".into(), version: it.next().map(str::to_string), running: it.next().and_then(|x| x.parse().ok()), error: None, ms: t0.elapsed().as_millis() as u64 }
            }
            Ok(c) => Docker { state: "down".into(), error: Some(if c.timed_out { "не ответил за 6 с".into() } else { "демон не отвечает".into() }), ms: t0.elapsed().as_millis() as u64, ..Default::default() },
            Err(e) => Docker { state: "down".into(), error: Some(e.to_string()), ..Default::default() },
        }
    };
    if let Ok(mut g) = DOCKER.lock() {
        *g = Some((Instant::now(), d.clone()));
    }
    d
}
