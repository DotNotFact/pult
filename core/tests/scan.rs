//! Разбор настоящей папки проекта, собранной во временном каталоге.
mod common;

use common::TempDir;
use pult_core::scan::{scan, ScanOpts};
use pult_core::{Project, Script};

const PACKAGE_JSON: &str = r#"{
  "name": "demo-site",
  "version": "1.2.3",
  "description": "Проверочный проект",
  "keywords": ["build", "test"],
  "scripts": {
    "dev": "next dev",
    "build": "next build",
    "prebuild": "rimraf dist",
    "deploy": "vercel --prod",
    "test": "vitest run"
  },
  "dependencies": { "next": "15.0.0" },
  "devDependencies": { "vitest": "2.0.0", "rimraf": "6.0.0" }
}
"#;

const MAKEFILE: &str = ".PHONY: bundle tidy\n\nbundle: ## Собрать\n\tgo build ./...\n\n# Убрать лишнее\ntidy:\n\t@go mod tidy\n";

const CLEAN_PS1: &str = "<#\n.SYNOPSIS\nУдаляет временные файлы сборки.\n.DESCRIPTION\nЧистит dist и кэш.\n#>\nparam([switch]$DryRun)\nRemove-Item dist -Recurse\n";

const COMPOSE: &str = "services:\n  web:\n    image: nginx\n    ports:\n      - \"8080:80\"\n  db:\n    image: postgres:17\n";

fn demo() -> TempDir {
    let d = TempDir::new("scan");
    d.write("package.json", PACKAGE_JSON);
    d.write("Makefile", MAKEFILE);
    d.write("scripts/clean.ps1", CLEAN_PS1);
    d.write("docker-compose.yml", COMPOSE);
    d
}

fn scan_demo(d: &TempDir) -> Project {
    common::kb();
    // git не должен найти репозиторий выше временной папки (например, если TEMP лежит внутри чьего-то репозитория)
    std::env::set_var("GIT_CEILING_DIRECTORIES", std::env::temp_dir());
    scan(d.path(), &ScanOpts::default()).expect("папка разбирается")
}

fn script<'a>(p: &'a Project, source: &str, name: &str) -> &'a Script {
    p.scripts.iter().find(|s| s.source == source && s.name == name).unwrap_or_else(|| {
        let all: Vec<String> = p.scripts.iter().map(|s| format!("{} / {}", s.source, s.name)).collect();
        panic!("нет скрипта {source} / {name}; есть: {all:#?}")
    })
}

#[test]
fn package_json_scripts_and_stages() {
    let d = demo();
    let p = scan_demo(&d);
    assert_eq!(p.name, "demo-site");
    assert_eq!(p.version, "1.2.3");
    assert_eq!(p.description, "Проверочный проект");
    for (name, stage) in [("dev", "dev"), ("build", "build"), ("test", "check"), ("deploy", "release")] {
        assert_eq!(script(&p, "package.json", name).stage, stage, "этап «{name}»");
    }
    let dev = script(&p, "package.json", "dev");
    assert_eq!(dev.argv, ["npm", "run", "dev"], "без lock-файла менеджер пакетов - npm");
    assert!(dev.info.long, "next dev работает, пока не остановят");
    assert!(!script(&p, "package.json", "build").info.long);
    // скрипты package.json идут первыми и в порядке файла
    let first: Vec<&str> = p.scripts.iter().take(5).map(|s| s.name.as_str()).collect();
    assert_eq!(first, ["dev", "build", "prebuild", "deploy", "test"]);
    // строка - ключ в scripts, а не «keywords» выше, где те же слова
    assert_eq!(script(&p, "package.json", "dev").line, Some(7));
    assert_eq!(script(&p, "package.json", "build").line, Some(8));
    assert_eq!(script(&p, "package.json", "test").line, Some(11));
    assert!(p.scripts.iter().all(|s| s.raw.is_empty()), "текст команды нужен только для объяснения и не уходит наружу");
}

