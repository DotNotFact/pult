//! Защиты от чужой папки: git с опасными настройками, сетевые пути, адрес remote для CI,
//! справка программы из проекта, поиск базы знаний, слежение за файлами.
mod common;

use common::TempDir;
use pult_core::git::{self, parse_config, parse_remote, valid_slug};
use pult_core::helpcmd;
use pult_core::model::Script;
use pult_core::paths::home_candidates;
use pult_core::scan::{scan, watch_kind, ScanOpts};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;

/// Данные Пульта (кэш, настройки) этих тестов - во временной папке, а не в настоящей.
fn isolate() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let d = std::env::temp_dir().join(format!("pult-test-data-{}", std::process::id()));
        std::env::set_var("PULT_DATA", d);
    });
    common::kb();
}

fn have_git() -> bool {
    Command::new("git").arg("--version").output().is_ok_and(|o| o.status.success())
}

fn run_git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git").args(args).current_dir(dir).output().is_ok_and(|o| o.status.success());
    assert!(ok, "git {args:?}");
}

fn slash(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Репозиторий, в котором `git status` запускает фильтр `x` из .git/config: фильтр создаёт файл-метку.
/// Файл a.txt в индексе, но с другим временем изменения - git обязан пересчитать его хеш через фильтр.
fn repo_with_filter(d: &TempDir, marker: &Path) {
    run_git(d.path(), &["init", "-q"]);
    d.write("a.txt", "hello\n");
    run_git(d.path(), &["add", "a.txt"]);
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(120);
    std::fs::File::options().write(true).open(d.path().join("a.txt")).and_then(|f| f.set_modified(later)).expect("время файла");
    d.write(".gitattributes", "* filter=x\n");
    let cfg = d.path().join(".git").join("config");
    let mut text = std::fs::read_to_string(&cfg).expect("config");
    text.push_str(&format!("[filter \"x\"]\n\tclean = \"echo hit > '{}' && cat\"\n", slash(marker)));
    std::fs::write(&cfg, text).expect("запись config");
}

#[test]
fn git_filter_repo_is_untrusted_and_filter_does_not_run() {
    isolate();
    if !have_git() {
        eprintln!("git не найден - тест пропущен");
        return;
    }
    let d = TempDir::new("git-filter");
    let marker = d.path().join("filter-ran.txt");
    repo_with_filter(&d, &marker);

    let g = git::info_with(d.path(), &[], |_| false).expect("репозиторий найден");
    assert!(g.untrusted, "фильтр в .git/config - не доверяем");
    assert!(!g.refused, "доверить можно: сетевого пути нет");
    assert_eq!(g.why.as_deref(), Some("key"));
    assert_eq!(g.why_key.as_deref(), Some("filter.x.clean"));
    assert!(g.files.is_empty() && g.commits.is_empty(), "git status и log не запускались");
    assert!(!marker.exists(), "фильтр из чужого .git/config не должен запуститься при открытии папки");

    // контроль: фикстура способна показать дефект - после «Доверять» git status запускает фильтр
    let g = git::info_with(d.path(), &[], |_| true).expect("репозиторий найден");
    assert!(!g.untrusted);
    assert!(marker.exists(), "доверенный репозиторий: git status пересчитал a.txt через фильтр");
}

#[test]
fn git_file_pointing_to_unc_is_refused() {
    isolate();
    let d = TempDir::new("git-unc");
    d.write(".git", "gitdir: //pult-test.invalid/share/repo.git\n");
    let loc = git::locate(d.path()).expect("файл .git найден");
    let v = loc.verdict.expect("сетевой gitdir не годится");
    assert!(v.refused, "сетевой путь не доверяется даже по кнопке");
    let g = git::info_with(d.path(), &[], |_| true).expect("ответ есть");
    assert!(g.untrusted && g.refused, "даже доверенный - git не запускается");
    assert_eq!(g.why.as_deref(), Some("unc"));

    let d2 = TempDir::new("git-unc2");
    d2.write(".git", "gitdir: \\\\pult-test.invalid\\share\\repo.git\n");
    assert!(git::locate(d2.path()).and_then(|l| l.verdict).is_some_and(|v| v.refused));
}

fn bare_git_dir(d: &TempDir, config: &str) {
    d.write(".git/HEAD", "ref: refs/heads/main\n");
    d.write(".git/config", config);
}

#[test]
fn unc_values_and_alternates_are_refused() {
    isolate();
    let d = TempDir::new("git-include");
    bare_git_dir(&d, "[core]\n\tbare = false\n[include]\n\tpath = \"\\\\\\\\pult-test.invalid\\\\share\\\\x\"\n");
    let v = git::locate(d.path()).and_then(|l| l.verdict).expect("include с сетевым путём");
    assert!(v.refused, "{v:?}");
    assert_eq!(v.key.as_deref(), Some("include.path"));

    let d = TempDir::new("git-excludes");
    bare_git_dir(&d, "[core]\n\texcludesFile = //pult-test.invalid/share/ignore\n");
    assert!(git::locate(d.path()).and_then(|l| l.verdict).is_some_and(|v| v.refused));

    let d = TempDir::new("git-alt");
    bare_git_dir(&d, "[core]\n\tbare = false\n");
    d.write(".git/objects/info/alternates", "\\\\pult-test.invalid\\share\\objects\n");
    let v = git::locate(d.path()).and_then(|l| l.verdict).expect("alternates на сетевой папке");
    assert!(v.refused);
    assert_eq!(v.why, "alternates");
}

#[test]
fn risky_keys_make_repo_untrusted() {
    isolate();
    for (cfg, key) in [
        ("[gpg]\n\tprogram = evil.exe\n", "gpg.program"),
        ("[log]\n\tshowSignature = true\n[gpg \"ssh\"]\n\tprogram = x\n", "gpg.ssh.program"),
        ("[includeIf \"gitdir:/x/\"]\n\tpath = other.cfg\n", "includeif.gitdir:/x/.path"),
        ("[core]\n\thooksPath = h\n", "core.hookspath"),
        ("[core]\n\tattributesFile = a\n", "core.attributesfile"),
        ("[core]\n\tsshCommand = x\n", "core.sshcommand"),
        ("[core]\n\tpager = x\n", "core.pager"),
        ("[diff \"bin\"]\n\ttextconv = x\n", "diff.bin.textconv"),
        ("[Filter \"lfs\"]\n\tprocess = x\n", "filter.lfs.process"),
    ] {
        let d = TempDir::new("git-risky");
        bare_git_dir(&d, cfg);
        let v = git::locate(d.path()).and_then(|l| l.verdict).unwrap_or_else(|| panic!("{key}: должен быть недоверенным"));
        assert!(!v.refused, "{key}");
        assert_eq!(v.key.as_deref(), Some(key));
    }
    // обычные настройки - доверяем
    let d = TempDir::new("git-plain");
    bare_git_dir(&d, "[core]\n\trepositoryformatversion = 0\n\tfsmonitor = true\n[remote \"origin\"]\n\turl = git@github.com:o/r.git\n");
    assert_eq!(git::locate(d.path()).and_then(|l| l.verdict), None);
}

#[test]
fn untrusted_repo_reports_branch_and_remotes_without_git() {
    isolate();
    let d = TempDir::new("git-offline");
    bare_git_dir(&d, "[remote \"origin\"]\n\turl = https://gitlab.com/a/b/c.git\n[filter \"x\"]\n\tclean = x\n");
    let g = git::info_with(d.path(), &["gitlab.com".to_string()], |_| false).expect("ответ");
    assert!(g.untrusted);
    assert_eq!(g.branch, "main");
    assert_eq!(g.remotes.len(), 1);
    assert_eq!(g.remotes[0].slug.as_deref(), Some("a/b/c"));
    assert!(g.remotes[0].trusted);
}

#[test]
fn git_dir_outside_project_is_untrusted_but_worktree_is_not() {
    isolate();
    let other = TempDir::new("git-elsewhere");
    other.write("HEAD", "ref: refs/heads/main\n");
    other.write("config", "[core]\n\tbare = false\n");
    let d = TempDir::new("git-outside");
    d.write(".git", &format!("gitdir: {}\n", slash(other.path())));
    let v = git::locate(d.path()).and_then(|l| l.verdict).expect("папка git вне проекта");
    assert!(!v.refused);
    assert_eq!(v.why, "outside");

    // настоящая рабочая копия worktree: основной репозиторий знает её адрес
    let main = TempDir::new("git-main");
    main.write(".git/HEAD", "ref: refs/heads/main\n");
    main.write(".git/config", "[core]\n\tbare = false\n");
    let wt = TempDir::new("git-wt");
    let gd = main.path().join(".git").join("worktrees").join("w");
    main.write(".git/worktrees/w/HEAD", "ref: refs/heads/feature\n");
    main.write(".git/worktrees/w/commondir", "../..\n");
    main.write(".git/worktrees/w/gitdir", &format!("{}\n", slash(&wt.path().join(".git"))));
    wt.write(".git", &format!("gitdir: {}\n", slash(&gd)));
    let loc = git::locate(wt.path()).expect("worktree найден");
    assert_eq!(loc.verdict, None, "worktree с обратной ссылкой - обычный репозиторий");

    // чужая рабочая копия: обратная ссылка ведёт не сюда
    let stranger = TempDir::new("git-stranger");
    stranger.write(".git", &format!("gitdir: {}\n", slash(&gd)));
    assert_eq!(git::locate(stranger.path()).and_then(|l| l.verdict).map(|v| v.why), Some("outside".into()));
}

#[test]
fn parse_config_handles_sections_quotes_and_continuations() {
    let e = parse_config(
        "# comment\n[core]\n\tbare\n\tFsMonitor = false ; c\n[filter \"a b\"]\n\tclean = \"x # y\" # c\n[Remote.Origin]\n\turl = one\\\n two\n[x] k = v\n",
    )
    .expect("разобрался");
    let get = |k: &str| e.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str());
    assert_eq!(get("core.bare"), Some(""));
    assert_eq!(get("core.fsmonitor"), Some("false"));
    assert_eq!(get("filter.a b.clean"), Some("x # y"));
    assert_eq!(get("remote.origin.url"), Some("one two"));
    assert_eq!(get("x.k"), Some("v"));
    assert_eq!(parse_config("k = v\n"), None, "ключ без секции - не разбор");
    assert_eq!(parse_config("[core\n"), None);
    let e = parse_config("[a]\n\tp = \"\\\\\\\\host\\\\share\"\n").expect("разобрался");
    assert_eq!(e[0].1, r"\\host\share");
}

