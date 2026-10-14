//! «Спросить у программы»: какую справку спрашивать - подкоманды или общую. Шаблоны - поле `help`
//! в базе знаний; здесь проверяется, что они применяются к настоящим командам.
mod common;

use pult_core::explain::{explain, Ctx};
use pult_core::helpcmd::pick;
use pult_core::Script;
use std::collections::HashMap;

/// Скрипт с командой `cmd` (как стандартная команда манифеста) и его объяснением.
fn script(cmd: &str) -> Script {
    let kb = common::kb();
    let root = common::repo_root();
    let argv = pult_core::explain::tokens(cmd);
    let mut s = Script {
        name: cmd.into(),
        cmd: cmd.into(),
        tool: argv.first().map(|a| pult_core::explain::word_of(a)),
        argv,
        raw: vec![cmd.into()],
        source: "git".into(),
        where_: ".".into(),
        ..Default::default()
    };
    let pkg = HashMap::new();
    s.info = explain(&s, &root, &Ctx { root: &root, kb: &kb, pkg_scripts: &pkg, depth: 0 });
    s
}

fn variants(cmd: &str) -> Option<(String, Vec<String>)> {
    let kb = common::kb();
    pick(&script(cmd), &kb).map(|(bin, v)| (bin, v.iter().map(|a| a.join(" ")).collect()))
}

fn v(bin: &str, list: &[&str]) -> Option<(String, Vec<String>)> {
    Some((bin.to_string(), list.iter().map(|s| s.to_string()).collect()))
}

#[test]
fn git_status_asks_subcommand_help_first() {
    // `git status --help` открыл бы страницу в браузере: у git шаблон «{bin} {sub} -h»
    assert_eq!(variants("git status"), v("git", &["status -h", "-h"]));
}

#[test]
fn cargo_check_asks_cargo_check_help() {
    assert_eq!(variants("cargo check"), v("cargo", &["check --help", "--help"]));
}

#[test]
fn npm_run_asks_npm_run_help() {
    assert_eq!(variants("npm run build"), v("npm", &["run --help", "--help"]));
}

#[test]
fn multiword_bin_keeps_its_words() {
    assert_eq!(variants("docker compose up -d"), v("docker", &["compose up --help", "compose --help"]));
}

#[test]
fn go_uses_help_word_before_subcommand() {
    assert_eq!(variants("go build ./..."), v("go", &["help build", "help"]));
}

#[test]
fn interpreter_never_gets_a_file_as_subcommand() {
    // node scripts/x.js --help запустил бы чужой файл: у runsFile только общая справка
    assert_eq!(variants("node scripts/build.js"), v("node", &["--help"]));
    assert_eq!(variants("python -m pytest"), v("python", &["--help"]));
}

#[test]
fn subcommand_must_follow_program_directly() {
    // ключ между программой и словом: слово может быть значением ключа, а не подкомандой
    let (_, list) = variants("docker compose -f x.yml up").expect("docker");
    assert!(list.iter().all(|a| !a.contains("x.yml")), "{list:?}");
    assert_eq!(list.last().map(String::as_str), Some("compose --help"));
}

#[test]
fn unknown_program_gets_plain_help() {
    assert_eq!(variants("pulttool-unknown build"), v("pulttool-unknown", &["--help"]));
}

#[test]
fn help_null_means_do_not_ask() {
    assert_eq!(variants("echo hi"), None);
}

#[test]
fn every_kb_help_pattern_starts_with_bin() {
    let kb = common::kb();
    for t in &kb.tools {
        for p in t.help_patterns() {
            assert!(p.starts_with("{bin}"), "{}: {p}", t.id);
        }
    }
}

