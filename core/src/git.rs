//! git: состояние репозитория.
//!
//! Чужая папка приносит свой `.git/config`, а git исполняет из него программы: фильтры
//! `filter.*.clean` при `git status`, `gpg.program` при проверке подписей в `git log`,
//! `core.fsmonitor` при чтении индекса. Поэтому до первого вызова git Пульт сам находит папку
//! `.git`, читает её настройки и решает, можно ли звать git (`locate`):
//! - сетевой путь где угодно (gitdir, commondir, alternates, значение настройки) - отказ без
//!   права доверить: обращение к `\\host\share` отдаёт хеш NTLM чужому серверу;
//! - опасный ключ, ссылка или папка `.git` вне проекта - «не доверяю»: git не запускается,
//!   пока пользователь не нажмёт «Доверять этому репозиторию» (`store::trust_repo`);
//! - иначе git вызывается с набором `SAFE`.
use crate::model::{Commit, GitFile, GitInfo, Remote};
use crate::runner;
use crate::util::{inside, is_unc, mtime_ms, read_head, rel};
use std::path::{Path, PathBuf};
use std::time::Duration;

const SAFE: [&str; 17] = [
    "--no-pager",
    "-c",
    "core.fsmonitor=false",
    "-c",
    "core.quotepath=false",
    "-c",
    "core.untrackedCache=false",
    "-c",
    "color.ui=false",
    "-c",
    "log.showSignature=false",
    "-c",
    "protocol.allow=never",
    "-c",
    "safe.bareRepository=explicit",
    "-c",
    "submodule.recurse=false",
];

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let mut argv: Vec<String> = vec!["git".into()];
    argv.extend(SAFE.iter().map(|s| s.to_string()));
    argv.extend(["-C".to_string(), dir.to_string_lossy().into_owned()]);
    argv.extend(args.iter().map(|s| s.to_string()));
    let c = runner::capture_env(&argv, dir, Duration::from_secs(10), &[("GIT_OPTIONAL_LOCKS", "0"), ("LC_ALL", "C")]).ok()?;
    (c.code == 0).then_some(c.out)
}

// ---------- адрес remote ----------
pub(crate) fn is_github(host: &str) -> bool {
    host.to_ascii_lowercase().contains("github")
}

/// Путь репозитория из адреса remote годится для запроса к CI.
/// GitHub: ровно «владелец/репозиторий» - иначе `gh` прочтёт «evil.example/o/r» как ХОСТ/владелец/репозиторий.
/// GitLab: вложенные группы «a/b/c» можно, но каждая часть - только буквы, цифры, «_», «.», «-».
pub fn valid_slug(host: &str, slug: &str) -> bool {
    let seg = crate::re!(r"^[A-Za-z0-9_.\-]+$");
    let parts: Vec<&str> = slug.split('/').collect();
    let ok_parts = parts.iter().all(|p| seg.is_match(p) && *p != "." && *p != "..");
    if !ok_parts || parts.len() < 2 {
        return false;
    }
    !is_github(host) || parts.len() == 2
}

/// Хост: имя без пользователя, порта и пути.
pub(crate) fn valid_host(host: &str) -> bool {
    crate::re!(r"^[A-Za-z0-9](?:[A-Za-z0-9\-.]*[A-Za-z0-9])?$").is_match(host)
}

pub fn parse_remote(url: &str) -> Option<(String, String)> {
    let re1 = crate::re!(r"^(?:https?|ssh|git)://(?:[^@/]+@)?([^/:]+)(?::\d+)?/(.+?)(?:\.git)?/?$");
    let re2 = crate::re!(r"^(?:[^@]+@)?([^:/]+):(.+?)(?:\.git)?$");
    let (host, slug) = match re1.captures(url) {
        Some(m) => (m[1].to_string(), m[2].to_string()),
        None => re2.captures(url).map(|m| (m[1].to_string(), m[2].to_string()))?,
    };
    (valid_host(&host) && valid_slug(&host, &slug)).then_some((host, slug))
}

// ---------- поиск репозитория и проверка его настроек ----------
/// Почему git для этого репозитория не запускается.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    /// сетевой путь: не запускается даже после «Доверять»
    pub refused: bool,
    /// код причины: unc, alternates, key, link, outside, parse, mismatch
    pub why: String,
    /// ключ настройки, из-за которого не доверяем
    pub key: Option<String>,
}

