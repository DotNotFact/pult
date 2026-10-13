//! git: состояние репозитория. Все вызовы с выключенным fsmonitor - иначе чужой
//! .git/config мог бы запустить свою программу при простом открытии папки.
use crate::model::{Commit, GitFile, GitInfo, Remote};
use crate::runner;
use crate::util::{mtime_ms, rel};
use std::path::{Path, PathBuf};
use std::time::Duration;

const SAFE: [&str; 8] = ["-c", "core.fsmonitor=false", "-c", "core.quotepath=false", "-c", "core.untrackedCache=false", "-c", "color.ui=false"];

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let mut argv: Vec<String> = vec!["git".into()];
    argv.extend(SAFE.iter().map(|s| s.to_string()));
    argv.extend(["-C".to_string(), dir.to_string_lossy().into_owned()]);
    argv.extend(args.iter().map(|s| s.to_string()));
    let c = runner::capture_env(&argv, dir, Duration::from_secs(10), &[("GIT_OPTIONAL_LOCKS", "0"), ("LC_ALL", "C")]).ok()?;
    (c.code == 0).then_some(c.out)
}

pub fn parse_remote(url: &str) -> Option<(String, String)> {
    let re1 = crate::re!(r"^(?:https?|ssh|git)://(?:[^@/]+@)?([^/:]+)(?::\d+)?/(.+?)(?:\.git)?/?$");
    if let Some(m) = re1.captures(url) {
        return Some((m[1].to_string(), m[2].to_string()));
    }
    let re2 = crate::re!(r"^(?:[^@]+@)?([^:/]+):(.+?)(?:\.git)?$");
    re2.captures(url).map(|m| (m[1].to_string(), m[2].to_string()))
}

const LABEL: [(&str, &str); 8] = [("M", "изменён"), ("A", "добавлен"), ("D", "удалён"), ("R", "переименован"), ("C", "скопирован"), ("U", "конфликт"), ("?", "новый"), ("!", "игнор")];