#[test]
fn remote_slug_must_not_smuggle_a_host() {
    // gh читает «evil.example/o/r» как ХОСТ/владелец/репозиторий
    assert_eq!(parse_remote("https://github.com/evil.example/o/r.git"), None);
    assert_eq!(parse_remote("git@github.com:evil.example/o/r.git"), None);
    assert_eq!(parse_remote("https://github.com/o/r.git"), Some(("github.com".into(), "o/r".into())));
    assert_eq!(parse_remote("git@github.com:o/r"), Some(("github.com".into(), "o/r".into())));
    // GitLab: вложенные группы можно
    assert_eq!(parse_remote("https://gitlab.com/g/sub/r.git"), Some(("gitlab.com".into(), "g/sub/r".into())));
    assert_eq!(parse_remote("git@gitlab.com:g/sub/r.git"), Some(("gitlab.com".into(), "g/sub/r".into())));
    assert!(valid_slug("gitlab.com", "a_b/c-d/e.f"));
    assert!(!valid_slug("gitlab.com", "a/../b"));
    assert!(!valid_slug("gitlab.com", "a/b c"));
    assert!(!valid_slug("gitlab.com", "a/b%2Fc"));
    assert!(!valid_slug("gitlab.com", "single"));
    assert!(!valid_slug("github.com", "a/b/c"));
    assert!(!valid_slug("github.com", "a/"));
    assert_eq!(parse_remote("https://user@evil host/o/r"), None);
}