#[test]
fn git_status_help_really_lists_status_keys() {
    // живой прогон: справка git status из папки данных, а не общий `git --help` без ключей
    if pult_core::runner::which("git").is_none() {
        return;
    }
    let d = std::env::temp_dir().join(format!("pult-test-help-data-{}", std::process::id()));
    std::env::set_var("PULT_DATA", &d);
    let kb = common::kb();
    let root = common::repo_root();
    let h = pult_core::helpcmd::ask(&script("git status"), &root, &kb);
    assert!(h.error.is_none() && h.confirm.is_none(), "{:?} {:?}", h.error, h.confirm);
    assert!(h.argv.iter().any(|a| a == "status"), "спрошена справка подкоманды: {:?}", h.argv);
    assert!(
        h.flags.iter().any(|f| f.flag.contains("--short")),
        "ключи git status: {:?}",
        h.flags.iter().map(|f| &f.flag).collect::<Vec<_>>()
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn parse_reads_git_style_help() {
    let text = "usage: git status [<options>] [--] [<pathspec>...]\n\n    -v, --[no-]verbose    be verbose\n    -s, --[no-]short      show status concisely\n    --[no-]porcelain[=<version>]\n                          machine-readable output\n    -u, --[no-]untracked-files[=<mode>]\n                          show untracked files, optional modes: all, normal, no.\n    --no-renames          do not detect renames\n";
    let (usage, _, flags) = pult_core::helpcmd::parse(text);
    assert!(usage.starts_with("usage: git status"));
    let f: Vec<(&str, &str)> = flags.iter().map(|f| (f.flag.as_str(), f.text.as_str())).collect();
    assert_eq!(
        f,
        [
            ("-v, --verbose", "be verbose"),
            ("-s, --short", "show status concisely"),
            ("--porcelain[=<version>]", "machine-readable output"),
            ("-u, --untracked-files[=<mode>]", "show untracked files, optional modes: all, normal, no."),
            ("--no-renames", "do not detect renames"),
        ]
    );
}

#[test]
fn parse_keeps_classic_help_and_skips_bare_keys() {
    let text = "Usage: tool [options]\n\nOptions:\n  -w, --watch          Watch files\n                       and rerun\n  --out <dir>          Output folder\n  --bare\n";
    let (_, _, flags) = pult_core::helpcmd::parse(text);
    let f: Vec<(&str, &str)> = flags.iter().map(|f| (f.flag.as_str(), f.text.as_str())).collect();
    assert_eq!(f, [("-w, --watch", "Watch files and rerun"), ("--out <dir>", "Output folder")]);
}

/// Скрипт package.json: команда - «npm run <имя>», а внутри - `raw`.
fn npm_script(raw: &str) -> Script {
    let kb = common::kb();
    let root = common::repo_root();
    let mut s = Script {
        name: "lint".into(),
        cmd: "npm run lint".into(),
        argv: vec!["npm".into(), "run".into(), "lint".into()],
        tool: Some("npm".into()),
        raw: vec![raw.into()],
        source: "package.json".into(),
        where_: ".".into(),
        ..Default::default()
    };
    let pkg = HashMap::new();
    s.info = explain(&s, &root, &Ctx { root: &root, kb: &kb, pkg_scripts: &pkg, depth: 0 });
    s
}

#[test]
fn npm_script_asks_its_own_program_not_npm() {
    let kb = common::kb();
    let got = pick(&npm_script("pulttool-unknown --check"), &kb).map(|(b, v)| (b, v.iter().map(|a| a.join(" ")).collect::<Vec<_>>()));
    assert_eq!(got, v("pulttool-unknown", &["--help"]));
    // первая программа команды, а не первая знакомая базе
    let got = pick(&npm_script("pulttool-unknown --check && eslint ."), &kb).map(|(b, _)| b);
    assert_eq!(got.as_deref(), Some("pulttool-unknown"));
    // знакомая программа из базы - со своими шаблонами
    let got = pick(&npm_script("cargo check"), &kb).map(|(b, v)| (b, v.iter().map(|a| a.join(" ")).collect::<Vec<_>>()));
    assert_eq!(got, v("cargo", &["check --help", "--help"]));
}

#[cfg(windows)]
#[test]
fn npm_script_program_from_node_modules_needs_consent() {
    let kb = common::kb();
    let d = common::TempDir::new("help-npm-local");
    let marker = d.path().join("ran.txt");
    d.write("node_modules/.bin/pulttool-npm.cmd", &format!("@echo off\r\necho ran> \"{}\"\r\n", marker.display()));
    let mut s = npm_script("pulttool-npm --check");
    s.info = explain(&s, d.path(), &Ctx { root: d.path(), kb: &kb, pkg_scripts: &HashMap::new(), depth: 0 });
    let h = pult_core::helpcmd::ask(&s, d.path(), &kb);
    let path = h.confirm.clone().expect("программа из node_modules проекта - только с согласия");
    assert!(path.ends_with("pulttool-npm.cmd"), "{path}");
    assert!(!marker.exists());
}
