//! ИИ-часть проекта (core/src/ai.rs): шапка markdown, JSON настроек, шаблоны путей, секреты, связи.
//! Проект собирается во временной папке; настоящая база знаний - из репозитория.
mod common;

use common::TempDir;
use pult_core::ai::{self, AiItem, AiMap, Fv, Glob};
use pult_core::Project;

fn build(d: &TempDir) -> AiMap {
    let kb = common::kb();
    let p = Project { path: d.path().to_string_lossy().into_owned(), ..Default::default() };
    ai::build(d.path(), &p, &kb)
}

fn item<'a>(m: &'a AiMap, id: &str) -> &'a AiItem {
    m.items
        .iter()
        .find(|i| i.id == id)
        .unwrap_or_else(|| panic!("нет детали {id}; есть: {:?}", m.items.iter().map(|i| &i.id).collect::<Vec<_>>()))
}

fn list(fm: &ai::Fm, key: &str) -> Vec<String> {
    fm.list(&[key.to_string()])
}

// ---------- шапка markdown ----------

#[test]
fn frontmatter_reads_plain_quoted_and_block_list() {
    let (fm, body) = ai::parse_frontmatter(
        "---\nname: reviewer\ndescription: \"Смотрит дифф: враждебно\"\npaths:\n  - \"apps/**\"\n  - 'docs/*.md'\n---\nТекст\n",
    );
    assert_eq!(fm.get("name"), Some(&Fv::Str("reviewer".into())));
    assert_eq!(fm.get("description"), Some(&Fv::Str("Смотрит дифф: враждебно".into())));
    assert_eq!(list(&fm, "paths"), vec!["apps/**", "docs/*.md"]);
    assert_eq!(body, "Текст\n");
}

#[test]
fn frontmatter_reads_inline_list_and_comma_string() {
    let (fm, _) = ai::parse_frontmatter("---\nglobs: [src/**/*.ts, \"tests/{a,b}/*.ts\"]\ntools: Read, Grep, Glob\n---\n");
    assert_eq!(list(&fm, "globs"), vec!["src/**/*.ts", "tests/{a,b}/*.ts"]);
    assert_eq!(list(&fm, "tools"), vec!["Read", "Grep", "Glob"]);
}

#[test]
fn frontmatter_reads_folded_and_literal_blocks() {
    let (fm, _) = ai::parse_frontmatter(
        "---\ndescription: >-\n  Первая строка\n  вторая строка\n\n  новый абзац\nnote: |\n  a\n  b\nlast: ok\n---\n",
    );
    assert_eq!(fm.get("description"), Some(&Fv::Str("Первая строка вторая строка\nновый абзац".into())));
    assert_eq!(fm.get("note"), Some(&Fv::Str("a\nb".into())));
    assert_eq!(fm.get("last"), Some(&Fv::Str("ok".into())));
}

#[test]
fn frontmatter_keeps_hash_inside_text_but_drops_real_comment() {
    let (fm, _) = ai::parse_frontmatter("---\ndescription: лендинг (до #160) на сайте  # комментарий\n---\n");
    assert_eq!(fm.get("description"), Some(&Fv::Str("лендинг (до #160) на сайте".into())));
}

#[test]
fn frontmatter_handles_crlf_bom_and_missing_close() {
    let (fm, body) = ai::parse_frontmatter("\u{feff}---\r\nname: x\r\n---\r\nтело");
    assert_eq!(fm.get("name"), Some(&Fv::Str("x".into())));
    assert_eq!(body, "тело");
    let (fm, body) = ai::parse_frontmatter("---\nname: x\nнет закрытия");
    assert!(fm.0.is_empty());
    assert!(body.starts_with("---"));
}

#[test]
fn frontmatter_skips_nested_map_without_losing_next_key() {
    let (fm, _) = ai::parse_frontmatter("---\nmeta:\n  a: 1\n  b: 2\nname: z\n---\n");
    assert_eq!(fm.get("name"), Some(&Fv::Str("z".into())));
    assert!(fm.get("meta").is_some());
}