#[test]
fn prebuild_hook_and_deploy_danger() {
    let d = demo();
    let p = scan_demo(&d);
    let pre = script(&p, "package.json", "prebuild");
    assert_eq!(pre.info.hook.as_deref(), Some("перед «build»"));
    assert!(pre.info.fx.iter().any(|f| f == "delete"), "rimraf удаляет: {:?}", pre.info.fx);
    assert!(script(&p, "package.json", "deploy").danger);
    assert!(!script(&p, "package.json", "build").danger);
    assert!(script(&p, "package.json", "build").info.hook.is_none());
}

#[test]
fn makefile_summary_from_comment() {
    let d = demo();
    let p = scan_demo(&d);
    let b = script(&p, "Makefile", "bundle");
    assert_eq!(b.desc, "Собрать");
    assert_eq!(b.info.summary, "Собрать", "комментарий автора главнее разбора рецепта");
    assert_eq!(b.argv, ["make", "bundle"]);
    assert_eq!(b.line, Some(3));
    let t = script(&p, "Makefile", "tidy");
    assert_eq!(t.desc, "Убрать лишнее", "комментарий строкой выше цели");
    assert!(!p.scripts.iter().any(|s| s.name == ".PHONY"), "служебные цели не скрипты");
    assert!(p.stacks.iter().any(|s| s == "Make"));
}

#[test]
fn powershell_script_header_and_flags() {
    let d = demo();
    let p = scan_demo(&d);
    let c = script(&p, "scripts/clean.ps1", "clean.ps1");
    assert_eq!(c.file_rel.as_deref(), Some("scripts/clean.ps1"));
    assert_eq!(c.group, "файлы: scripts");
    assert_eq!(c.stage, "ops", "clean - сопровождение");
    assert!(c.info.summary.starts_with("Удаляет временные файлы сборки"), "summary: {}", c.info.summary);
    assert!(c.info.header.contains("Чистит dist"), "header: {}", c.info.header);
    assert!(c.info.flags.iter().any(|f| f.flag == "-DryRun" && f.from == "file"), "flags: {:?}", c.info.flags);
    assert_eq!(c.argv.first().map(String::as_str), Some("powershell"));
    assert!(c.argv.last().is_some_and(|a| a.ends_with("clean.ps1")));
}

#[test]
fn compose_services_and_topics() {
    let d = demo();
    let p = scan_demo(&d);
    let up = script(&p, "docker-compose.yml", "compose up -d");
    assert_eq!(up.desc, "Сервисы: web, db");
    assert!(up.virtual_, "стандартная команда по манифесту");
    assert_eq!(up.stage, "dev");
    assert!(p.health.docker);
    assert!(p.stacks.iter().any(|s| s == "Docker"), "stacks: {:?}", p.stacks);
    let topics: Vec<&str> = p.topics.iter().map(|t| t.id.as_str()).collect();
    for t in ["docker", "docker-compose", "node"] {
        assert!(topics.contains(&t), "урок {t}; есть: {topics:?}");
    }
    assert!(p.deps.iter().any(|d| d == "next"));
}

#[test]
fn folder_without_git() {
    let d = demo();
    let p = scan_demo(&d);
    assert!(p.git.is_none(), "временная папка - не репозиторий");
    assert!(!p.scripts.iter().any(|s| s.source == "git"), "без репозитория нет команд git");
    assert!(p.ci.is_empty());
    let ids: std::collections::BTreeSet<&str> = p.scripts.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids.len(), p.scripts.len(), "id скриптов уникальны");
}

#[test]
fn health_reports_missing_node_modules() {
    let d = demo();
    let p = scan_demo(&d);
    assert_eq!(p.health.deps.as_deref(), Some("missing"));
    assert!(!p.health.env);
}

#[test]
fn scan_rejects_file_and_missing_folder() {
    let d = demo();
    common::kb();
    assert!(scan(&d.path().join("package.json"), &ScanOpts::default()).is_err());
    assert!(scan(&d.path().join("нет-такой-папки"), &ScanOpts::default()).is_err());
    assert!(scan(std::path::Path::new(r"\\server\share"), &ScanOpts::default()).is_err(), "сетевые пути не открываем");
}

