//! База знаний из knowledge/: загрузка и этапы по stages.json.
mod common;

#[test]
fn knowledge_loads_from_disk_without_errors() {
    let kb = common::kb();
    assert!(kb.errors.is_empty(), "ошибки базы знаний: {:#?}", kb.errors);
    assert!(kb.from_disk, "тест обязан читать knowledge/ репозитория, а не вшитую копию");
    assert!(!kb.tools.is_empty());
    assert!(!kb.topics.is_empty());
    assert!(kb.stages.iter().any(|s| s.id == "other"), "этап «Прочее» нужен для всего, что не опознано");
    // id программ уникальны: tool(id) отдаёт первое совпадение, второе было бы невидимо
    let mut ids: Vec<&str> = kb.tools.iter().map(|t| t.id.as_str()).collect();
    ids.sort_unstable();
    let before = ids.len();
    ids.dedup();
    assert_eq!(before, ids.len(), "повторяющиеся id программ");
}

#[test]
fn classify_by_script_name() {
    let kb = common::kb();
    for (name, want) in [
        ("dev", "dev"),
        ("start", "dev"),
        ("build", "build"),
        ("test", "check"),
        ("test:api", "check"),
        ("lint", "check"),
        ("deploy", "release"),
        ("deploy-prod", "release"),
        ("install", "setup"),
        ("clean", "ops"),
        ("docker:up", "dev"),
        ("zzz", "other"),
    ] {
        assert_eq!(kb.classify(name, ""), want, "этап скрипта «{name}»");
    }
}

#[test]
fn classify_needs_whole_word() {
    let kb = common::kb();
    // «runner» не «run», «rebuild» не «build»: слово должно стоять целиком
    assert_eq!(kb.classify("runner", ""), "ops");
    assert_eq!(kb.classify("prebuildify", ""), "other");
}

#[test]
fn classify_priority_lower_number_first() {
    let kb = common::kb();
    // release (1) раньше build (4), check (2) раньше build (4)
    assert_eq!(kb.classify("build:prod", ""), "release");
    assert_eq!(kb.classify("test:build", ""), "check");
}

#[test]
fn classify_falls_back_to_command_text() {
    let kb = common::kb();
    assert_eq!(kb.classify("x", "eslint . && tsc --noEmit"), "check");
    assert_eq!(kb.classify("x", "zzqx"), "other");
}

#[test]
fn match_tool_takes_longest_prefix() {
    let kb = common::kb();
    let w = |s: &str| s.split_whitespace().map(str::to_string).collect::<Vec<_>>();
    let (i, n) = kb.match_tool(&w("docker compose up -d")).expect("docker compose");
    assert_eq!((kb.tools[i].id.as_str(), n), ("docker-compose", 2));
    let (i, n) = kb.match_tool(&w("docker ps")).expect("docker");
    assert_eq!((kb.tools[i].id.as_str(), n), ("docker", 1));
    assert!(kb.match_tool(&w("zzqx run")).is_none());
}

#[test]
fn danger_name() {
    let kb = common::kb();
    assert!(kb.is_danger_name("deploy"));
    assert!(kb.is_danger_name("db:drop"));
    assert!(kb.is_danger_name("release"));
    assert!(!kb.is_danger_name("build"));
    assert!(!kb.is_danger_name("prerelease-notes"), "release опасен только в начале имени");
}

#[test]
fn danger_name_has_exceptions_from_data() {
    let kb = common::kb();
    assert!(kb.is_danger_name("release"));
    assert!(kb.is_danger_name("deploy:prod"));
    // пишут заметки или только показывают - не опасны, хотя слово похоже
    assert!(!kb.is_danger_name("release-notes"));
    assert!(!kb.is_danger_name("deploy:preview"));
    assert!(!kb.is_danger_name("deploy-dry-run"));
}

/// Программа из базы только с полем `help` (и `runsFile`, если задан).
fn tool_with_help(help: serde_json::Value, runs_file: bool) -> pult_core::kb::Tool {
    serde_json::from_value(serde_json::json!({ "id": "t", "name": "T", "bins": ["t"], "help": help, "runsFile": runs_file }))
        .expect("программа из JSON")
}

#[test]
fn help_patterns_empty_when_tool_has_no_help() {
    let t = tool_with_help(serde_json::Value::Null, false);
    assert!(t.help_patterns().is_empty());
}

#[test]
fn help_patterns_plain_key_asks_subcommand_first_then_program() {
    let t = tool_with_help("--help".into(), false);
    assert_eq!(t.help_patterns(), ["{bin} {sub} --help", "{bin} --help"]);
}

#[test]
fn help_patterns_plain_key_of_interpreter_asks_program_only() {
    // у runsFile слово после программы - файл: «node build.js --help» запустил бы его
    let t = tool_with_help("--help".into(), true);
    assert_eq!(t.help_patterns(), ["{bin} --help"]);
}

#[test]
fn help_patterns_template_string_is_kept_as_is() {
    let t = tool_with_help("{bin} help {sub}".into(), true);
    assert_eq!(t.help_patterns(), ["{bin} help {sub}"]);
}

#[test]
fn help_patterns_list_is_kept_in_order() {
    let t = tool_with_help(serde_json::json!(["{bin} {sub} -h", "{bin} -h"]), false);
    assert_eq!(t.help_patterns(), ["{bin} {sub} -h", "{bin} -h"]);
}