// ---------- JSON ----------

#[test]
fn json_accepts_bom_comments_and_trailing_commas() {
    let v = ai::parse_json("\u{feff}{\n // комментарий\n \"a\": [1, 2,], /* блок */ \"b\": \"//не комментарий\",\n}").expect("разбирается");
    assert_eq!(v["b"], "//не комментарий");
    assert_eq!(v["a"][1], 2);
}

#[test]
fn json_broken_gives_position_not_panic() {
    let e = ai::parse_json("{ \"a\": ").expect_err("не разбирается");
    assert!(e.contains("строка"), "{e}");
}

// ---------- шаблоны путей ----------

#[test]
fn glob_double_star_crosses_folders_and_single_does_not() {
    let g = Glob::new("apps/api/**").unwrap();
    assert!(g.is_match("apps/api/x/y.cs") && !g.is_match("apps/web/x.cs") && !g.is_match("apps/api"));
    let g = Glob::new("src/*.rs").unwrap();
    assert!(g.is_match("src/a.rs") && !g.is_match("src/sub/a.rs"));
}

#[test]
fn glob_without_slash_matches_any_depth_and_anchored_does_not() {
    assert!(Glob::new("*.ts").unwrap().is_match("a/b/c.ts"));
    assert!(!Glob::anchored("CLAUDE.md").unwrap().is_match("apps/web/claude.md"));
    assert!(Glob::anchored("CLAUDE.md").unwrap().is_match("claude.md"));
}

#[test]
fn glob_braces_and_zero_folder_double_star() {
    let g = Glob::new("**/*.{ts,tsx}").unwrap();
    assert!(g.is_match("a.ts") && g.is_match("x/y/a.tsx") && !g.is_match("a.rs"));
}

// ---------- краткое содержание ----------

#[test]
fn cut_breaks_at_sentence_and_marks_ellipsis_only_when_cut() {
    assert_eq!(ai::cut("Коротко.", 50), "Коротко.");
    let t = "Первое предложение о том, что это такое. Второе предложение сильно длиннее и не влезет целиком сюда.";
    assert_eq!(ai::cut(t, 60), "Первое предложение о том, что это такое.");
    let w = ai::cut("слово слово слово слово слово слово слово слово", 20);
    assert!(w.ends_with('…') && w.chars().count() <= 21, "{w}");
}

#[test]
fn first_paragraph_skips_heading_comment_fence_and_table() {
    let md = "# Заголовок\n\n<!-- скрыто\nещё -->\n```\ncode\n```\n| a | b |\n|---|---|\n\nПервый **осмысленный** абзац,\nна две строки.\n\nВторой.";
    assert_eq!(ai::first_paragraph(md), "Первый осмысленный абзац, на две строки.");
}

#[test]
fn script_header_reads_shell_and_js_comment_blocks() {
    assert_eq!(
        ai::script_header("#!/usr/bin/env bash\n#\n# Проверка автора.\n# Вторая строка.\n\nset -e\n"),
        "Проверка автора. Вторая строка."
    );
    assert_eq!(
        ai::script_header("#!/usr/bin/env node\n// Хук после правки.\n// Запускает типы.\n\nimport x from 'y';"),
        "Хук после правки. Запускает типы."
    );
}

// ---------- проект целиком ----------

const SETTINGS: &str = "\u{feff}{\n  // комментарий в настройках\n  \"permissions\": { \"allow\": [\"Bash(npm test)\"], \"deny\": [\"Bash(rm -rf:*)\"], },\n  \"env\": { \"API_TOKEN\": \"supersecretvalue1234567890\", \"MODE\": \"dev\" },\n  \"hooks\": { \"PostToolUse\": [ { \"matcher\": \"Edit|Write\", \"hooks\": [ { \"type\": \"command\", \"command\": \"node \\\"$CLAUDE_PROJECT_DIR/.claude/hooks/h.mjs\\\" --token abcdefghij\", \"timeout\": 30 } ] } ] }\n}\n";