#[test]
fn folder_with_git_gets_git_info_and_commands() {
    if pult_core::runner::which("git").is_none() {
        eprintln!("git не установлен - проверка пропущена");
        return;
    }
    let d = demo();
    let argv: Vec<String> = ["git", "init", "-q"].iter().map(|s| s.to_string()).collect();
    let c = pult_core::runner::capture(&argv, d.path(), std::time::Duration::from_secs(20)).expect("git init");
    assert_eq!(c.code, 0, "git init: {}", c.err);
    let p = scan_demo(&d);
    let g = p.git.as_ref().expect("папка - репозиторий");
    assert_eq!(g.prefix, "", "проект в корне репозитория");
    assert!(g.dirty >= 4, "все файлы проекта новые: {}", g.dirty);
    assert!(g.files.iter().any(|f| f.path == "package.json" && f.code == "?"));
    assert!(p.scripts.iter().any(|s| s.source == "git" && s.virtual_ && !s.danger), "команды git - стандартные, не скрипты автора");
}

#[test]
fn cargo_workspace_gives_version_and_description() {
    let d = TempDir::new("cargo");
    d.write(
        "Cargo.toml",
        "[workspace]\nmembers = [\"core\"]\n\n[workspace.package]\nversion = \"4.5.6\"\ndescription = \"Описание из Cargo\"\n\n[workspace.dependencies]\nserde = { version = \"1\" }\n",
    );
    let p = scan_demo(&d);
    assert_eq!(p.version, "4.5.6");
    assert_eq!(p.description, "Описание из Cargo");
}

fn topic_ids(p: &Project) -> Vec<&str> {
    p.topics.iter().map(|t| t.id.as_str()).collect()
}

#[test]
fn gradle_wrapper_is_found_by_path_and_explained() {
    let d = TempDir::new("gradlew");
    d.write("build.gradle.kts", "plugins { java }\n");
    d.write("gradlew.bat", "@echo off\r\n");
    let p = scan_demo(&d);
    let b = script(&p, "build.gradle.kts", "gradle build");
    let wrapper = d.path().join("gradlew.bat").to_string_lossy().into_owned();
    // программа - сам файл обёртки: «нет программы gradlew» и выключенный запуск были ошибкой
    assert_eq!(b.tool.as_deref(), Some(wrapper.as_str()));
    assert!(pult_core::runner::which(b.tool.as_deref().unwrap_or_default()).is_some());
    assert!(b.info.known, "{}", b.info.summary);
    assert_eq!(b.info.tools, vec!["gradle".to_string()]);
    assert!(!b.info.summary.contains("Выполняет"), "{}", b.info.summary);
}

#[test]
fn package_init_py_is_not_a_script() {
    let d = TempDir::new("initpy");
    d.write("scripts/__init__.py", "");
    d.write("scripts/collect.py", "\"\"\"Собирает данные.\"\"\"\nprint(1)\n");
    let p = scan_demo(&d);
    let names: Vec<&str> = p.scripts.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"collect.py"), "{names:?}");
    assert!(!names.contains(&"__init__.py"), "служебный файл пакета не запускают: {names:?}");
}