#[test]
fn home_looks_only_next_to_exe_and_in_dev_repo() {
    let root = if cfg!(windows) { PathBuf::from(r"D:\src\pult") } else { PathBuf::from("/src/pult") };
    let exe = root.join("target").join("debug").join("Pult.exe");
    assert_eq!(home_candidates(&exe), vec![root.join("target").join("debug"), root.clone()]);
    let test = root.join("target").join("release").join("deps").join("security-1.exe");
    assert_eq!(home_candidates(&test), vec![root.join("target").join("release").join("deps"), root.clone()]);
    let installed =
        if cfg!(windows) { PathBuf::from(r"C:\Users\me\Downloads\pult\Pult.exe") } else { PathBuf::from("/home/me/Downloads/pult/Pult") };
    assert_eq!(home_candidates(&installed), vec![installed.parent().expect("папка").to_path_buf()], "выше папки exe не ищем");
}

fn help_script(tool: &str) -> Script {
    Script { id: "t".into(), name: tool.into(), where_: ".".into(), tool: Some(tool.into()), ..Default::default() }
}

#[cfg(windows)]
#[test]
fn help_does_not_run_project_binary_without_consent() {
    isolate();
    let kb = common::kb();
    let d = TempDir::new("help-local");
    let marker = d.path().join("ran.txt");
    d.write("node_modules/.bin/pulttool.cmd", &format!("@echo off\r\necho ran> \"{}\"\r\necho --flag   описание\r\n", marker.display()));
    let s = help_script("pulttool");
    let h = helpcmd::ask(&s, d.path(), &kb);
    let path = h.confirm.clone().expect("файл из проекта требует согласия");
    assert!(path.ends_with("pulttool.cmd"), "{path}");
    assert!(!marker.exists(), "без согласия файл из проекта не запускается");
    // согласие на другой путь не считается
    let h2 = helpcmd::ask_confirmed(&s, d.path(), &kb, Some("C:\\other.cmd"));
    assert!(h2.confirm.is_some() && !marker.exists());
    // контроль: с согласием запускается
    let h3 = helpcmd::ask_confirmed(&s, d.path(), &kb, Some(&path));
    assert!(h3.confirm.is_none(), "{:?}", h3.error);
    assert!(marker.exists(), "после согласия справка спрошена");
}