fn fixture() -> TempDir {
    let d = TempDir::new("ai");
    d.write("CLAUDE.md", "# Проект\n\nГлавный файл. Правила - `.claude/rules/a.md`, агент - `reviewer`, хук - `.claude/hooks/h.mjs`.\n");
    d.write("AGENTS.md", "Для других инструментов.\n");
    d.write(
        ".claude/rules/a.md",
        "---\ndescription: Правило про Rust\npaths:\n  - \"src/**/*.rs\"\n  - \"nothing/**\"\n---\n# A\nТекст.\n",
    );
    d.write(".claude/rules/b.md", "# B\n\nПравило без шапки, читается всегда.\n");
    d.write(".claude/rules/c.md", "---\ndescription: >\n  Многострочное\n  описание.\npaths: [docs/**]\n---\n");
    d.write(".claude/agents/reviewer.md", "---\nname: reviewer\ndescription: Смотрит дифф (до #12) враждебно.\ntools: Read, Grep\nmodel: opus\n---\nСмотрит `.claude/rules/b.md`.\n");
    d.write(".claude/skills/deploy/SKILL.md", "---\ndescription: Как выкладывать\n---\n# Deploy\n");
    d.write(".claude/settings.json", SETTINGS);
    d.write(".claude/hooks/h.mjs", "#!/usr/bin/env node\n// Хук: проверка типов после правки.\n");
    d.write(".claude/hooks/lonely.mjs", "// Лежит без подключения.\n");
    d.write(".mcp.json", "{ \"mcpServers\": { \"db\": { \"command\": \"npx\", \"args\": [\"db-mcp\", \"--api-key\", \"zzzzzzzzzzzzzzzzzzzz\", \"--url=https://user:pass@host/db\"], \"env\": { \"DB_PASSWORD\": \"hunter2\", \"REGION\": \"eu\" } } } }");
    d.write(".githooks/pre-commit", "#!/usr/bin/env bash\n# Автор коммита.\n. \"$(dirname \"$0\")/helper.sh\"\n");
    d.write(".githooks/helper.sh", "# Общий помощник.\n");
    d.write("src/a.rs", "fn main() {}\n");
    d.write("src/b.rs", "\n");
    d.write("docs/x.md", "x\n");
    d.write(".env", "SECRET_FROM_ENV_FILE=do-not-read-me\n");
    d
}

#[test]
fn kinds_are_found_with_modes_and_summaries() {
    let d = fixture();
    let m = build(&d);
    let kinds: std::collections::BTreeMap<_, _> = m.kinds.iter().map(|k| (k.id.as_str(), k.count)).collect();
    assert_eq!(
        (kinds["instructions"], kinds["rules"], kinds["agents"], kinds["skills"], kinds["mcp"], kinds["githooks"]),
        (2, 3, 1, 1, 1, 2)
    );
    assert_eq!(item(&m, "rules:.claude/rules/a.md").mode, "paths");
    assert_eq!(item(&m, "rules:.claude/rules/b.md").mode, "always");
    assert_eq!(item(&m, "rules:.claude/rules/c.md").summary, "Многострочное описание.");
    assert_eq!(item(&m, "rules:.claude/rules/b.md").summary, "Правило без шапки, читается всегда.");
    let a = item(&m, "agents:.claude/agents/reviewer.md");
    assert_eq!((a.model.as_deref(), a.tools.clone()), (Some("opus"), vec!["Read".to_string(), "Grep".to_string()]));
    assert_eq!(a.summary, "Смотрит дифф (до #12) враждебно.");
    assert_eq!(item(&m, "skills:.claude/skills/deploy/SKILL.md").name, "deploy");
}

#[test]
fn rule_globs_count_real_files_and_zero_is_visible() {
    let d = fixture();
    let m = build(&d);
    let a = item(&m, "rules:.claude/rules/a.md");
    assert_eq!(a.paths.iter().map(|p| (p.glob.as_str(), p.files)).collect::<Vec<_>>(), vec![("src/**/*.rs", 2), ("nothing/**", 0)]);
    assert_eq!(a.files_matched, 2);
    assert_eq!(item(&m, "rules:.claude/rules/c.md").files_matched, 1);
}

