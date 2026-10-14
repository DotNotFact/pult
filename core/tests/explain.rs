//! Объяснение команд по настоящей базе знаний из knowledge/.
mod common;

use pult_core::explain::{explain, split_segments, word_of, Ctx};
use pult_core::{Info, Script};
use std::collections::HashMap;

/// Объяснить команду так, как будто это скрипт package.json в корне.
fn explain_with(cmd: &str, siblings: &[(&str, &str)]) -> Info {
    let kb = common::kb();
    let root = common::repo_root();
    let mut pkg: HashMap<String, HashMap<String, String>> = HashMap::new();
    let mine = pkg.entry(".".into()).or_default();
    for (n, c) in siblings {
        mine.insert(n.to_string(), c.to_string());
    }
    let s = Script {
        name: "проверяемый".into(),
        cmd: cmd.into(),
        raw: vec![cmd.into()],
        source: "package.json".into(),
        where_: ".".into(),
        ..Default::default()
    };
    explain(&s, &root, &Ctx { root: &root, kb: &kb, pkg_scripts: &pkg, depth: 0 })
}

fn ex(cmd: &str) -> Info {
    explain_with(cmd, &[])
}

#[test]
fn docker_compose_up_is_compose_not_long_with_docker_effect() {
    let i = ex("docker compose up -d --build");
    assert!(i.known);
    assert_eq!(i.tools, vec!["docker-compose".to_string()], "программа по самому длинному совпадению слов");
    assert!(!i.long, "-d отпускает терминал: команда не долгая");
    assert!(i.fx.iter().any(|f| f == "docker"), "fx: {:?}", i.fx);
    let kinds: Vec<&str> = i.parts.iter().map(|p| p.kind.as_str()).collect();
    assert_eq!(kinds, ["bin", "sub", "flag", "flag"], "части: {:?}", i.parts);
    assert_eq!(i.parts[0].text, "docker compose");
    assert_eq!(i.parts[1].text, "up");
    assert_eq!(i.source, "kb");
}

#[test]
fn next_build_is_not_long() {
    let i = ex("next build");
    assert!(i.known);
    assert!(!i.long, "сборка заканчивается сама");
}

#[test]
fn vitest_watch_is_long() {
    let i = ex("vitest --watch");
    assert!(i.known);
    assert!(i.long, "наблюдение за файлами работает, пока его не остановить");
}

#[test]
fn rimraf_deletes() {
    let i = ex("rimraf dist");
    assert!(i.fx.iter().any(|f| f == "delete"), "fx: {:?}", i.fx);
    assert!(!i.fx.iter().any(|f| f == "read"), "удаление не бывает «только чтением»");
}

#[test]
fn git_push_force_is_danger() {
    let i = ex("git push --force");
    assert!(i.fx.iter().any(|f| f == "danger"), "fx: {:?}", i.fx);
    // от обычного push (по базе знаний он тоже под подтверждением) отличается подкомандой
    let sub = i.parts.iter().find(|p| p.kind == "sub").expect("подкоманда push");
    assert!(sub.note.contains("перезапис"), "часть: {sub:?}");
    assert!(i.parts.iter().any(|p| p.kind == "flag" && p.text == "--force"));
}

#[test]
fn chain_of_script_refs_is_expanded() {
    let i = explain_with("pnpm run lint && pnpm run test", &[("lint", "eslint ."), ("test", "vitest run")]);
    assert_eq!(i.refs, vec!["lint".to_string(), "test".to_string()]);
    assert!(i.summary.starts_with("По очереди:"), "summary: {}", i.summary);
    assert!(i.summary.contains("«lint»") && i.summary.contains("«test»"), "summary: {}", i.summary);
    assert!(i.known, "обе ссылки ведут на знакомые программы");
    assert!(i.tools.iter().any(|t| t == "eslint"), "программы из раскрытых скриптов: {:?}", i.tools);
    assert!(i.tools.iter().any(|t| t == "vitest"), "программы из раскрытых скриптов: {:?}", i.tools);
}

#[test]
fn missing_script_ref_is_named_not_expanded() {
    let i = ex("pnpm run nope");
    assert_eq!(i.refs, vec!["nope".to_string()]);
    assert!(i.summary.contains("nope"), "summary: {}", i.summary);
}

#[test]
fn unknown_program_is_not_known() {
    let i = ex("zzqx-neizvestnaya-programma --flag");
    assert!(!i.known);
    assert!(i.tools.is_empty());
    assert_eq!(i.parts[0].kind, "bin");
    assert!(i.fx.is_empty(), "о незнакомой программе не выдумываем последствия: {:?}", i.fx);
}