impl Verdict {
    fn refused(why: &str, key: Option<String>) -> Verdict {
        Verdict { refused: true, why: why.into(), key }
    }
    fn untrusted(why: &str, key: Option<String>) -> Verdict {
        Verdict { refused: false, why: why.into(), key }
    }
}

/// Где лежит репозиторий и что в его настройках.
#[derive(Debug, Clone)]
pub struct Located {
    /// рабочая папка (там, где лежит .git)
    pub work: PathBuf,
    /// папка git этой рабочей копии
    pub git_dir: PathBuf,
    /// общая папка git (у рабочих копий worktree - папка основного репозитория)
    pub common: PathBuf,
    /// настройки репозитория: «секция.подсекция.ключ» и значение
    pub entries: Vec<(String, String)>,
    /// None - можно звать git
    pub verdict: Option<Verdict>,
}

/// Ссылка (symlink, junction): None - не ссылка, Some(true) - ведёт на сетевой путь.
fn link_to_unc(p: &Path) -> Option<bool> {
    let m = std::fs::symlink_metadata(p).ok()?;
    if !m.file_type().is_symlink() {
        return None;
    }
    Some(std::fs::read_link(p).map(|t| is_unc(&t.to_string_lossy())).unwrap_or(true))
}

/// Найти репозиторий так же, как его найдёт git: вверх от папки до первой `.git`.
/// Сам git при этом не запускается: ни одного обращения к сети и ни одной чужой программы.
pub fn locate(dir: &Path) -> Option<Located> {
    for a in dir.ancestors() {
        let dot = a.join(".git");
        let Ok(meta) = std::fs::symlink_metadata(&dot) else { continue };
        let blank = |git_dir: PathBuf, v: Verdict| Located {
            work: a.to_path_buf(),
            common: git_dir.clone(),
            git_dir,
            entries: vec![],
            verdict: Some(v),
        };
        if meta.file_type().is_symlink() {
            return Some(match link_to_unc(&dot) {
                Some(false) => blank(dot, Verdict::untrusted("link", None)),
                _ => blank(dot, Verdict::refused("unc", None)),
            });
        }
        if meta.is_dir() {
            // папка без HEAD - не репозиторий, git идёт выше; так же поступаем и мы
            if !dot.join("HEAD").is_file() {
                continue;
            }
            return Some(finish(a, dot));
        }
        // файл .git: «gitdir: <путь>» (рабочая копия worktree, подмодуль)
        let text = read_head(&dot, 4096)?;
        let target = text.lines().next()?.trim().strip_prefix("gitdir:")?.trim().to_string();
        if target.is_empty() {
            return None;
        }
        if is_unc(&target) {
            return Some(blank(PathBuf::from(&target), Verdict::refused("unc", Some("gitdir".into()))));
        }
        let git_dir = if Path::new(&target).is_absolute() { PathBuf::from(&target) } else { a.join(&target) };
        if !git_dir.join("HEAD").is_file() {
            return None;
        }
        return Some(finish(a, git_dir));
    }
    None
}