#[test]
fn watch_kind_follows_scan_rules() {
    let kb = common::kb();
    let root = if cfg!(windows) { PathBuf::from(r"C:\p") } else { PathBuf::from("/p") };
    let k = |rel: &str| watch_kind(&root, &root.join(rel), &kb);
    assert_eq!(k("package.json"), 1);
    assert_eq!(k("pnpm-lock.yaml"), 1);
    assert_eq!(k("Makefile"), 1);
    assert_eq!(k("svc/api/Api.csproj"), 1);
    assert_eq!(k("Cargo.toml"), 1);
    assert_eq!(k(".github/workflows/ci.yml"), 1);
    assert_eq!(k("build.ps1"), 1, "скрипты в корне разбор тоже находит");
    assert_eq!(k("scripts/deploy.sh"), 1);
    assert_eq!(k("deploy.py"), 0, "файлы .py в корне разбор пропускает");
    assert_eq!(k("src/app.js"), 0);
    assert_eq!(k("scripts/tool.js"), 0, ".js не запускается как файл-скрипт");
    assert_eq!(k(".git/HEAD"), 2);
    assert_eq!(k(".git/refs/heads/main"), 2);
    assert_eq!(k(".git/objects/ab/cd"), 0);
    assert_eq!(k("node_modules/.modules.yaml"), 1);
    assert_eq!(k("node_modules/react/index.js"), 0);
    assert_eq!(k("readme.md"), 1);
}

#[test]
fn cargo_package_header_with_comment_and_literal_strings() {
    isolate();
    let d = TempDir::new("cargo-pkg");
    d.write(
        "Cargo.toml",
        "[ package ] # главный\nname = 'lit-name'\nversion = \"1.2.3\" # версия\ndescription = \"say \\\"hi\\\" ok\" # c\n\n[dependencies]\nserde = \"1\"\n",
    );
    let p = scan(d.path(), &ScanOpts { trusted_hosts: vec![] }).expect("разбор");
    assert_eq!(p.name, "lit-name");
    assert_eq!(p.version, "1.2.3");
    assert_eq!(p.description, "say \"hi\" ok");
}

#[cfg(windows)]
#[test]
fn ls_skips_junctions() {
    let d = TempDir::new("junction");
    let outside = TempDir::new("junction-target");
    outside.write("package.json", "{}");
    std::fs::create_dir_all(d.path().join("real")).expect("папка");
    let ok = Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(d.path().join("link"))
        .arg(outside.path())
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok {
        eprintln!("junction не создалась - тест пропущен");
        return;
    }
    let names: Vec<String> = pult_core::util::ls(d.path()).iter().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, vec!["real".to_string()], "по ссылке разбор не идёт");
    assert_eq!(
        pult_core::util::resolve_inside(d.path(), &d.path().join("link").join("package.json")),
        None,
        "ссылка ведёт за пределы проекта"
    );
    assert!(pult_core::util::resolve_inside(d.path(), &d.path().join("real")).is_some());
}