#[test]
fn split_segments_respects_quotes() {
    assert_eq!(split_segments("a && b | c ; d"), ["a", "b", "c", "d"]);
    assert_eq!(split_segments(r#"echo "a && b" && c"#), [r#"echo "a && b""#, "c"]);
    assert_eq!(split_segments("a || b"), ["a", "b"]);
}

#[test]
fn word_of_strips_path_and_extension() {
    assert_eq!(word_of(r"node_modules\.bin\Vitest.CMD"), "vitest");
    assert_eq!(word_of("./scripts/build.ps1"), "build");
    assert_eq!(word_of("git"), "git");
}

#[test]
fn tokens_keep_windows_backslashes() {
    use pult_core::explain::tokens;
    // обёртка gradlew/mvnw в стандартной команде - полный путь Windows; sh-разбор съедал «\»
    assert_eq!(tokens(r"D:\p\android\gradlew.bat build"), [r"D:\p\android\gradlew.bat", "build"]);
    assert_eq!(tokens(r"node scripts\build.js --out dist"), ["node", r"scripts\build.js", "--out", "dist"]);
    assert_eq!(tokens(r#"node "C:\My Tools\x.js" 'a b'"#), ["node", r"C:\My Tools\x.js", "a b"]);
    assert_eq!(tokens(r#"echo \"hi\" a\ b"#), ["echo", r#""hi""#, "a b"]);
    assert_eq!(tokens(r#"cross-env A="x \"y\"" b"#), ["cross-env", r#"A=x "y""#, "b"]);
    assert_eq!(tokens("  a   b  "), ["a", "b"]);
    assert_eq!(tokens(r#"a """#), ["a", ""], "пустые кавычки - пустой аргумент");
}

#[test]
fn gradle_wrapper_by_full_path_is_gradle() {
    let i = ex(r"D:\proj\android\gradlew.bat build");
    assert!(i.known, "{}", i.summary);
    assert_eq!(i.tools, vec!["gradle".to_string()]);
    assert_eq!(i.parts[0].text, r"D:\proj\android\gradlew.bat", "путь в разборе команды не теряет обратную косую");
}

#[test]
fn react_scripts_are_known() {
    let start = ex("react-scripts start");
    assert!(start.known && start.long && start.url, "{}", start.summary);
    let test = ex("react-scripts test --watchAll=false");
    assert!(test.known && !test.long, "один прогон тестов не долгий: {}", test.summary);
    assert!(ex("react-scripts eject").fx.iter().any(|f| f == "danger"));
}

#[test]
fn npm_default_test_script_is_understood() {
    let i = ex(r#"echo "Error: no test specified" && exit 1"#);
    assert!(i.known);
    assert_eq!(i.tools, vec!["echo".to_string(), "exit".to_string()]);
    assert!(!i.summary.contains("выполняет «"), "{}", i.summary);
}

#[test]
fn lead_joins_wrapped_first_sentence() {
    use pult_core::explain::lead;
    // фраза перенесена на вторую строку после запятой - берётся целиком, до точки
    let h = "Включает CI на машине: дистрибутив, держатель,\nDocker Engine и раннер. Пара к ci-off.ps1.\n\nПодробности ниже.";
    assert_eq!(lead(h), "Включает CI на машине: дистрибутив, держатель, Docker Engine и раннер.");
    // продолжение со строчной буквы
    assert_eq!(lead("Освобождает порты, не перезагружая\nмашину. Ещё фраза."), "Освобождает порты, не перезагружая машину.");
    // заголовок без точки и новая мысль с заглавной - не склеиваются
    assert_eq!(lead("Сборка плеера\nЗапуск: node x.js"), "Сборка плеера");
    // пункт списка - новая мысль
    assert_eq!(lead("Что делает:\n- первое\n- второе"), "Что делает:");
    // длинная строка перенесена по ширине: продолжение с заглавной или с ключа - та же фраза
    assert_eq!(
        lead("Замер одной командой: рвётся ли сеть из контейнера, и рвётся ли она\nС ХОСТА теми же запросами.\nДальше."),
        "Замер одной командой: рвётся ли сеть из контейнера, и рвётся ли она С ХОСТА теми же запросами."
    );
    assert_eq!(
        lead("Поднимает Docker: Docker Engine в WSL, а с ключом\n-Desktop - Docker Desktop."),
        "Поднимает Docker: Docker Engine в WSL, а с ключом -Desktop - Docker Desktop."
    );
    // одна строка - как была
    assert_eq!(lead("Проверка хука."), "Проверка хука.");
    // «т.е. в» и имя файла с точкой не считаются концом фразы
    assert_eq!(lead("Читает файл a.json, т.е. в нём настройки."), "Читает файл a.json, т.е. в нём настройки.");
}