fn finish(work: &Path, git_dir: PathBuf) -> Located {
    let mut loc = Located { work: work.to_path_buf(), git_dir: git_dir.clone(), common: git_dir.clone(), entries: vec![], verdict: None };
    let mut untrusted: Option<Verdict> = None;
    let mut note = |v: Verdict| {
        if untrusted.is_none() {
            untrusted = Some(v);
        }
    };

    // commondir: у рабочей копии worktree настройки и объекты лежат в основном репозитории
    let commondir = git_dir.join("commondir");
    match link_to_unc(&commondir) {
        Some(true) => {
            loc.verdict = Some(Verdict::refused("unc", Some("commondir".into())));
            return loc;
        }
        Some(false) => note(Verdict::untrusted("link", Some("commondir".into()))),
        None => {}
    }
    if let Some(c) = read_head(&commondir, 4096) {
        let c = c.trim();
        if is_unc(c) {
            loc.verdict = Some(Verdict::refused("unc", Some("commondir".into())));
            return loc;
        }
        if !c.is_empty() {
            loc.common = if Path::new(c).is_absolute() { PathBuf::from(c) } else { git_dir.join(c) };
        }
    }

    let mut files: Vec<PathBuf> = Vec::new();
    for f in [loc.common.join("config"), loc.common.join("config.worktree"), git_dir.join("config"), git_dir.join("config.worktree")] {
        if !files.contains(&f) {
            files.push(f);
        }
    }
    for f in &files {
        match link_to_unc(f) {
            Some(true) => {
                loc.verdict = Some(Verdict::refused("unc", Some("config".into())));
                return loc;
            }
            Some(false) => note(Verdict::untrusted("link", Some("config".into()))),
            None => {}
        }
        let Ok(meta) = std::fs::metadata(f) else { continue };
        if meta.len() > 1_000_000 {
            note(Verdict::untrusted("parse", None));
            continue;
        }
        match read_head(f, 1_000_000).and_then(|t| parse_config(&t)) {
            Some(e) => loc.entries.extend(e),
            None => note(Verdict::untrusted("parse", None)),
        }
    }

    // alternates: git читает объекты из перечисленных папок, сетевая - это NTLM
    let alt = loc.common.join("objects").join("info").join("alternates");
    if link_to_unc(&alt) == Some(true) {
        loc.verdict = Some(Verdict::refused("alternates", None));
        return loc;
    }
    if let Some(t) = read_head(&alt, 64 * 1024) {
        if t.lines().map(str::trim).any(|l| !l.is_empty() && !l.starts_with('#') && is_unc(l.trim_matches('"'))) {
            loc.verdict = Some(Verdict::refused("alternates", None));
            return loc;
        }
    }

    if let Some((k, _)) = loc.entries.iter().find(|(_, v)| is_unc(v)) {
        loc.verdict = Some(Verdict::refused("unc", Some(k.clone())));
        return loc;
    }

    for (k, v) in &loc.entries {
        if k == "core.worktree" {
            // подмодуль указывает обратно на свою папку - это нормально; любая другая - нет
            let p = if Path::new(v).is_absolute() { PathBuf::from(v) } else { git_dir.join(v) };
            if !same_path(&p, work) {
                note(Verdict::untrusted("key", Some(k.clone())));
            }
        } else if risky(k) {
            note(Verdict::untrusted("key", Some(k.clone())));
        }
    }

    // папка git вне проекта - только у настоящей рабочей копии worktree или подмодуля
    if !inside(work, &git_dir) || !inside(work, &loc.common) {
        let back = read_head(&git_dir.join("gitdir"), 4096).is_some_and(|t| {
            let t = t.trim();
            !is_unc(t) && same_path(Path::new(t), &work.join(".git"))
        });
        let sub = loc.common == git_dir && loc.entries.iter().any(|(k, _)| k == "core.worktree");
        if !(back || sub) {
            note(Verdict::untrusted("outside", None));
        }
    }
    loc.verdict = untrusted;
    loc
}

fn same_path(a: &Path, b: &Path) -> bool {
    inside(a, b) && inside(b, a)
}

/// Ключи, через которые настройки репозитория запускают программы или читают чужие файлы.
fn risky(key: &str) -> bool {
    const PREFIX: [&str; 4] = ["filter.", "gpg.", "include.", "includeif."];
    const EXACT: [&str; 7] =
        ["core.hookspath", "core.excludesfile", "core.attributesfile", "core.sshcommand", "core.pager", "core.askpass", "diff.external"];
    PREFIX.iter().any(|p| key.starts_with(p))
        || EXACT.contains(&key)
        || (key.starts_with("diff.") && (key.ends_with(".textconv") || key.ends_with(".command")))
        || (key.starts_with("merge.") && key.ends_with(".driver"))
}

/// Разбор файла настроек git. Секция и ключ - в нижнем регистре, подсекция - как в файле.
/// None - файл не разобрался: такому репозиторию не доверяем.
pub fn parse_config(text: &str) -> Option<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut section = String::new();
    let mut lines = text.lines();
    while let Some(raw) = lines.next() {
        let mut l = raw.trim_start().to_string();
        if l.starts_with('[') {
            let (name, rest) = header(&l)?;
            section = name;
            l = rest.trim_start().to_string();
        }
        if l.is_empty() || l.starts_with('#') || l.starts_with(';') {
            continue;
        }
        if section.is_empty() {
            return None;
        }
        let klen = l.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-')).unwrap_or(l.len());
        if klen == 0 {
            return None;
        }
        let key = l[..klen].to_ascii_lowercase();
        let rest = l[klen..].trim_start();
        let value = if rest.is_empty() || rest.starts_with('#') || rest.starts_with(';') {
            String::new()
        } else {
            let mut v = rest.strip_prefix('=')?.to_string();
            // «\» в конце строки - продолжение значения на следующей
            while ends_with_escape(&v) {
                v.pop();
                v.push_str(lines.next().unwrap_or(""));
            }
            unescape(&v)?
        };
        out.push((format!("{section}.{key}"), value));
    }
    Some(out)
}