#[test]
fn hook_points_to_script_and_unwired_script_is_marked() {
    let d = fixture();
    let m = build(&d);
    let h = item(&m, "hooks:.claude/settings.json#PostToolUse/0/0");
    assert_eq!((h.event.as_deref(), h.matcher.as_deref(), h.timeout), (Some("PostToolUse"), Some("Edit|Write"), Some(30)));
    assert_eq!(h.script.as_deref(), Some(".claude/hooks/h.mjs"));
    assert_eq!(h.summary, "Хук: проверка типов после правки.");
    assert!(item(&m, "hooks:.claude/hooks/lonely.mjs").unwired);
    assert!(m.items.iter().all(|i| i.id != "hooks:.claude/hooks/h.mjs"), "подключённый скрипт не дублируется отдельной деталью");
}

#[test]
fn links_come_only_from_text_and_not_from_guesses() {
    let d = fixture();
    let m = build(&d);
    let c = item(&m, "instructions:CLAUDE.md");
    for t in ["rules:.claude/rules/a.md", "agents:.claude/agents/reviewer.md"] {
        assert!(c.refs_out.iter().any(|x| x == t), "CLAUDE.md не ссылается на {t}: {:?}", c.refs_out);
    }
    assert!(!c.refs_out.iter().any(|x| x == "rules:.claude/rules/c.md"), "правило c нигде не названо");
    assert!(item(&m, "rules:.claude/rules/b.md").refs_in.contains(&"agents:.claude/agents/reviewer.md".to_string()));
    // githook -> общий файл: путь из текста скрипта
    assert!(item(&m, "githooks:.githooks/pre-commit").refs_out.contains(&"githooks:.githooks/helper.sh".to_string()));
    assert!(m.kind_edges.iter().any(|e| e.from == "instructions" && e.to == "rules"));
}

#[test]
fn secrets_are_masked_in_env_args_and_command() {
    let d = fixture();
    let m = build(&d);
    let all = serde_json::to_string(&m).unwrap();
    for leak in [
        "supersecretvalue1234567890",
        "hunter2",
        "zzzzzzzzzzzzzzzzzzzz",
        "user:pass",
        "abcdefghij",
        "do-not-read-me",
        "SECRET_FROM_ENV_FILE",
    ] {
        assert!(!all.contains(leak), "в карту попало «{leak}»");
    }
    let s = item(&m, "settings:.claude/settings.json");
    let t = s.env.iter().find(|e| e.key == "API_TOKEN").expect("имя ключа видно");
    assert!(t.masked && t.value.is_empty());
    let mode = s.env.iter().find(|e| e.key == "MODE").unwrap();
    assert!(!mode.masked && mode.value == "dev", "безобидное значение показывается");
    let db = item(&m, "mcp:.mcp.json#db");
    assert!(db.command.as_deref().unwrap().contains("db-mcp"));
    assert!(db.env.iter().any(|e| e.key == "DB_PASSWORD" && e.masked));
}

#[test]
fn broken_json_is_reported_and_does_not_stop_the_rest() {
    let d = fixture();
    d.write(".cursor/mcp.json", "{ \"mcpServers\": { ");
    let m = build(&d);
    assert!(m.problems.iter().any(|p| p.file == ".cursor/mcp.json"), "{:?}", m.problems);
    assert!(item(&m, "mcp:.cursor/mcp.json").broken.is_some());
    assert_eq!(item(&m, "rules:.claude/rules/b.md").mode, "always");
}