pub fn info(dir: &Path, trusted_hosts: &[String]) -> Option<GitInfo> {
    let top = git(dir, &["rev-parse", "--show-toplevel", "--absolute-git-dir", "--git-common-dir"])?;
    let mut lines = top.lines();
    let toplevel = PathBuf::from(lines.next()?.trim());
    let git_dir = PathBuf::from(lines.next()?.trim());
    let common_raw = lines.next().unwrap_or("").trim().to_string();
    let common = if common_raw.is_empty() { git_dir.clone() } else if Path::new(&common_raw).is_absolute() { PathBuf::from(&common_raw) } else { dir.join(&common_raw) };
    let prefix = {
        let d = std::fs::canonicalize(dir).unwrap_or(dir.to_path_buf());
        let t = std::fs::canonicalize(&toplevel).unwrap_or(toplevel.clone());
        let r = rel(&t, &d);
        if r == "." { String::new() } else { r }
    };

    let (st, log, rem, stash, tag) = std::thread::scope(|s| {
        let a = s.spawn(|| git(dir, &["status", "--porcelain=v1", "-b", "-z", "--untracked-files=normal"]));
        let b = s.spawn(|| git(dir, &["log", "-n", "20", "--format=%h%x1f%s%x1f%an%x1f%cI%x1f%D"]));
        let c = s.spawn(|| git(dir, &["remote", "-v"]));
        let d = s.spawn(|| git(dir, &["stash", "list", "--format=%gd"]));
        let e = s.spawn(|| git(dir, &["describe", "--tags", "--abbrev=0"]));
        (a.join().ok().flatten(), b.join().ok().flatten(), c.join().ok().flatten(), d.join().ok().flatten(), e.join().ok().flatten())
    });

    let st = st.unwrap_or_default();
    let mut recs = st.split('\0').filter(|s| !s.is_empty());
    let head = recs.next().unwrap_or("");
    let (mut branch, mut upstream, mut ab) = ("?".to_string(), None, String::new());
    if head.starts_with("## HEAD (no branch)") {
        branch = "HEAD (отсоединён)".into();
    } else if let Some(h) = head.strip_prefix("## ") {
        let h = h.trim_start_matches("No commits yet on ").trim_start_matches("Initial commit on ");
        let (name_part, rest) = match h.find(" [") {
            Some(i) => (&h[..i], h[i + 2..].trim_end_matches(']').to_string()),
            None => (h, String::new()),
        };
        ab = rest;
        match name_part.split_once("...") {
            Some((b, u)) => {
                branch = b.into();
                upstream = Some(u.into());
            }
            None => branch = name_part.into(),
        }
    }
    let num = |key: &str| -> u32 { ab.split(", ").find_map(|p| p.strip_prefix(key).and_then(|n| n.trim().parse().ok())).unwrap_or(0) };
    let mut files = Vec::new();
    let mut dirty = 0;
    let all: Vec<&str> = recs.collect();
    let mut i = 0;
    while i < all.len() {
        let r = all[i];
        i += 1;
        if r.len() < 4 {
            continue;
        }
        dirty += 1;
        let x = &r[0..1];
        let y = &r[1..2];
        let mut p = r[3..].to_string();
        if x == "R" || x == "C" {
            // при переименовании следующая запись - старое имя
            i += 1;
        }
        // путь от корня репозитория -> путь от папки проекта
        if !prefix.is_empty() {
            match p.strip_prefix(&format!("{prefix}/")) {
                Some(rest) => p = rest.to_string(),
                None => continue,
            }
        }
        let code = if x == "?" { "?" } else if x == "U" || y == "U" || (x == "A" && y == "A") || (x == "D" && y == "D") { "U" } else if x != " " { x } else { y };
        if files.len() < 400 {
            files.push(GitFile { path: p, code: code.into(), label: LABEL.iter().find(|(k, _)| *k == code).map(|(_, v)| v.to_string()).unwrap_or_else(|| code.to_string()), staged: x != " " && x != "?" });
        }
    }

    let mut remotes: Vec<Remote> = Vec::new();
    for l in rem.unwrap_or_default().lines() {
        let mut it = l.split_whitespace();
        let (Some(name), Some(url), Some(kind)) = (it.next(), it.next(), it.next()) else { continue };
        if kind != "(fetch)" {
            continue;
        }
        let parsed = parse_remote(url);
        let key = parsed.as_ref().map(|(h, s)| format!("{h}/{s}")).unwrap_or_else(|| url.to_string());
        let dup = remotes.iter().any(|r| r.host.as_ref().zip(r.slug.as_ref()).map(|(h, s)| format!("{h}/{s}")).unwrap_or_else(|| r.url.clone()) == key);
        let host = parsed.as_ref().map(|p| p.0.clone());
        let trusted = host.as_ref().is_some_and(|h| trusted_hosts.iter().any(|t| t.eq_ignore_ascii_case(h)));
        remotes.push(Remote { name: name.into(), url: url.into(), host, slug: parsed.map(|p| p.1), dup, trusted });
    }

    let commits = log
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.split('\u{1f}').collect();
            (p.len() >= 4).then(|| Commit {
                hash: p[0].into(),
                subject: p[1].into(),
                author: p[2].into(),
                date: p[3].into(),
                refs: p.get(4).map(|r| r.split(", ").filter(|x| !x.is_empty() && !x.starts_with("HEAD")).take(4).map(str::to_string).collect()).unwrap_or_default(),
            })
        })
        .collect();

    let fetched = mtime_ms(&common.join("FETCH_HEAD"));
    let op = if git_dir.join("MERGE_HEAD").exists() {
        Some("merge".into())
    } else if git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists() {
        Some("rebase".into())
    } else {
        None
    };

    Some(GitInfo {
        toplevel: toplevel.to_string_lossy().into_owned(),
        prefix,
        branch,
        upstream,
        ahead: num("ahead "),
        behind: num("behind "),
        gone: ab.contains("gone"),
        dirty,
        files,
        stash: stash.unwrap_or_default().lines().filter(|l| !l.is_empty()).count() as u32,
        tag: tag.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()),
        fetched_at: (fetched > 0).then_some(fetched),
        op,
        remotes,
        commits,
    })
}