fn ends_with_escape(v: &str) -> bool {
    v.chars().rev().take_while(|c| *c == '\\').count() % 2 == 1
}

/// «[core]», «[filter "x"]», «[old.style]» -> имя секции и остаток строки после «]».
fn header(l: &str) -> Option<(String, &str)> {
    let body = &l[1..];
    let name_end = body.find(|c: char| c == ']' || c.is_whitespace() || c == '"')?;
    let name = body[..name_end].to_ascii_lowercase();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.') {
        return None;
    }
    let after = body[name_end..].trim_start();
    if let Some(rest) = after.strip_prefix(']') {
        return Some((name, rest));
    }
    let mut it = after.strip_prefix('"')?.char_indices();
    let mut sub = String::new();
    while let Some((i, c)) = it.next() {
        match c {
            '\\' => sub.push(it.next()?.1),
            '"' => {
                let rest = after[1 + i + 1..].trim_start().strip_prefix(']')?;
                return Some((format!("{name}.{sub}"), rest));
            }
            _ => sub.push(c),
        }
    }
    None
}

fn unescape(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut quoted = false;
    let mut it = raw.chars();
    while let Some(c) = it.next() {
        match c {
            '"' => quoted = !quoted,
            '\\' => match it.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'b' => {
                    out.pop();
                }
                x => out.push(x),
            },
            '#' | ';' if !quoted => break,
            _ => out.push(c),
        }
    }
    (!quoted).then(|| out.trim().to_string())
}

// ---------- состояние ----------
const LABEL: [(&str, &str); 8] = [
    ("M", "изменён"),
    ("A", "добавлен"),
    ("D", "удалён"),
    ("R", "переименован"),
    ("C", "скопирован"),
    ("U", "конфликт"),
    ("?", "новый"),
    ("!", "игнор"),
];

fn remote_of(name: &str, url: &str, trusted_hosts: &[String], before: &[Remote]) -> Remote {
    let parsed = parse_remote(url);
    let key = parsed.as_ref().map(|(h, s)| format!("{h}/{s}")).unwrap_or_else(|| url.to_string());
    let dup = before
        .iter()
        .any(|r| r.host.as_ref().zip(r.slug.as_ref()).map(|(h, s)| format!("{h}/{s}")).unwrap_or_else(|| r.url.clone()) == key);
    let host = parsed.as_ref().map(|p| p.0.clone());
    let trusted = host.as_ref().is_some_and(|h| trusted_hosts.iter().any(|t| t.eq_ignore_ascii_case(h)));
    Remote { name: name.into(), url: url.into(), host, slug: parsed.map(|p| p.1), dup, trusted }
}

/// Состояние без git: ветка из HEAD, remote из прочитанных настроек.
fn offline(dir: &Path, loc: &Located, v: &Verdict, trusted_hosts: &[String]) -> GitInfo {
    let branch = if v.refused {
        "?".to_string()
    } else {
        match read_head(&loc.git_dir.join("HEAD"), 512) {
            Some(h) => match h.trim().strip_prefix("ref:") {
                Some(r) => r.trim().trim_start_matches("refs/heads/").to_string(),
                None => "HEAD (отсоединён)".into(),
            },
            None => "?".into(),
        }
    };
    let mut remotes: Vec<Remote> = Vec::new();
    for (k, url) in &loc.entries {
        if let Some(name) = k.strip_prefix("remote.").and_then(|r| r.strip_suffix(".url")) {
            let r = remote_of(name, url, trusted_hosts, &remotes);
            remotes.push(r);
        }
    }
    let prefix = rel(&loc.work, dir);
    GitInfo {
        toplevel: loc.work.to_string_lossy().into_owned(),
        prefix: if prefix == "." { String::new() } else { prefix },
        branch,
        remotes,
        untrusted: true,
        refused: v.refused,
        why: Some(v.why.clone()),
        why_key: v.key.clone(),
        ..Default::default()
    }
}

/// Записей в stash: строки журнала `logs/refs/stash` в общей папке git. Нет журнала - нет stash.
fn stash_count(common: &Path) -> u32 {
    read_head(&common.join("logs").join("refs").join("stash"), 8 << 20)
        .map(|t| t.lines().filter(|l| !l.trim().is_empty()).count() as u32)
        .unwrap_or(0)
}