#[test]
fn private_settings_show_only_numbers() {
    let d = fixture();
    d.write(
        ".claude/settings.local.json",
        "{ \"permissions\": { \"allow\": [\"Bash(curl -H 'Authorization: Bearer TOPSECRETTOKEN' x)\"] }, \"env\": { \"K\": \"v\" } }",
    );
    let m = build(&d);
    let all = serde_json::to_string(&m).unwrap();
    assert!(!all.contains("TOPSECRETTOKEN") && !all.contains("Bearer"));
    let l = item(&m, "settings:.claude/settings.local.json");
    assert!(l.private && l.env.is_empty() && l.facts.iter().any(|f| f.k == "allow" && f.v == "1"));
}

#[test]
fn githooks_split_hooks_from_helpers_and_report_hookspath() {
    let d = fixture();
    d.write(".git/config", "[core]\n\thooksPath = .githooks\n");
    let m = build(&d);
    assert_eq!(item(&m, "githooks:.githooks/pre-commit").mode, "git");
    assert_eq!(item(&m, "githooks:.githooks/helper.sh").mode, "helper");
    assert_eq!(item(&m, "githooks:.githooks/pre-commit").active, Some(true));
    assert_eq!(m.hooks_path.as_deref(), Some(".githooks"));
}

#[test]
fn worktrees_folder_is_a_signal_and_its_copies_are_not_walked() {
    let d = fixture();
    d.write(".claude/worktrees/wt1/CLAUDE.md", "копия\n");
    d.write(".claude/worktrees/wt1/.claude/rules/z.md", "копия правила\n");
    let m = build(&d);
    assert_eq!(item(&m, "orchestration:worktrees").facts.iter().find(|f| f.k == "dirs").map(|f| f.v.as_str()), Some("1"));
    assert!(m.items.iter().all(|i| !i.file.contains("worktrees/wt1")), "копии проекта не считаются деталями");
}

#[test]
fn symlinked_folder_is_not_followed() {
    // ссылка стоит ровно там, где лежит папка настроек: пойди по ней разбор - правило чужой папки стало бы деталью
    let d = TempDir::new("ai-link");
    d.write(
        "CLAUDE.md",
        "# P

Единственный файл.
",
    );
    let outside = TempDir::new("ai-outside");
    outside.write(
        "rules/leak.md",
        "---
description: чужое
---
",
    );
    let link = d.path().join(".claude");
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(outside.path(), &link).is_ok();
    #[cfg(not(windows))]
    let made = std::os::unix::fs::symlink(outside.path(), &link).is_ok();
    assert!(made, "ссылку создать не удалось: проверка без неё ничего не доказывает");
    let m = build(&d);
    assert!(m.items.iter().all(|i| i.name != "leak"), "по ссылке не ходим: {:?}", m.items.iter().map(|i| &i.id).collect::<Vec<_>>());
    assert_eq!(m.items.len(), 1);
}

#[test]
fn story_has_only_steps_with_matching_parts() {
    let d = TempDir::new("ai-min");
    d.write("CLAUDE.md", "# P\n\nЕдинственный файл.\n");
    let m = build(&d);
    let ids: Vec<&str> = m.story.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec!["read", "code"], "без хуков, агентов и проверок остальных шагов нет");
}

#[test]
fn empty_folder_has_no_ai_and_markers_are_empty() {
    let d = TempDir::new("ai-none");
    d.write("README.md", "x\n");
    let m = build(&d);
    assert!(m.items.is_empty() && m.story.is_empty());
}

#[test]
fn watcher_tells_ai_edits_from_other_files() {
    let kb = common::kb();
    let root = std::path::Path::new("C:/proj");
    let k = |rel: &str| pult_core::scan::watch_kind(root, &root.join(rel), &kb);
    assert_eq!(k(".claude/rules/a.md"), 3, "правка правила пересобирает только карту ИИ-части");
    assert_eq!(k(".claude"), 1, "появилась папка .claude - перечитать проект");
    assert_eq!(k("CLAUDE.md"), 1);
    assert_eq!(k(".claude/worktrees/x/.claude/rules/a.md"), 0, "чужие рабочие копии не слушаем");
    assert_eq!(k("src/main.rs"), 0);
}
