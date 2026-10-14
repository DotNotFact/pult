//! Число записей stash читается из журнала refs/stash, без процесса `git stash list`.
//! Сверка с самим git: столько же, сколько он печатает, и ноль, когда stash пуст.
mod common;

use common::TempDir;
use pult_core::git;
use std::path::Path;
use std::process::Command;

fn have_git() -> bool {
    Command::new("git").arg("--version").output().is_ok_and(|o| o.status.success())
}

fn run_git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "core.hooksPath=", "-c", "commit.gpgsign=false"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git");
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn stash_count_matches_git_stash_list() {
    std::env::set_var("PULT_DATA", std::env::temp_dir().join(format!("pult-test-data-{}", std::process::id())));
    common::kb();
    if !have_git() {
        eprintln!("git не найден - тест пропущен");
        return;
    }
    let d = TempDir::new("git-stash");
    run_git(d.path(), &["init", "-q"]);
    d.write("a.txt", "1\n");
    run_git(d.path(), &["add", "a.txt"]);
    run_git(d.path(), &["commit", "-q", "-m", "first"]);

    let g = git::info_with(d.path(), &[], |_| true).expect("репозиторий");
    assert_eq!(g.stash, 0, "stash пуст");

    for n in ["2\n", "3\n", "4\n"] {
        d.write("a.txt", n);
        run_git(d.path(), &["stash", "-q"]);
    }
    let want = run_git(d.path(), &["stash", "list"]).lines().filter(|l| !l.is_empty()).count() as u32;
    assert_eq!(want, 3, "фикстура: три записи stash");
    let g = git::info_with(d.path(), &[], |_| true).expect("репозиторий");
    assert_eq!(g.stash, want, "столько же, сколько печатает git stash list");

    run_git(d.path(), &["stash", "drop", "-q"]);
    let g = git::info_with(d.path(), &[], |_| true).expect("репозиторий");
    assert_eq!(g.stash, 2, "после drop");
}