/// Состояние репозитория. Доверие пользователя - из настроек (`store::repo_trusted`).
pub fn info(dir: &Path, trusted_hosts: &[String]) -> Option<GitInfo> {
    info_with(dir, trusted_hosts, crate::store::repo_trusted)
}

/// То же, но кому доверять - решает `is_trusted` (корень рабочей папки репозитория).
pub fn info_with(dir: &Path, trusted_hosts: &[String], is_trusted: impl Fn(&Path) -> bool) -> Option<GitInfo> {
    let loc = locate(dir)?;
    if let Some(v) = &loc.verdict {
        if v.refused || !is_trusted(&loc.work) {
            return Some(offline(dir, &loc, v, trusted_hosts));
        }
    }
    let top = git(dir, &["rev-parse", "--show-toplevel", "--absolute-git-dir", "--git-common-dir"])?;
    let mut lines = top.lines();
    let toplevel = PathBuf::from(lines.next()?.trim());
    let git_dir = PathBuf::from(lines.next()?.trim());
    // git нашёл не ту папку, что проверили мы, - дальше не идём
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    if canon(&git_dir) != canon(&loc.git_dir) {
        return Some(offline(dir, &loc, &Verdict::untrusted("mismatch", None), trusted_hosts));
    }
    let common_raw = lines.next().unwrap_or("").trim().to_string();
    let common = if common_raw.is_empty() {
        git_dir.clone()
    } else if Path::new(&common_raw).is_absolute() {
        PathBuf::from(&common_raw)
    } else {
        dir.join(&common_raw)
    };
    let prefix = {
        let d = std::fs::canonicalize(dir).unwrap_or(dir.to_path_buf());
        let t = std::fs::canonicalize(&toplevel).unwrap_or(toplevel.clone());
        let r = rel(&t, &d);
        if r == "." {
            String::new()
        } else {
            r
        }
    };

    // stash считается по журналу refs/stash - это те же строки, что печатает `git stash list`, но
    // без ещё одного процесса git (40-100 мс на запуск в Windows). У хранилища ссылок reftable
    // журнала-файла нет - там спрашиваем git, как раньше.
    let reftable = loc.entries.iter().any(|(k, v)| k == "extensions.refstorage" && v.eq_ignore_ascii_case("reftable"));
    let (st, log, rem, stash, tag) = std::thread::scope(|s| {
        let a = s.spawn(|| git(dir, &["status", "--porcelain=v1", "-b", "-z", "--untracked-files=normal", "--ignore-submodules=all"]));
        let b = s.spawn(|| git(dir, &["log", "-n", "20", "--no-show-signature", "--format=%h%x1f%s%x1f%an%x1f%cI%x1f%D"]));
        let c = s.spawn(|| git(dir, &["remote", "-v"]));
        let d = s.spawn(|| {
            if reftable {
                git(dir, &["stash", "list", "--format=%gd"]).map(|t| t.lines().filter(|l| !l.is_empty()).count() as u32)
            } else {
                Some(stash_count(&common))
            }
        });
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
        let code = if x == "?" {
            "?"
        } else if x == "U" || y == "U" || (x == "A" && y == "A") || (x == "D" && y == "D") {
            "U"
        } else if x != " " {
            x
        } else {
            y
        };
        if files.len() < 400 {
            files.push(GitFile {
                path: p,
                code: code.into(),
                label: LABEL.iter().find(|(k, _)| *k == code).map(|(_, v)| v.to_string()).unwrap_or_else(|| code.to_string()),
                staged: x != " " && x != "?",
            });
        }
    }

    let mut remotes: Vec<Remote> = Vec::new();
    for l in rem.unwrap_or_default().lines() {
        let mut it = l.split_whitespace();
        let (Some(name), Some(url), Some(kind)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        if kind != "(fetch)" {
            continue;
        }
        let r = remote_of(name, url, trusted_hosts, &remotes);
        remotes.push(r);
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
                refs: p
                    .get(4)
                    .map(|r| r.split(", ").filter(|x| !x.is_empty() && !x.starts_with("HEAD")).take(4).map(str::to_string).collect())
                    .unwrap_or_default(),
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
        stash: stash.unwrap_or(0),
        tag: tag.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()),
        fetched_at: (fetched > 0).then_some(fetched),
        op,
        remotes,
        commits,
        // доверенный пользователем репозиторий с опасными настройками - помечаем, но git зовём
        untrusted: false,
        refused: false,
        why: loc.verdict.as_ref().map(|v| v.why.clone()),
        why_key: loc.verdict.and_then(|v| v.key),
    })
}