#[test]
fn file_script_reading_by_default_is_not_a_check() {
    let d = TempDir::new("createadmin");
    d.write("package.json", r#"{ "scripts": { "create-admin": "node scripts/create-admin.js" } }"#);
    d.write("scripts/create-admin.js", "// Скрипт создания администратора\nconsole.log(1)\n");
    let p = scan_demo(&d);
    let s = script(&p, "package.json", "create-admin");
    assert_eq!(s.info.summary, "Скрипт создания администратора");
    assert_ne!(s.stage, "check", "«только читает» у node - умолчание, а не проверка");
}

#[test]
fn readme_only_folder_has_no_shell_lesson() {
    let d = TempDir::new("readme");
    d.write("README.md", "# Заметки\n\nПросто текст.\n");
    let p = scan_demo(&d);
    let t = topic_ids(&p);
    assert!(t.contains(&"terminal"), "{t:?}");
    assert!(!t.contains(&"shell-scripts"), "скриптов нет - урок о них не нужен: {t:?}");
}

#[test]
fn shell_lesson_by_script_file_and_csharp_by_loose_sources() {
    let d = TempDir::new("loose");
    d.write("run.ps1", "Write-Host 1\n");
    d.write("Program.cs", "class P {}\n");
    let p = scan_demo(&d);
    let t = topic_ids(&p);
    assert!(t.contains(&"shell-scripts"), "{t:?}");
    assert!(t.contains(&"csharp"), "исходники C# без проекта - тоже C#: {t:?}");
}

#[test]
fn empty_folder_scans() {
    let d = TempDir::new("empty");
    let p = scan_demo(&d);
    assert!(p.scripts.is_empty());
    assert_eq!(topic_ids(&p), ["terminal"]);
}

#[test]
fn readme_description_counts_letters_not_bytes() {
    // абзац по-русски в 300 с лишним букв (600+ байт) приходит целиком, а не обрывается на середине
    let line = "Сверяет норму расхода топлива с фактическим объёмом заправки и шлёт владельцу сообщение,";
    let d = TempDir::new("readme-ru");
    d.write("README.md", &format!("# Проект\n\n{line}\n{line}\n{line}\nесли цифры не сходятся.\n\n## Дальше\n"));
    let p = scan_demo(&d);
    assert!(p.description.ends_with("если цифры не сходятся."), "{}", p.description);
}

#[test]
fn bare_cli_passthrough_is_not_a_check() {
    // «"ng": "ng"» - просто вход в Angular CLI: что он сделает, решат аргументы, это не проверка
    let d = TempDir::new("ng");
    d.write("package.json", r#"{ "scripts": { "ng": "ng", "lint": "eslint ." } }"#);
    let p = scan_demo(&d);
    assert_ne!(script(&p, "package.json", "ng").stage, "check");
    assert_eq!(script(&p, "package.json", "lint").stage, "check");
}

#[test]
fn pyproject_deps_with_extras_groups_and_poetry() {
    let d = TempDir::new("pyproject");
    d.write(
        "pyproject.toml",
        "[project]\nname = \"bot\"\ndependencies = [\n    \"uvicorn[standard]>=0.30\",  # сервер\n    \"fastapi>=0.115\",\n    'httpx',\n]\n\n[project.optional-dependencies]\ndev = [\"pytest>=8\", \"mypy\"]\n\n[tool.poetry.group.web.dependencies]\npython = \"^3.12\"\ndjango = \"^5\"\n\n[tool.mypy]\nstrict = true\n",
    );
    let p = scan_demo(&d);
    for dep in ["uvicorn", "fastapi", "httpx", "pytest", "mypy", "django"] {
        assert!(p.deps.iter().any(|x| x == dep), "нет {dep}: {:?}", p.deps);
    }
    assert!(!p.deps.iter().any(|x| x == "python" || x == "strict"), "{:?}", p.deps);
    assert!(p.stacks.iter().any(|s| s == "FastAPI"), "fastapi после «uvicorn[standard]» не теряется: {:?}", p.stacks);
}

#[test]
fn cargo_dependency_tables_give_crate_name_not_keys() {
    let d = TempDir::new("cargo-deps");
    d.write(
        "Cargo.toml",
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nregex = \"1\"\n\n[dependencies.serde]\nversion = \"1\"\nfeatures = [\"derive\"]\n\n[target.'cfg(windows)'.dependencies]\nwindows-sys = \"0.59\"\n",
    );
    let p = scan_demo(&d);
    for dep in ["regex", "serde", "windows-sys"] {
        assert!(p.deps.iter().any(|x| x == dep), "нет {dep}: {:?}", p.deps);
    }
    assert!(!p.deps.iter().any(|x| x == "version" || x == "features"), "ключи таблицы - не зависимости: {:?}", p.deps);
}

#[test]
fn dotnet_packages_any_attribute_order_and_central_props() {
    let d = TempDir::new("csproj");
    d.write("App.csproj", "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <ItemGroup>\n    <PackageReference Version=\"8.0.0\" Include=\"Microsoft.EntityFrameworkCore\" />\n    <PackageReference Include=\"Serilog\" />\n  </ItemGroup>\n</Project>\n");
    d.write(
        "Directory.Packages.props",
        "<Project>\n  <ItemGroup>\n    <PackageVersion Include=\"MassTransit\" Version=\"8.0.0\" />\n  </ItemGroup>\n</Project>\n",
    );
    let p = scan_demo(&d);
    for dep in ["microsoft.entityframeworkcore", "serilog", "masstransit"] {
        assert!(p.deps.iter().any(|x| x == dep), "нет {dep}: {:?}", p.deps);
    }
    assert!(topic_ids(&p).contains(&"databases"), "EF Core - урок о базах: {:?}", topic_ids(&p));
}

#[test]
fn dotnet_solution_members_keep_only_run_and_libraries_do_not_run() {
    let d = TempDir::new("sln");
    d.write("Shop.sln", "Microsoft Visual Studio Solution File\n");
    d.write("Shop.Api/Shop.Api.csproj", "<Project Sdk=\"Microsoft.NET.Sdk.Web\"></Project>\n");
    d.write(
        "Shop.Data/Shop.Data.csproj",
        "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>\n",
    );
    let p = scan_demo(&d);
    let names = |w: &str| -> Vec<String> { p.scripts.iter().filter(|s| s.where_ == w).map(|s| s.name.clone()).collect() };
    assert!(names(".").iter().any(|n| n == "dotnet build"), "решение собирается из корня: {:?}", names("."));
    assert_eq!(names("Shop.Api"), ["dotnet run", "dotnet watch"], "у проекта решения - только запуск");
    assert!(names("Shop.Data").is_empty(), "библиотеку в решении не запускают и не собирают отдельно: {:?}", names("Shop.Data"));

    // проект без решения: свои сборка и тесты есть, а запуска у библиотеки нет
    let d = TempDir::new("lib");
    d.write("Lib.csproj", "<Project Sdk=\"Microsoft.NET.Sdk\"></Project>\n");
    let p = scan_demo(&d);
    let all: Vec<&str> = p.scripts.iter().map(|s| s.name.as_str()).collect();
    assert!(all.contains(&"dotnet build") && all.contains(&"dotnet test"), "{all:?}");
    assert!(!all.contains(&"dotnet run"), "{all:?}");
    // консольная программа - запускается
    let d = TempDir::new("exe");
    d.write("Tool.csproj", "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType></PropertyGroup></Project>\n");
    let p = scan_demo(&d);
    assert!(p.scripts.iter().any(|s| s.name == "dotnet run"));
}

#[test]
fn dotnet_project_two_levels_below_solution_is_still_a_member() {
    let d = TempDir::new("sln-deep");
    d.write("Shop.sln", "Microsoft Visual Studio Solution File\n");
    d.write("src/Shop.Api/Shop.Api.csproj", "<Project Sdk=\"Microsoft.NET.Sdk.Web\"></Project>\n");
    let p = scan_demo(&d);
    let names: Vec<&str> = p.scripts.iter().filter(|s| s.where_ == "src/Shop.Api").map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["dotnet run", "dotnet watch"], "решение двумя папками выше - проект всё равно в нём");
}

#[test]
fn dotnet_solution_above_project_root_does_not_count() {
    // решение лежит выше открытой папки: Пульт его не видит, у проекта свои сборка и тесты
    let d = TempDir::new("sln-outside");
    d.write("Shop.sln", "Microsoft Visual Studio Solution File\n");
    d.write("Shop.Api/Shop.Api.csproj", "<Project Sdk=\"Microsoft.NET.Sdk.Web\"></Project>\n");
    common::kb();
    std::env::set_var("GIT_CEILING_DIRECTORIES", std::env::temp_dir());
    let p = scan(&d.path().join("Shop.Api"), &ScanOpts::default()).expect("папка разбирается");
    let names: Vec<&str> = p.scripts.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"dotnet build"), "{names:?}");
}
