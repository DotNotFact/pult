//! Пульт в терминале: интерактивный экран и команды. Тот же движок и та же база знаний, что у окна.
use paint::Paint;
use pult_core::model::{Project, Script};
use pult_core::util::display_argv;
use pult_core::{ai, ci, env, kb, runner, scan, store};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs, Wrap};
use ratatui::{DefaultTerminal, Frame};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const ACCENT: Color = Color::Rgb(110, 145, 255);
const MUTE: Color = Color::Rgb(140, 138, 130);

fn fx_color(level: &str) -> Color {
    match level {
        "ok" => Color::Rgb(90, 200, 130),
        "warn" => Color::Rgb(225, 180, 70),
        "bad" => Color::Rgb(240, 110, 110),
        _ => Color::Rgb(110, 155, 255),
    }
}
fn status_color(s: &str) -> Color {
    match s {
        "success" | "ok" => fx_color("ok"),
        "failed" | "fail" => fx_color("bad"),
        "running" => fx_color("info"),
        "pending" | "manual" => fx_color("warn"),
        _ => MUTE,
    }
}
fn ru(s: &str) -> &'static str {
    match s {
        "success" => "успех",
        "ok" => "готово",
        "failed" | "fail" => "ошибка",
        "running" => "идёт",
        "pending" => "в очереди",
        "canceled" => "отменён",
        "stopped" => "остановлен",
        "skipped" => "пропущен",
        "manual" => "вручную",
        _ => "",
    }
}
fn dur(ms: u64) -> String {
    if ms < 10_000 {
        let s = ms as f64 / 1000.0;
        format!("{:.1} с", s).replace('.', ",")
    } else {
        // сначала округлить до целых секунд, потом делить на минуты: иначе 119,6 с - «1 м 60 с»
        let secs = ms.saturating_add(500) / 1000;
        if secs < 60 {
            format!("{secs} с")
        } else {
            format!("{} м {:02} с", secs / 60, secs % 60)
        }
    }
}
fn ago(ms: u64) -> String {
    let s = pult_core::util::now_ms().saturating_sub(ms) / 1000;
    match s {
        0..45 => "только что".into(),
        45..3600 => format!("{} мин назад", s / 60),
        3600..86400 => format!("{} ч назад", s / 3600),
        _ => format!("{} дн назад", s / 86400),
    }
}
fn parse_iso(s: &str) -> u64 {
    // «2026-10-13T19:51:21Z» -> мс; грубо, только для «сколько назад»
    let d: Vec<u64> = s.get(0..19).unwrap_or("").split(|c: char| !c.is_ascii_digit()).filter_map(|x| x.parse().ok()).collect();
    // дата приходит из вывода glab/gh: вне календаря - как нечитаемая, без переполнения ниже
    let sane = [(1, 9999), (1, 12), (1, 31), (0, 23), (0, 59), (0, 60)];
    if d.len() < 6 || d.iter().zip(sane).any(|(&v, (lo, hi))| v < lo || v > hi) {
        return 0;
    }
    let [y, m, day] = [d[0], d[1], d[2]].map(|v| v as i64);
    let a = (14 - m) / 12;
    let (yy, mm) = (y + 4800 - a, m + 12 * a - 3);
    let jdn = day + (153 * mm + 2) / 5 + 365 * yy + yy / 4 - yy / 100 + yy / 400 - 32045;
    // до 1970 (нулевое время Go «0001-01-01T00:00:00Z» у незаполненного поля) - тоже 0: прежде
    // отрицательное число дней становилось огромным u64 и переполняло умножение
    let Ok(days) = u64::try_from(jdn - 2_440_588) else { return 0 };
    (days * 86400 + d[3] * 3600 + d[4] * 60 + d[5]) * 1000
}

/// Не больше `n` знаков; обрезанное кончается «…», чтобы было видно, что текст длиннее.
fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn fail(msg: &str) -> ! {
    eprintln!("{} {msg}", "✗".red());
    std::process::exit(1)
}

// ---------- проект ----------
fn looks_like_project(d: &Path) -> bool {
    [
        "package.json",
        ".git",
        "Makefile",
        "Cargo.toml",
        "go.mod",
        "pyproject.toml",
        "composer.json",
        "pom.xml",
        "build.gradle",
        "docker-compose.yml",
        "compose.yaml",
        "justfile",
    ]
    .iter()
    .any(|m| d.join(m).exists())
        || pult_core::util::ls(d).iter().any(|e| e.file_name().to_string_lossy().to_lowercase().ends_with(".sln"))
}
fn canon(p: &str) -> PathBuf {
    let c = std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p));
    let s = c.to_string_lossy().to_string();
    PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s))
}
fn resolve_root(explicit: Option<&str>) -> PathBuf {
    if let Some(e) = explicit {
        let r = canon(e);
        if !r.is_dir() {
            fail(&format!("Папка не найдена: {e}"));
        }
        return r;
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    if looks_like_project(&cwd) {
        return canon(&cwd.to_string_lossy());
    }
    if let Some(l) = store::config().last.filter(|l| Path::new(l).is_dir()) {
        return PathBuf::from(l);
    }
    fail("Не понял, какой проект открыть. Перейдите в папку проекта или укажите путь: pult -C D:\\путь")
}
fn load(root: &Path, fresh: bool) -> Project {
    let s = root.to_string_lossy().to_string();
    if !fresh {
        if let Some(p) = store::cached_scan(&s) {
            return p;
        }
    }
    let p = scan::scan(root, &scan::ScanOpts { trusted_hosts: store::trusted_hosts() }).unwrap_or_else(|e| fail(&e));
    store::save_scan(&p);
    p
}
fn find<'a>(p: &'a Project, q: &str) -> Result<&'a Script, String> {
    let (where_, name) = match q.split_once(':') {
        Some((w, n)) if p.scripts.iter().any(|s| s.where_ == w) => (Some(w), n),
        _ => (None, q),
    };
    let exact: Vec<&Script> = p.scripts.iter().filter(|s| s.name == name && where_.is_none_or(|w| s.where_ == w)).collect();
    if let Some(s) = exact.iter().find(|s| s.where_ == ".").or(exact.first()) {
        return Ok(s);
    }
    let low = name.to_lowercase();
    let near: Vec<&Script> = p.scripts.iter().filter(|s| s.name.to_lowercase().contains(&low)).collect();
    match near.len() {
        1 => Ok(near[0]),
        0 => Err(format!("Скрипт «{q}» не найден. Список: pult ls")),
        _ => Err(format!("Подходит несколько - уточните: {}", near.iter().take(10).map(|s| label(s)).collect::<Vec<_>>().join(", "))),
    }
}
/// Имя скрипта, по которому его найдут explain и run: из подпапки - «папка:имя».
fn label(s: &Script) -> String {
    if s.where_ == "." {
        s.name.clone()
    } else {
        format!("{}:{}", s.where_, s.name)
    }
}
fn effects() -> Value {
    kb::get().effects.clone()
}
fn fx_line(s: &Script) -> Vec<Span<'static>> {
    let e = effects();
    let order = ["danger", "delete", "admin", "db", "install", "write", "network", "docker", "read"];
    let mut fx = s.info.fx.clone();
    fx.sort_by_key(|f| order.iter().position(|o| o == f).unwrap_or(99));
    let mut out: Vec<Span> = Vec::new();
    for f in fx {
        out.push(Span::styled(
            format!("{} ", e[&f]["label"].as_str().unwrap_or(&f)),
            Style::default().fg(fx_color(e[&f]["level"].as_str().unwrap_or("info"))),
        ));
    }
    if s.info.long {
        out.push(Span::styled("долгий ", Style::default().fg(MUTE)));
    }
    out
}

// ---------- команды ----------
fn help() {
    println!(
        "{} {} - управление проектом из терминала\n\n\
  pult                       интерактивный экран (Tab - экраны, Enter - запуск)\n\
  pult info                  сводка: стек, git, состояние, этапы, CI, уроки\n\
  pult ls [поиск]            все скрипты с объяснениями (--stage check - один этап)\n\
  pult explain <скрипт>      что делает, команда по частям, что внутри, ключи\n\
  pult run <скрипт> [-- арг] запустить; вывод - прямо здесь. --yes - без вопроса для опасных\n\
  pult ci [номер]            пайплайны; с номером - этапы и джобы; pult ci log <джоба>\n\
  pult learn [урок]          уроки по технологиям проекта, прямо в терминале\n\
  pult ai [--json]           ИИ-часть проекта: правила, агенты, хуки, MCP, проверки и как они связаны\n\
  pult ui [путь]             открыть окно Пульта\n\n\
  Проект: текущая папка, если это проект, иначе последний открытый. Другой - ключом -C <путь>.\n\
  Скрипт из подпапки: «apps/web:build». Имя можно сокращать: «pult run typ» найдёт typecheck.",
        "▶".blue(),
        "Пульт".bold()
    );
}

fn cmd_info(p: &Project) {
    println!(
        "\n{} {}  {}",
        p.name.as_str().bold(),
        if p.version.is_empty() { String::new() } else { format!("v{}", p.version) },
        p.path.as_str().dark_grey()
    );
    if !p.description.is_empty() {
        println!("{}", clip(&p.description, 300).dark_grey());
    }
    println!("{}\n", p.stacks.join(" · ").blue());
    if let Some(g) = &p.git {
        println!(
            "{} {} → {}   {}   ↑{} ↓{}{}",
            "Git".bold(),
            g.branch,
            g.upstream.clone().unwrap_or_else(|| "не связана с сервером".into()),
            if g.dirty > 0 { format!("{} не в коммите", g.dirty).yellow().to_string() } else { "чисто".green().to_string() },
            g.ahead,
            g.behind,
            if g.stash > 0 { format!("   отложено: {}", g.stash) } else { String::new() }
        );
    }
    let names: Vec<&str> = p.scripts.iter().filter_map(|s| s.tool.as_deref()).collect();
    let tools = env::tools(names);
    let miss: Vec<&String> = tools.iter().filter(|(_, v)| v.is_none()).map(|(k, _)| k).collect();
    println!(
        "{} {}",
        "Программы".bold(),
        if miss.is_empty() {
            "все на месте".green().to_string()
        } else {
            format!("нет: {}", miss.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")).red().to_string()
        }
    );
    let mut by: BTreeMap<String, usize> = BTreeMap::new();
    for s in &p.scripts {
        *by.entry(s.stage.clone()).or_default() += 1;
    }
    let max = by.values().copied().max().unwrap_or(1);
    println!("\n{} ({} скриптов)", "Путь разработки".bold(), p.scripts.len());
    for st in &kb::get().stages {
        if let Some(n) = by.get(&st.id) {
            println!("  {:<14} {} {n}", st.label, "█".repeat((n * 30 / max).max(1)).blue());
        }
    }
    for c in &p.ci {
        let title = if c.kind == "gitlab" {
            "GitLab CI"
        } else if c.name.is_empty() {
            "GitHub Actions"
        } else {
            c.name.as_str()
        };
        println!("\n{} {}", title.bold(), c.file.as_str().dark_grey());
        // у GitLab - этапы с числом джоб; у GitHub Actions этапов нет - сразу джобы
        let line = if c.kind == "gitlab" {
            c.stages.iter().map(|s| format!("{} ({})", s.name, s.jobs.len())).collect::<Vec<_>>().join(" → ")
        } else {
            c.jobs.iter().map(|j| j.name.clone()).collect::<Vec<_>>().join(", ")
        };
        if !line.is_empty() {
            println!("  {line}");
        }
    }
    if !p.topics.is_empty() {
        println!("\n{} {}", "Уроки".bold(), p.topics.iter().map(|t| t.title.clone()).collect::<Vec<_>>().join(", "));
        println!("{}", "  читать: pult learn <урок>".dark_grey());
    }
    println!();
}

fn cmd_ls(p: &Project, q: &str, stage: Option<&str>) {
    let low = q.to_lowercase();
    let list: Vec<&Script> = p
        .scripts
        .iter()
        .filter(|s| {
            stage.is_none_or(|x| s.stage == x)
                && (low.is_empty() || format!("{} {} {} {}", s.name, s.info.summary, s.cmd, s.where_).to_lowercase().contains(&low))
        })
        .collect();
    for st in &kb::get().stages {
        let rows: Vec<&&Script> = list.iter().filter(|s| s.stage == st.id).collect();
        if rows.is_empty() {
            continue;
        }
        println!("\n{} {}", st.label.as_str().bold(), rows.len().to_string().dark_grey());
        for s in rows {
            let name = label(s);
            let fx: String = fx_line(s).iter().map(|x| x.content.to_string()).collect();
            // длинное имя не обрезается (по нему зовут explain и run), но от описания его отделяют два пробела
            println!("  {:<34}  {}  {}", name, clip(&s.info.summary, 80), fx.dark_grey());
        }
    }
    println!("{}", format!("\n{} из {}. Подробнее: pult explain <имя> · запуск: pult run <имя>", list.len(), p.scripts.len()).dark_grey());
}

fn cmd_explain(p: &Project, q: &str) {
    let s = find(p, q).unwrap_or_else(|e| fail(&e));
    let i = &s.info;
    let e = effects();
    let k = kb::get();
    let stage = k.stages.iter().find(|x| x.id == s.stage).map(|x| x.label.clone()).unwrap_or_default();
    println!(
        "\n{}  {} · {}",
        s.name.as_str().bold(),
        stage.dark_grey(),
        if s.where_ == "." { "корень".to_string() } else { s.where_.clone() }.dark_grey()
    );
    println!("\n{}\n  {}", "Что делает".bold(), i.summary);
    for x in &i.steps {
        println!("  · {}", x.as_str().dark_grey());
    }
    if !i.about.is_empty() {
        println!("  {} {}", "Программа:".dark_grey(), i.about);
    }
    if !i.parts.is_empty() {
        println!("\n{}", "Команда по частям".bold());
        let parts = &k.ui["parts"];
        for pt in &i.parts {
            let note = if pt.note.is_empty() { parts[&pt.kind].as_str().unwrap_or("").to_string() } else { pt.note.clone() };
            println!("  {}  {}", format!("{:<22}", pt.text).cyan(), note.dark_grey());
        }
    }
    if !i.how.is_empty() {
        println!("\n{}", "Что происходит внутри".bold());
        for (n, h) in i.how.iter().enumerate() {
            println!("  {}. {}", n + 1, h);
        }
    }
    println!("\n{}", "Что произойдёт при запуске".bold());
    for f in &i.fx {
        println!("  {} {}", "●".with(color_ct(e[f]["level"].as_str().unwrap_or("info"))), e[f]["say"].as_str().unwrap_or(f));
    }
    let w = &k.ui["will"];
    println!("  {} {}", "●".blue(), if i.long { w["long"].as_str().unwrap_or("") } else { w["short"].as_str().unwrap_or("") });
    println!("\n{}\n  {}", "Команда".bold(), s.cmd.as_str().cyan());
    let flags: Vec<&pult_core::FlagInfo> = i.flags.iter().take(16).collect();
    if !flags.is_empty() {
        println!("\n{}", "Ключи".bold());
        for f in flags {
            println!(
                "  {:<22}  {}{}",
                f.flag,
                f.text.as_str().dark_grey(),
                if f.used { " · уже в команде".green().to_string() } else { String::new() }
            );
        }
        println!("{}", format!("  пример: pult run {} -- {}", display_argv(&[label(s)]), i.flags[0].flag).dark_grey());
    }
    if !i.topics.is_empty() {
        println!("\n{} {}", "Уроки:".bold(), i.topics.join(", "));
    }
    println!();
}
fn color_ct(level: &str) -> ratatui::crossterm::style::Color {
    use ratatui::crossterm::style::Color as C;
    match level {
        "ok" => C::Green,
        "warn" => C::Yellow,
        "bad" => C::Red,
        _ => C::Blue,
    }
}

fn ask_yes(q: &str) -> bool {
    print!("{q} [y/N] ");
    let _ = io::stdout().flush();
    let mut s = String::new();
    let _ = io::stdin().read_line(&mut s);
    matches!(s.trim().to_lowercase().as_str(), "y" | "yes" | "д" | "да")
}

fn run_in_terminal(p: &Project, s: &Script, args: &str) -> i32 {
    let mut argv = s.argv.clone();
    if let Ok(extra) = shell_words::split(args) {
        if !extra.is_empty() {
            if s.source == "package.json" && argv.first().is_some_and(|x| x == "npm") {
                argv.push("--".into());
            }
            argv.extend(extra);
        }
    }
    let cwd = Path::new(&p.path).join(if s.where_ == "." { "" } else { &s.where_ });
    println!(
        "\n{} {}  {}\n",
        "▶".blue(),
        display_argv(&argv).bold(),
        format!("в {}", if s.where_ == "." { "корне проекта" } else { &s.where_ }).dark_grey()
    );
    let t0 = Instant::now();
    let code = runner::run_inherit(&argv, &cwd).unwrap_or_else(|e| {
        eprintln!("{} {e}", "Не запустилось:".red());
        -1
    });
    let took = dur(t0.elapsed().as_millis() as u64);
    if code == 0 {
        println!("\n{}", format!("■ Готово за {took}").green());
    } else {
        println!("\n{}", format!("■ Ошибка: код {code} · {took}").red());
    }
    code
}

fn cmd_run(p: &Project, q: &str, args: &str, yes: bool) -> i32 {
    let s = find(p, q).unwrap_or_else(|e| fail(&e));
    if s.tool.as_deref().is_some_and(|t| runner::which(t).is_none()) {
        fail(&format!("Не запустится: нет программы «{}»", s.tool.clone().unwrap_or_default()));
    }
    if s.danger && !yes {
        println!("{} {}: {}\n  {}", "!".yellow(), s.name.as_str().bold(), s.info.summary, s.cmd.as_str().cyan());
        if !ask_yes("Это похоже на действие, которое не отменить. Запустить?") {
            fail("Отменено");
        }
    }
    run_in_terminal(p, s, args)
}

fn ci_remote(p: &Project) -> Option<pult_core::model::Remote> {
    let mut r: Vec<_> = p
        .git
        .as_ref()?
        .remotes
        .iter()
        .filter(|r| !r.dup && r.host.as_ref().is_some_and(|h| h.contains("gitlab") || h.contains("github")))
        .cloned()
        .collect();
    r.sort_by_key(|x| !x.host.as_deref().unwrap_or("").contains("gitlab"));
    r.into_iter().next()
}
fn cmd_ci(p: &Project, rest: &[String]) {
    let r = ci_remote(p).unwrap_or_else(|| fail("Нет адреса репозитория на GitLab или GitHub"));
    let dir = Path::new(&p.path);
    if rest.first().map(String::as_str) == Some("log") {
        let id: u64 = rest.get(1).and_then(|x| x.parse().ok()).unwrap_or_else(|| fail("Укажите номер джобы: pult ci log 123"));
        print!("{}", ci::job_log(dir, &r, id).unwrap_or_else(|e| fail(&e)));
        return;
    }
    if let Some(id) = rest.first().and_then(|x| x.parse::<u64>().ok()) {
        for st in ci::jobs(dir, &r, id).unwrap_or_else(|e| fail(&e)) {
            println!("\n  {}", st.name.as_str().bold());
            for j in st.jobs {
                println!(
                    "    {} {:<34} {:<11} {}  #{}",
                    "●".with(color_ct(match j.status.as_str() {
                        "success" => "ok",
                        "failed" => "bad",
                        "running" => "info",
                        _ => "warn",
                    })),
                    j.name,
                    ru(&j.status),
                    j.duration.map(|d| dur(d.saturating_mul(1000))).unwrap_or_default(),
                    j.id
                );
            }
        }
        println!("{}", "\nЖурнал джобы: pult ci log <номер>".dark_grey());
        return;
    }
    println!("\n{}\n", format!("{} · {}", r.host.clone().unwrap_or_default(), r.slug.clone().unwrap_or_default()).bold());
    for x in ci::runs(dir, &r, None).unwrap_or_else(|e| fail(&e)).into_iter().take(15) {
        println!(
            "  {} #{:<12} {:<26} {:<11} {}",
            "●".with(color_ct(match x.status.as_str() {
                "success" => "ok",
                "failed" => "bad",
                "running" => "info",
                _ => "warn",
            })),
            x.id,
            x.r#ref.replace("refs/merge-requests/", "MR !").replace("/head", ""),
            ru(&x.status),
            ago(parse_iso(&x.created_at))
        );
    }
    println!("{}", "\nЭтапы и джобы: pult ci <номер>".dark_grey());
}

/// Урок текстом: секции из того же JSON, что у окна.
fn lesson_lines(t: &Value, width: usize) -> Vec<Line<'static>> {
    let mut out: Vec<Line> = Vec::new();
    let color = t["color"].as_str().and_then(hex).unwrap_or(ACCENT);
    let s = |v: &Value| v.as_str().unwrap_or("").to_string();
    out.push(Line::from(Span::styled(
        format!(" {} ", s(&t["title"])),
        Style::default().fg(Color::Black).bg(color).add_modifier(Modifier::BOLD),
    )));
    out.push(Line::from(s(&t["summary"])));
    for sec in t["sections"].as_array().into_iter().flatten() {
        out.push(Line::from(""));
        let title = s(&sec["title"]);
        let ty = s(&sec["type"]);
        let head = if title.is_empty() {
            match ty.as_str() {
                "terms" => "Слова, которые надо знать".into(),
                "mistakes" => "Что ломается и как чинить".into(),
                "cheatsheet" => "Шпаргалка".into(),
                "rules" => "Как правильно".into(),
                "practice" => "Попробуйте сами".into(),
                "quiz" => "Проверьте себя".into(),
                _ => String::new(),
            }
        } else {
            title
        };
        if !head.is_empty() && ty != "project" {
            out.push(Line::from(Span::styled(format!("▌ {head}"), Style::default().fg(color).add_modifier(Modifier::BOLD))));
        }
        match ty.as_str() {
            "idea" => {
                out.push(Line::from(s(&sec["text"])));
                if !s(&sec["analogy"]).is_empty() {
                    out.push(Line::from(vec![Span::styled("Как в жизни: ", Style::default().fg(MUTE)), Span::raw(s(&sec["analogy"]))]));
                }
            }
            "diagram" => {
                let names: HashMap<String, String> =
                    sec["nodes"].as_array().into_iter().flatten().map(|n| (s(&n["id"]), s(&n["label"]))).collect();
                for n in sec["nodes"].as_array().into_iter().flatten() {
                    out.push(Line::from(vec![
                        Span::styled(format!("  [{}] ", s(&n["label"])), Style::default().fg(color)),
                        Span::styled(s(&n["note"]), Style::default().fg(MUTE)),
                    ]));
                }
                for e in sec["edges"].as_array().into_iter().flatten() {
                    let a = names.get(&s(&e[0])).cloned().unwrap_or_default();
                    let b = names.get(&s(&e[1])).cloned().unwrap_or_default();
                    out.push(Line::from(vec![
                        Span::raw(format!("  {a} ")),
                        Span::styled(format!("──{}──▶", s(&e[2])), Style::default().fg(MUTE)),
                        Span::raw(format!(" {b}")),
                    ]));
                }
            }
            "terms" => {
                for x in sec["items"].as_array().into_iter().flatten() {
                    out.push(Line::from(vec![
                        Span::styled(format!("  {} ", s(&x["term"])), Style::default().add_modifier(Modifier::BOLD)),
                        Span::styled(format!("- {}", s(&x["text"])), Style::default().fg(MUTE)),
                    ]));
                }
            }
            "steps" => {
                for (n, x) in sec["items"].as_array().into_iter().flatten().enumerate() {
                    out.push(Line::from(vec![Span::styled(format!("  {}. ", n + 1), Style::default().fg(color)), Span::raw(s(x))]));
                }
            }
            "anatomy" => {
                for p in sec["parts"].as_array().into_iter().flatten() {
                    out.push(Line::from(vec![
                        Span::styled(format!("  {:<20}  ", s(&p["text"])), Style::default().fg(Color::Cyan)),
                        Span::styled(s(&p["note"]), Style::default().fg(MUTE)),
                    ]));
                }
            }
            "example" => {
                if !s(&sec["label"]).is_empty() {
                    out.push(Line::from(Span::styled(
                        format!("  {}", s(&sec["label"])),
                        Style::default().fg(MUTE).add_modifier(Modifier::BOLD),
                    )));
                }
                for l in s(&sec["code"]).lines() {
                    out.push(Line::from(Span::styled(format!("  │ {l}"), Style::default().fg(Color::Cyan))));
                }
                if !s(&sec["output"]).is_empty() {
                    if !s(&sec["outputLabel"]).is_empty() {
                        out.push(Line::from(Span::styled(
                            format!("  {}", s(&sec["outputLabel"])),
                            Style::default().fg(MUTE).add_modifier(Modifier::BOLD),
                        )));
                    }
                    for l in s(&sec["output"]).lines() {
                        out.push(Line::from(Span::styled(format!("  ⟶ {l}"), Style::default().fg(MUTE))));
                    }
                }
                if !s(&sec["note"]).is_empty() {
                    out.push(Line::from(Span::styled(format!("  {}", s(&sec["note"])), Style::default().fg(MUTE))));
                }
            }
            "codepair" => {
                // в терминале два кода - один под другим
                for side in ["left", "right"] {
                    out.push(Line::from(Span::styled(
                        format!("  {}", s(&sec[side]["label"])),
                        Style::default().fg(MUTE).add_modifier(Modifier::BOLD),
                    )));
                    for l in s(&sec[side]["code"]).lines() {
                        out.push(Line::from(Span::styled(format!("  │ {l}"), Style::default().fg(Color::Cyan))));
                    }
                }
                if !s(&sec["note"]).is_empty() {
                    out.push(Line::from(Span::styled(format!("  {}", s(&sec["note"])), Style::default().fg(MUTE))));
                }
            }
            "practice" => {
                out.push(Line::from(Span::raw(format!("  {}", s(&sec["task"])))));
                for (n, x) in sec["steps"].as_array().into_iter().flatten().enumerate() {
                    out.push(Line::from(vec![Span::styled(format!("  {}. ", n + 1), Style::default().fg(color)), Span::raw(s(x))]));
                }
                if !s(&sec["check"]).is_empty() {
                    out.push(Line::from(Span::styled(
                        format!("  Как понять, что получилось: {}", s(&sec["check"])),
                        Style::default().fg(MUTE),
                    )));
                }
                if !s(&sec["hint"]).is_empty() {
                    out.push(Line::from(Span::styled(format!("  Подсказка: {}", s(&sec["hint"])), Style::default().fg(MUTE))));
                }
                if !s(&sec["solution"]).is_empty() {
                    out.push(Line::from(Span::styled("  Решение:", Style::default().fg(MUTE).add_modifier(Modifier::BOLD))));
                    for l in s(&sec["solution"]).lines() {
                        out.push(Line::from(Span::styled(format!("  │ {l}"), Style::default().fg(Color::Cyan))));
                    }
                }
            }
            "rules" => {
                for x in sec["items"].as_array().into_iter().flatten() {
                    let (lv, word) = match s(&x["kind"]).as_str() {
                        "do" => ("ok", "делайте"),
                        "dont" => ("bad", "не делайте"),
                        "when" => ("info", "когда"),
                        _ => ("warn", "совет"),
                    };
                    let title = s(&x["title"]);
                    out.push(Line::from(vec![
                        Span::styled(format!("  {word:<11}"), Style::default().fg(fx_color(lv)).add_modifier(Modifier::BOLD)),
                        Span::raw(if title.is_empty() { s(&x["text"]) } else { format!("{title}. {}", s(&x["text"])) }),
                    ]));
                }
            }
            "compare" => {
                let cols: Vec<String> = sec["cols"].as_array().into_iter().flatten().map(&s).collect();
                let w = (width.saturating_sub(4) / cols.len().max(1)).clamp(8, 30);
                let rows: Vec<Vec<String>> =
                    sec["rows"].as_array().into_iter().flatten().map(|r| r.as_array().into_iter().flatten().map(&s).collect()).collect();
                let fits = cols.iter().chain(rows.iter().flatten()).all(|x| x.chars().count() < w);
                if fits {
                    let cell = |x: &str| format!("{x:<w$}");
                    out.push(Line::from(Span::styled(
                        format!("  {}", cols.iter().map(|c| cell(c)).collect::<String>()),
                        Style::default().add_modifier(Modifier::BOLD),
                    )));
                    for r in &rows {
                        out.push(Line::from(format!("  {}", r.iter().map(|c| cell(c)).collect::<String>())));
                    }
                } else {
                    // ячейки не влезают в колонки: строка таблицы - карточкой, текст целиком, без обрезки
                    for r in &rows {
                        out.push(Line::from(Span::styled(
                            format!("  {}", r.first().cloned().unwrap_or_default()),
                            Style::default().add_modifier(Modifier::BOLD),
                        )));
                        for (c, v) in cols.iter().zip(r).skip(1) {
                            out.push(Line::from(vec![Span::styled(format!("    {c}: "), Style::default().fg(MUTE)), Span::raw(v.clone())]));
                        }
                    }
                }
            }
            "chart" => {
                let items: Vec<&Value> = sec["items"].as_array().into_iter().flatten().collect();
                let max = items.iter().filter_map(|x| x["value"].as_f64()).fold(1.0, f64::max);
                for x in items {
                    let v = x["value"].as_f64().unwrap_or(0.0);
                    out.push(Line::from(vec![
                        Span::raw(format!("  {:<18} ", clip(&s(&x["label"]), 18))),
                        Span::styled("█".repeat(((v / max) * 30.0).round().max(1.0) as usize), Style::default().fg(color)),
                        Span::raw(format!(" {v} {}", s(&sec["unit"]))),
                    ]));
                }
                if !s(&sec["note"]).is_empty() {
                    out.push(Line::from(Span::styled(format!("  {}", s(&sec["note"])), Style::default().fg(MUTE))));
                }
            }
            "mistakes" => {
                for x in sec["items"].as_array().into_iter().flatten() {
                    out.push(Line::from(vec![Span::styled("  ✗ ", Style::default().fg(fx_color("bad"))), Span::raw(s(&x["problem"]))]));
                    out.push(Line::from(vec![
                        Span::styled("  ✓ ", Style::default().fg(fx_color("ok"))),
                        Span::styled(s(&x["fix"]), Style::default().fg(MUTE)),
                    ]));
                }
            }
            "cheatsheet" => {
                let mut group = String::new();
                for x in sec["items"].as_array().into_iter().flatten() {
                    let g = s(&x["group"]);
                    if !g.is_empty() && g != group {
                        out.push(Line::from(Span::styled(format!("  {g}"), Style::default().fg(MUTE).add_modifier(Modifier::BOLD))));
                        group = g;
                    }
                    out.push(Line::from(vec![
                        Span::styled(format!("  {:<28}  ", s(&x["cmd"])), Style::default().fg(Color::Cyan)),
                        Span::styled(s(&x["text"]), Style::default().fg(MUTE)),
                    ]));
                }
            }
            "quiz" => {
                for (n, q) in sec["items"].as_array().into_iter().flatten().enumerate() {
                    out.push(Line::from(format!("  {}. {}", n + 1, s(&q["q"]))));
                    let ans = q["answer"].as_u64().unwrap_or(0) as usize;
                    for (k, o) in q["options"].as_array().into_iter().flatten().enumerate() {
                        out.push(Line::from(Span::styled(
                            format!("     {} {}", if k == ans { "▸" } else { "·" }, s(o)),
                            Style::default().fg(if k == ans { fx_color("ok") } else { MUTE }),
                        )));
                    }
                    out.push(Line::from(Span::styled(
                        format!("     ответ: {}", s(&q["why"])),
                        Style::default().fg(MUTE).add_modifier(Modifier::ITALIC),
                    )));
                }
            }
            _ => {}
        }
    }
    out
}
fn hex(h: &str) -> Option<Color> {
    let h = h.trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    Some(Color::Rgb(u8::from_str_radix(&h[0..2], 16).ok()?, u8::from_str_radix(&h[2..4], 16).ok()?, u8::from_str_radix(&h[4..6], 16).ok()?))
}

fn cmd_learn(p: &Project, q: Option<&str>) {
    let k = kb::get();
    match q {
        None => {
            println!("\n{}", "Уроки для этого проекта".bold());
            for t in &p.topics {
                println!("  {}  {}  {}", format!("{:<18}", t.id).cyan(), t.title, format!("({})", t.why).dark_grey());
            }
            println!("{}", "\nЧитать: pult learn <урок>   ·   все уроки: pult learn --all".dark_grey());
        }
        Some("--all") => {
            for t in &k.topics {
                println!("  {}  {}", format!("{:<18}", t["id"].as_str().unwrap_or("")).cyan(), t["title"].as_str().unwrap_or(""));
            }
        }
        Some(id) => {
            let t = k
                .topics
                .iter()
                .find(|t| t["id"] == id || t["title"].as_str().is_some_and(|x| x.eq_ignore_ascii_case(id)))
                .unwrap_or_else(|| fail(&format!("Урока «{id}» нет. Список: pult learn --all")));
            let w = ratatui::crossterm::terminal::size().map(|s| s.0 as usize).unwrap_or(100);
            for l in lesson_lines(t, w) {
                // печать без ratatui: только текст, цвета - через crossterm
                let mut line = String::new();
                for sp in l.spans {
                    let mut c = sp.content.to_string().sc();
                    if let Some(Color::Rgb(r, g, b)) = sp.style.fg {
                        c = c.with(ratatui::crossterm::style::Color::Rgb { r, g, b });
                    } else if sp.style.fg == Some(Color::Cyan) {
                        c = c.cyan();
                    }
                    if let Some(Color::Rgb(r, g, b)) = sp.style.bg {
                        c = c.on(ratatui::crossterm::style::Color::Rgb { r, g, b }).black();
                    }
                    if sp.style.add_modifier.contains(Modifier::BOLD) {
                        c = c.bold();
                    }
                    line.push_str(&c.to_string());
                }
                println!("{line}");
            }
            // урок могли назвать заголовком («Docker»): скрипты ищутся по его id
            let tid = t["id"].as_str().unwrap_or(id);
            let rows: Vec<&Script> = p.scripts.iter().filter(|s| s.info.topics.iter().any(|x| x == tid)).take(12).collect();
            if !rows.is_empty() {
                println!("\n{}", "▌ Где это в вашем проекте".bold());
                for s in rows {
                    println!("  {}  {}", format!("{:<28}", label(s)).cyan(), s.info.summary.as_str().dark_grey());
                }
            }
        }
    }
}

/// ИИ-часть проекта: те же данные, что на вкладке «ИИ» окна (pult_core::ai).
fn cmd_ai(p: &Project, rest: &[String]) {
    let k = kb::get();
    let m = ai::build(Path::new(&p.path), p, &k);
    if rest.iter().any(|a| a == "--json") {
        println!("{}", serde_json::to_string_pretty(&m).unwrap_or_default());
        return;
    }
    if m.items.is_empty() {
        println!("\n{}", "ИИ-части в этой папке не найдено".bold());
        return;
    }
    let cap = if m.capped { ", обход обрезан" } else { "" };
    println!(
        "\n{} {}  {}",
        "ИИ-часть".bold(),
        p.name.as_str().bold(),
        format!("({} деталей, {} мс, обошли {} файлов{cap})", m.items.len(), m.ms, m.walked).dark_grey()
    );
    for st in m.kinds.iter().filter(|s| s.count > 0) {
        let modes = st.modes.iter().map(|(mode, n)| format!("{n} {}", ai::mode_label(&k, &st.id, mode))).collect::<Vec<_>>().join(", ");
        let roles = if st.roles.is_empty() {
            String::new()
        } else {
            format!(" [{}]", st.roles.iter().map(|(r, n)| format!("{r} {n}")).collect::<Vec<_>>().join(", "))
        };
        println!("\n{} ({})  {}", ai::kind_title(&k, &st.id).bold(), st.count, format!("{modes}{roles}").dark_grey());
        for it in m.items.iter().filter(|it| it.kind == st.id).take(60) {
            let mut tags: Vec<String> = vec![];
            if !it.paths.is_empty() {
                tags.push(format!("по путям: {} шабл., {} файлов", it.paths.len(), it.files_matched));
            }
            if let Some(e) = &it.event {
                tags.push(format!("{e}{}", it.matcher.as_ref().map(|x| format!(" [{x}]")).unwrap_or_default()));
            }
            if let Some(x) = &it.model {
                tags.push(x.clone());
            }
            if it.unwired {
                tags.push("не подключён".into());
            }
            if it.active == Some(false) {
                tags.push("git их не видит".into());
            }
            if let Some(b) = &it.broken {
                tags.push(format!("не разобран: {b}"));
            }
            let refs = if it.refs_in.is_empty() { String::new() } else { format!(" ←{}", it.refs_in.len()) };
            println!("  {}  {}{}", format!("{:<26}", clip(&it.name, 26)).cyan(), clip(&it.summary, 90), refs.as_str().dark_grey());
            if !tags.is_empty() {
                println!("  {}  {}", " ".repeat(26), tags.join(" · ").dark_grey());
            }
        }
    }
    if !m.story.is_empty() {
        println!("\n{}", "Как это работает вместе".bold());
        for (i, s) in m.story.iter().enumerate() {
            println!("  {}. {}  {}", i + 1, s.title.as_str().cyan(), s.text.as_str().dark_grey());
        }
    }
    for pr in &m.problems {
        println!("\n{} {}: {}", "Не разобран".yellow(), pr.file, pr.error);
    }
    println!();
}

fn cmd_ui(root: Option<String>) {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join(if cfg!(windows) { "Pult.exe" } else { "Pult" })))
        .filter(|p| p.exists());
    match exe {
        Some(e) => {
            let mut c = std::process::Command::new(e);
            if let Some(r) = root {
                c.arg(r);
            }
            let _ = c.spawn();
            println!("{}", "Открываю окно Пульта…".green());
        }
        None => fail("Рядом с pult.exe нет Pult.exe"),
    }
}

// =====================================================================
// интерактивный экран
// =====================================================================
#[derive(PartialEq, Clone, Copy)]
enum View {
    Overview,
    Scripts,
    Ci,
    Learn,
    Projects,
}
const VIEWS: [(View, &str); 5] =
    [(View::Overview, "Обзор"), (View::Scripts, "Скрипты"), (View::Ci, "CI"), (View::Learn, "Уроки"), (View::Projects, "Проекты")];

struct Tui {
    p: Project,
    view: View,
    q: String,
    stage: Option<String>,
    sel: ListState,
    tools: BTreeMap<String, Option<String>>,
    msg: String,
    ci_runs: Option<Result<Vec<ci::Run>, String>>,
    ci_jobs: Option<(u64, Result<Vec<ci::StageJobs>, String>)>,
    ci_sel: ListState,
    learn_sel: ListState,
    learn_scroll: u16,
    proj_sel: ListState,
    pager: Option<(String, Vec<String>, usize)>,
    fresh: Option<mpsc::Receiver<Project>>,
}

impl Tui {
    fn filtered(&self) -> Vec<&Script> {
        let low = self.q.to_lowercase();
        let k = kb::get();
        let mut out = Vec::new();
        for st in &k.stages {
            for s in self.p.scripts.iter().filter(|s| s.stage == st.id) {
                if self.stage.as_deref().is_some_and(|x| x != s.stage) {
                    continue;
                }
                if !low.is_empty() && !format!("{} {} {} {}", s.name, s.info.summary, s.cmd, s.where_).to_lowercase().contains(&low) {
                    continue;
                }
                out.push(s);
            }
        }
        out
    }
}

fn header(f: &mut Frame, area: Rect, t: &Tui) {
    let g =
        t.p.git
            .as_ref()
            .map(|g| {
                format!(
                    "{}{}{}",
                    g.branch,
                    if g.dirty > 0 { format!(" ●{}", g.dirty) } else { " ✓".into() },
                    if g.behind > 0 { format!(" ↓{}", g.behind) } else { String::new() }
                )
            })
            .unwrap_or_else(|| "без git".into());
    let [l, r] = Layout::horizontal([Constraint::Min(20), Constraint::Length(48)]).areas(area);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" ▶ Пульт ", Style::default().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::raw(format!("  {}  ", t.p.name)),
            Span::styled(g, Style::default().fg(MUTE)),
        ])),
        l,
    );
    let idx = VIEWS.iter().position(|(v, _)| *v == t.view).unwrap_or(0);
    f.render_widget(
        Tabs::new(VIEWS.iter().enumerate().map(|(i, (_, n))| format!("{} {n}", i + 1)))
            .select(idx)
            .highlight_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
            .divider(" "),
        r,
    );
}

fn draw(f: &mut Frame, t: &mut Tui) {
    let [top, body, foot] = Layout::vertical([Constraint::Length(1), Constraint::Min(5), Constraint::Length(1)]).areas(f.area());
    header(f, top, t);
    let hint = match t.view {
        _ if t.pager.is_some() => " ↑↓ PgUp PgDn Home End · e следующая ошибка · Esc назад",
        View::Scripts => " Enter запуск · ↑↓ выбор · печатайте для поиска · ←→ этап · Tab экран · Ctrl+R перечитать · Esc выход",
        View::Ci => " Enter этапы / журнал · ↑↓ выбор · r обновить · Backspace назад · Tab экран · Esc выход",
        View::Learn => " ↑↓ урок · PgUp PgDn прокрутка · Tab экран · Esc выход",
        View::Projects => " Enter открыть · ↑↓ выбор · Tab экран · Esc выход",
        View::Overview => " Tab или 1-5 экраны · Ctrl+R перечитать · Esc выход",
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(hint, Style::default().fg(MUTE)),
            Span::styled(format!("  {}", t.msg), Style::default().fg(fx_color("warn"))),
        ])),
        foot,
    );
    if let Some((title, lines, top_line)) = &t.pager {
        let errs = lines.iter().filter(|l| l.to_lowercase().contains("error") || l.contains("ошибка")).count();
        let text: Vec<Line> = lines
            .iter()
            .skip(*top_line)
            .take(body.height as usize)
            .map(|l| {
                if l.to_lowercase().contains("error") || l.to_lowercase().contains("failed") {
                    Line::from(Span::styled(l.clone(), Style::default().fg(fx_color("bad"))))
                } else {
                    Line::from(l.clone())
                }
            })
            .collect();
        f.render_widget(
            Paragraph::new(text).block(Block::default().borders(Borders::TOP).title(format!(
                " {title} · строки {}-{} из {} · ошибок {errs} ",
                top_line + 1,
                (top_line + body.height as usize).min(lines.len()),
                lines.len()
            ))),
            body,
        );
        return;
    }
    match t.view {
        View::Scripts => draw_scripts(f, body, t),
        View::Overview => draw_overview(f, body, t),
        View::Ci => draw_ci(f, body, t),
        View::Learn => draw_learn(f, body, t),
        View::Projects => draw_projects(f, body, t),
    }
}

fn draw_scripts(f: &mut Frame, area: Rect, t: &mut Tui) {
    let [search, list, detail] = Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(6)]).areas(area);
    let k = kb::get();
    let stage =
        t.stage.as_ref().and_then(|s| k.stages.iter().find(|x| &x.id == s)).map(|x| x.label.clone()).unwrap_or_else(|| "все этапы".into());
    let items: Vec<Script> = t.filtered().into_iter().cloned().collect();
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Поиск: ", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(t.q.clone()),
            Span::styled("▏", Style::default().fg(ACCENT)),
            Span::styled(format!("   {} из {} · ←→ этап: ", items.len(), t.p.scripts.len()), Style::default().fg(MUTE)),
            Span::styled(stage, Style::default().fg(ACCENT)),
        ])),
        search,
    );
    let name_w = (list.width as usize * 30 / 100).clamp(16, 38);
    let rows: Vec<ListItem> = items
        .iter()
        .map(|s| {
            let missing = s.tool.as_ref().is_some_and(|x| t.tools.get(x).is_some_and(|v| v.is_none()));
            let name = label(s);
            let name: String = format!("{:<name_w$}", clip(&name, name_w - 1));
            let style = if missing {
                Style::default().fg(MUTE)
            } else if s.danger {
                Style::default().fg(fx_color("warn"))
            } else {
                Style::default()
            };
            let stage = k.stages.iter().find(|x| x.id == s.stage).map(|x| x.label.chars().take(4).collect::<String>()).unwrap_or_default();
            ListItem::new(Line::from(vec![
                Span::styled(format!("{stage:<5}"), Style::default().fg(MUTE)),
                Span::styled(name, style),
                Span::raw(s.info.summary.clone()),
            ]))
        })
        .collect();
    if t.sel.selected().is_none_or(|i| i >= rows.len()) {
        t.sel.select(if rows.is_empty() { None } else { Some(0) });
    }
    f.render_stateful_widget(
        List::new(rows).highlight_style(Style::default().bg(Color::Rgb(40, 44, 60)).add_modifier(Modifier::BOLD)).highlight_symbol("› "),
        list,
        &mut t.sel,
    );
    let s = t.sel.selected().and_then(|i| items.get(i));
    let text = match s {
        Some(s) => {
            let mut l1 = vec![Span::styled(format!(" {} ", s.name), Style::default().add_modifier(Modifier::BOLD))];
            l1.extend(fx_line(s));
            Text::from(vec![
                Line::from(l1),
                Line::from(format!(" {}", s.info.summary)),
                Line::from(vec![
                    Span::styled(" $ ", Style::default().fg(MUTE)),
                    Span::styled(s.cmd.clone(), Style::default().fg(Color::Cyan)),
                ]),
                Line::from(Span::styled(
                    format!(" {}", s.info.how.first().cloned().unwrap_or_else(|| s.info.about.clone())),
                    Style::default().fg(MUTE),
                )),
            ])
        }
        None => Text::from(" Ничего не найдено. Esc - очистить поиск."),
    };
    f.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }).block(Block::default().borders(Borders::TOP)), detail);
}

fn draw_overview(f: &mut Frame, area: Rect, t: &Tui) {
    let p = &t.p;
    let k = kb::get();
    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            format!(" {} {}", p.name, if p.version.is_empty() { String::new() } else { format!("v{}", p.version) }),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!(" {}", p.description.chars().take(area.width as usize * 2).collect::<String>()),
            Style::default().fg(MUTE),
        )),
        Line::from(Span::styled(format!(" {}", p.stacks.join(" · ")), Style::default().fg(ACCENT))),
        Line::from(""),
        Line::from(Span::styled(" Путь разработки", Style::default().add_modifier(Modifier::BOLD))),
    ];
    let mut by: HashMap<&str, usize> = HashMap::new();
    for s in &p.scripts {
        *by.entry(s.stage.as_str()).or_default() += 1;
    }
    let max = by.values().copied().max().unwrap_or(1);
    for st in &k.stages {
        if let Some(n) = by.get(st.id.as_str()) {
            lines.push(Line::from(vec![
                Span::raw(format!("   {:<14} ", st.label)),
                Span::styled("█".repeat((n * 36 / max).max(1)), Style::default().fg(ACCENT)),
                Span::raw(format!(" {n}")),
            ]));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(" Состояние", Style::default().add_modifier(Modifier::BOLD))));
    let dot = |lv: &str, text: String| Line::from(vec![Span::styled("   ● ", Style::default().fg(fx_color(lv))), Span::raw(text)]);
    if let Some(g) = &p.git {
        lines.push(dot(
            if g.dirty > 0 { "warn" } else { "ok" },
            format!(
                "git: ветка {}, {}{}",
                g.branch,
                if g.dirty > 0 { format!("{} не в коммите", g.dirty) } else { "всё в коммитах".into() },
                if g.behind > 0 { format!(", на сервере ещё {}", g.behind) } else { String::new() }
            ),
        ));
    }
    if let Some(d) = &p.health.deps {
        lines.push(dot(
            if d == "ok" {
                "ok"
            } else if d == "stale" {
                "warn"
            } else {
                "bad"
            },
            format!(
                "зависимости: {}",
                match d.as_str() {
                    "ok" => "установлены",
                    "stale" => "устарели",
                    _ => "не установлены",
                }
            ),
        ));
    }
    let miss: Vec<&String> = t.tools.iter().filter(|(_, v)| v.is_none()).map(|(k, _)| k).collect();
    lines.push(if miss.is_empty() {
        dot("ok", "все нужные программы есть".into())
    } else {
        dot("bad", format!("нет программ: {}", miss.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")))
    });
    for c in &p.ci {
        lines.push(dot("info", format!("CI: {} - {} джоб", if c.kind == "gitlab" { "GitLab" } else { &c.file }, c.job_count)));
    }
    if !p.topics.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" Уроки для этого проекта (экран 4)", Style::default().add_modifier(Modifier::BOLD))));
        lines.push(Line::from(format!("   {}", p.topics.iter().map(|x| x.title.clone()).collect::<Vec<_>>().join(" · "))));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), area);
}

fn draw_ci(f: &mut Frame, area: Rect, t: &mut Tui) {
    if let Some((id, res)) = &t.ci_jobs {
        let mut items: Vec<ListItem> = Vec::new();
        match res {
            Ok(stages) => {
                for st in stages {
                    items.push(ListItem::new(Line::from(Span::styled(format!(" {}", st.name.to_uppercase()), Style::default().fg(MUTE)))));
                    for j in &st.jobs {
                        items.push(ListItem::new(Line::from(vec![
                            Span::styled("  ● ", Style::default().fg(status_color(&j.status))),
                            Span::raw(format!("{:<38}", j.name)),
                            Span::styled(
                                format!("{:<11}{}", ru(&j.status), j.duration.map(|d| dur(d.saturating_mul(1000))).unwrap_or_default()),
                                Style::default().fg(MUTE),
                            ),
                        ])));
                    }
                }
            }
            Err(e) => items.push(ListItem::new(Span::styled(format!(" {e}"), Style::default().fg(fx_color("bad"))))),
        }
        f.render_stateful_widget(
            List::new(items)
                .block(Block::default().title(format!(" Пайплайн #{id} ")).borders(Borders::TOP))
                .highlight_style(Style::default().bg(Color::Rgb(40, 44, 60)))
                .highlight_symbol("› "),
            area,
            &mut t.ci_sel,
        );
        return;
    }
    let items: Vec<ListItem> = match &t.ci_runs {
        None => vec![ListItem::new(" Загружаю… (статус обновляется только по r - без опроса)")],
        Some(Err(e)) => vec![ListItem::new(Span::styled(format!(" {e}"), Style::default().fg(fx_color("bad"))))],
        Some(Ok(runs)) => runs
            .iter()
            .map(|r| {
                ListItem::new(Line::from(vec![
                    Span::styled(" ● ", Style::default().fg(status_color(&r.status))),
                    Span::raw(format!("#{:<12} {:<28}", r.id, r.r#ref.replace("refs/merge-requests/", "MR !").replace("/head", ""))),
                    Span::styled(format!("{:<11} {}", ru(&r.status), ago(parse_iso(&r.created_at))), Style::default().fg(MUTE)),
                ]))
            })
            .collect(),
    };
    let title = ci_remote(&t.p)
        .map(|r| format!(" {} · {} ", r.host.unwrap_or_default(), r.slug.unwrap_or_default()))
        .unwrap_or_else(|| " CI ".into());
    f.render_stateful_widget(
        List::new(items)
            .block(Block::default().title(title).borders(Borders::TOP))
            .highlight_style(Style::default().bg(Color::Rgb(40, 44, 60)))
            .highlight_symbol("› "),
        area,
        &mut t.ci_sel,
    );
}

fn draw_learn(f: &mut Frame, area: Rect, t: &mut Tui) {
    let [list, body] = Layout::horizontal([Constraint::Length(26), Constraint::Min(20)]).areas(area);
    let k = kb::get();
    let ids: Vec<String> = t.p.topics.iter().map(|x| x.id.clone()).collect();
    let items: Vec<ListItem> =
        t.p.topics
            .iter()
            .map(|x| {
                ListItem::new(Line::from(vec![
                    Span::styled("■ ", Style::default().fg(hex(&x.color).unwrap_or(ACCENT))),
                    Span::raw(x.title.clone()),
                ]))
            })
            .collect();
    if t.learn_sel.selected().is_none() && !items.is_empty() {
        t.learn_sel.select(Some(0));
    }
    f.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::RIGHT).title(" Уроки "))
            .highlight_style(Style::default().bg(Color::Rgb(40, 44, 60)).add_modifier(Modifier::BOLD)),
        list,
        &mut t.learn_sel,
    );
    let topic = t.learn_sel.selected().and_then(|i| ids.get(i)).and_then(|id| k.topic(id));
    let text =
        topic.map(|tp| lesson_lines(tp, body.width as usize)).unwrap_or_else(|| vec![Line::from(" Для этого проекта уроков не нашлось.")]);
    f.render_widget(
        Paragraph::new(text).wrap(Wrap { trim: false }).scroll((t.learn_scroll, 0)).block(Block::default().borders(Borders::NONE)),
        body.inner(ratatui::layout::Margin { horizontal: 1, vertical: 0 }),
    );
}

fn draw_projects(f: &mut Frame, area: Rect, t: &mut Tui) {
    let rec = recent_sorted();
    let items: Vec<ListItem> = rec
        .iter()
        .map(|r| {
            ListItem::new(Line::from(vec![
                Span::raw(if r.pinned { "★ " } else { "  " }),
                Span::styled(format!("{:<26}", r.name), if r.path == t.p.path { Style::default().fg(ACCENT) } else { Style::default() }),
                Span::styled(r.path.clone(), Style::default().fg(MUTE)),
            ]))
        })
        .collect();
    if t.proj_sel.selected().is_none() && !items.is_empty() {
        t.proj_sel.select(Some(0));
    }
    f.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::TOP).title(" Недавние проекты - те же, что в окне "))
            .highlight_style(Style::default().bg(Color::Rgb(40, 44, 60)))
            .highlight_symbol("› "),
        area,
        &mut t.proj_sel,
    );
}
fn recent_sorted() -> Vec<store::Recent> {
    let mut r = store::config().recent;
    r.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.at.cmp(&a.at)));
    r
}

fn suspend_and_run(term: &mut DefaultTerminal, t: &Tui, s: &Script) -> io::Result<i32> {
    ratatui::restore();
    if s.danger {
        println!("{} {}: {}\n  {}", "!".yellow(), s.name.as_str().bold(), s.info.summary, s.cmd.as_str().cyan());
        if !ask_yes("Это похоже на действие, которое не отменить. Запустить?") {
            *term = ratatui::init();
            return Ok(-2);
        }
    }
    let code = run_in_terminal(&t.p, s, "");
    print!("{}", "\nEnter - назад в Пульт ".dark_grey());
    let _ = io::stdout().flush();
    let mut buf = String::new();
    let _ = io::stdin().read_line(&mut buf);
    *term = ratatui::init();
    Ok(code)
}

fn spawn_fresh(root: &Path) -> mpsc::Receiver<Project> {
    let (tx, rx) = mpsc::channel();
    let r = root.to_path_buf();
    std::thread::spawn(move || {
        if let Ok(p) = scan::scan(&r, &scan::ScanOpts { trusted_hosts: store::trusted_hosts() }) {
            store::save_scan(&p);
            let _ = tx.send(p);
        }
    });
    rx
}

fn tui(root: PathBuf) -> io::Result<()> {
    let p = load(&root, false);
    // свежий разбор в фоне - всегда: и поверх кэша, и после только что сделанного
    let fresh = Some(spawn_fresh(&root));
    let tools = env::tools(p.scripts.iter().filter_map(|s| s.tool.as_deref()));
    let mut t = Tui {
        p,
        view: View::Scripts,
        q: String::new(),
        stage: None,
        sel: ListState::default(),
        tools,
        msg: String::new(),
        ci_runs: None,
        ci_jobs: None,
        ci_sel: ListState::default(),
        learn_sel: ListState::default(),
        learn_scroll: 0,
        proj_sel: ListState::default(),
        pager: None,
        fresh,
    };
    let mut term = ratatui::init();
    let _ = execute!(io::stdout(), EnterAlternateScreen);
    let res = (|| -> io::Result<()> {
        loop {
            if let Some(rx) = &t.fresh {
                if let Ok(p) = rx.try_recv() {
                    t.p = p;
                    t.fresh = None;
                }
            }
            term.draw(|f| draw(f, &mut t))?;
            if !event::poll(Duration::from_millis(250))? {
                continue;
            }
            let Event::Key(key) = event::read()? else { continue };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            t.msg.clear();
            if !handle_key(&mut t, key, &mut term)? {
                return Ok(());
            }
        }
    })();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
    ratatui::restore();
    res
}

fn move_sel(s: &mut ListState, n: usize, d: i32) {
    if n == 0 {
        return;
    }
    let cur = s.selected().unwrap_or(0) as i32;
    s.select(Some((cur + d).clamp(0, n as i32 - 1) as usize));
}

/// false - выйти
fn handle_key(t: &mut Tui, key: KeyEvent, term: &mut DefaultTerminal) -> io::Result<bool> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('q')) {
        return Ok(false);
    }
    if let Some((_, lines, top)) = &mut t.pager {
        let page = term.size().map(|s| s.height as usize).unwrap_or(30).saturating_sub(4);
        match key.code {
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('q') => t.pager = None,
            KeyCode::Up => *top = top.saturating_sub(1),
            KeyCode::Down => *top = (*top + 1).min(lines.len().saturating_sub(1)),
            KeyCode::PageUp => *top = top.saturating_sub(page),
            KeyCode::PageDown | KeyCode::Char(' ') => *top = (*top + page).min(lines.len().saturating_sub(1)),
            KeyCode::Home => *top = 0,
            KeyCode::End => *top = lines.len().saturating_sub(page),
            KeyCode::Char('e') => {
                if let Some(i) = lines
                    .iter()
                    .enumerate()
                    .skip(*top + 1)
                    .find(|(_, l)| l.to_lowercase().contains("error") || l.to_lowercase().contains("failed"))
                    .map(|(i, _)| i)
                {
                    *top = i.saturating_sub(3);
                }
            }
            _ => {}
        }
        return Ok(true);
    }
    let vi = VIEWS.iter().position(|(v, _)| *v == t.view).unwrap_or(0);
    match key.code {
        KeyCode::Tab => {
            t.view = VIEWS[(vi + 1) % VIEWS.len()].0;
            return Ok(true);
        }
        KeyCode::BackTab => {
            t.view = VIEWS[(vi + VIEWS.len() - 1) % VIEWS.len()].0;
            return Ok(true);
        }
        KeyCode::Char('r') if ctrl => {
            t.p = load(Path::new(&t.p.path.clone()), true);
            t.msg = "перечитано".into();
            return Ok(true);
        }
        _ => {}
    }
    if t.view != View::Scripts {
        if let KeyCode::Char(c @ '1'..='5') = key.code {
            t.view = VIEWS[(c as u8 - b'1') as usize].0;
            return Ok(true);
        }
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) && !(t.view == View::Ci && t.ci_jobs.is_some()) {
            return Ok(false);
        }
    }
    match t.view {
        View::Scripts => {
            let n = t.filtered().len();
            match key.code {
                KeyCode::Esc => {
                    if t.q.is_empty() && t.stage.is_none() {
                        return Ok(false);
                    }
                    t.q.clear();
                    t.stage = None;
                }
                KeyCode::Up => move_sel(&mut t.sel, n, -1),
                KeyCode::Down => move_sel(&mut t.sel, n, 1),
                KeyCode::PageUp => move_sel(&mut t.sel, n, -15),
                KeyCode::PageDown => move_sel(&mut t.sel, n, 15),
                KeyCode::Home => t.sel.select(Some(0)),
                KeyCode::End => t.sel.select(Some(n.saturating_sub(1))),
                KeyCode::Left | KeyCode::Right => {
                    let k = kb::get();
                    let ids: Vec<Option<String>> = std::iter::once(None)
                        .chain(k.stages.iter().filter(|s| t.p.scripts.iter().any(|x| x.stage == s.id)).map(|s| Some(s.id.clone())))
                        .collect();
                    let i = ids.iter().position(|x| *x == t.stage).unwrap_or(0);
                    t.stage = ids[if key.code == KeyCode::Right { (i + 1) % ids.len() } else { (i + ids.len() - 1) % ids.len() }].clone();
                    t.sel.select(Some(0));
                }
                KeyCode::Backspace => {
                    t.q.pop();
                    t.sel.select(Some(0));
                }
                KeyCode::Enter => {
                    let s = t.sel.selected().and_then(|i| t.filtered().get(i).map(|s| (*s).clone()));
                    if let Some(s) = s {
                        if s.tool.as_ref().is_some_and(|x| t.tools.get(x).is_some_and(|v| v.is_none())) {
                            t.msg = format!("нет программы {}", s.tool.clone().unwrap_or_default());
                        } else {
                            let code = suspend_and_run(term, t, &s)?;
                            t.msg = match code {
                                0 => format!("✓ {} - готово", s.name),
                                -2 => "отменено".into(),
                                c => format!("✗ {} - код {c}", s.name),
                            };
                        }
                    }
                }
                KeyCode::Char(c) if !ctrl => {
                    t.q.push(c);
                    t.sel.select(Some(0));
                }
                _ => {}
            }
        }
        View::Ci => {
            let remote = ci_remote(&t.p);
            if t.ci_runs.is_none() || key.code == KeyCode::Char('r') {
                match &remote {
                    Some(r) if r.trusted => {
                        if let Some((id, _)) = &t.ci_jobs {
                            let id = *id;
                            t.ci_jobs = Some((id, ci::jobs(Path::new(&t.p.path), r, id)));
                        } else {
                            t.ci_runs = Some(ci::runs(Path::new(&t.p.path), r, None));
                        }
                    }
                    Some(r) => {
                        t.ci_runs =
                            Some(Err(format!("Хост {} не в списке доверенных (настройки окна Пульта)", r.host.clone().unwrap_or_default())))
                    }
                    None => t.ci_runs = Some(Err("Нет адреса репозитория на GitLab или GitHub".into())),
                }
                if key.code == KeyCode::Char('r') {
                    return Ok(true);
                }
            }
            let jobs_flat: Option<Vec<ci::Job>> = t.ci_jobs.as_ref().and_then(|(_, r)| r.as_ref().ok()).map(|st| {
                st.iter()
                    .flat_map(|s| std::iter::once(ci::Job { name: String::new(), ..Default::default() }).chain(s.jobs.iter().cloned()))
                    .collect()
            });
            let n = jobs_flat
                .as_ref()
                .map(|v| v.len())
                .or_else(|| t.ci_runs.as_ref().and_then(|r| r.as_ref().ok()).map(|v| v.len()))
                .unwrap_or(0);
            match key.code {
                KeyCode::Up => move_sel(&mut t.ci_sel, n, -1),
                KeyCode::Down => move_sel(&mut t.ci_sel, n, 1),
                KeyCode::Esc | KeyCode::Backspace if t.ci_jobs.is_some() => {
                    t.ci_jobs = None;
                    t.ci_sel.select(Some(0));
                }
                KeyCode::Enter => {
                    let Some(r) = remote else { return Ok(true) };
                    if let Some(list) = jobs_flat {
                        if let Some(j) = t.ci_sel.selected().and_then(|i| list.get(i)).filter(|j| !j.name.is_empty()) {
                            match ci::job_log(Path::new(&t.p.path), &r, j.id) {
                                Ok(text) => {
                                    let lines: Vec<String> = text
                                        .replace("\r\n", "\n")
                                        .split('\n')
                                        .map(|l| l.rsplit('\r').next().unwrap_or("").to_string())
                                        .map(|l| strip_ansi(&l))
                                        .collect();
                                    let top = lines.len().saturating_sub(20);
                                    t.pager = Some((format!("Журнал · {}", j.name), lines, top));
                                }
                                Err(e) => t.msg = e,
                            }
                        }
                    } else if let Some(run) =
                        t.ci_sel.selected().and_then(|i| t.ci_runs.as_ref().and_then(|x| x.as_ref().ok()).and_then(|v| v.get(i)).cloned())
                    {
                        t.ci_jobs = Some((run.id, ci::jobs(Path::new(&t.p.path), &r, run.id)));
                        t.ci_sel.select(Some(1));
                    }
                }
                _ => {}
            }
        }
        View::Learn => {
            let n = t.p.topics.len();
            match key.code {
                KeyCode::Up => {
                    move_sel(&mut t.learn_sel, n, -1);
                    t.learn_scroll = 0;
                }
                KeyCode::Down => {
                    move_sel(&mut t.learn_sel, n, 1);
                    t.learn_scroll = 0;
                }
                KeyCode::PageDown | KeyCode::Char(' ') => t.learn_scroll = t.learn_scroll.saturating_add(15),
                KeyCode::PageUp => t.learn_scroll = t.learn_scroll.saturating_sub(15),
                _ => {}
            }
        }
        View::Projects => {
            let rec = recent_sorted();
            match key.code {
                KeyCode::Up => move_sel(&mut t.proj_sel, rec.len(), -1),
                KeyCode::Down => move_sel(&mut t.proj_sel, rec.len(), 1),
                KeyCode::Enter => {
                    if let Some(r) = t.proj_sel.selected().and_then(|i| rec.get(i)).filter(|r| Path::new(&r.path).is_dir()) {
                        t.p = load(Path::new(&r.path), false);
                        t.tools = env::tools(t.p.scripts.iter().filter_map(|s| s.tool.as_deref()));
                        t.ci_runs = None;
                        t.ci_jobs = None;
                        t.view = View::Scripts;
                        t.sel.select(Some(0));
                        t.learn_sel.select(None);
                    }
                }
                _ => {}
            }
        }
        View::Overview => {}
    }
    Ok(true)
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\x1b' {
            if it.peek() == Some(&'[') {
                it.next();
                for d in it.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Цвет и жирный в обычном выводе (не в экране TUI). В терминал - как есть; в файл, в канал или при
/// NO_COLOR - чистый текст: crossterm сам гасит только цвета, а коды жирного оставляет.
mod paint {
    use ratatui::crossterm::style::{style, Color, StyledContent, Stylize};
    use std::fmt::Display;
    use std::sync::atomic::{AtomicBool, Ordering};

    pub static PLAIN: AtomicBool = AtomicBool::new(false);
    fn on() -> bool {
        !PLAIN.load(Ordering::Relaxed)
    }

    macro_rules! paint_fn {
        ($($n:ident),*) => {$(
            fn $n(self) -> StyledContent<Self::D> {
                let s = self.sc();
                if on() { Stylize::$n(s) } else { s }
            }
        )*};
    }

    pub trait Paint: Sized {
        type D: Display;
        fn sc(self) -> StyledContent<Self::D>;
        paint_fn!(bold, black, blue, cyan, dark_grey, green, red, yellow);
        fn with(self, c: Color) -> StyledContent<Self::D> {
            let s = self.sc();
            if on() {
                Stylize::with(s, c)
            } else {
                s
            }
        }
        fn on(self, c: Color) -> StyledContent<Self::D> {
            let s = self.sc();
            if on() {
                Stylize::on(s, c)
            } else {
                s
            }
        }
    }
    impl<D: Display> Paint for StyledContent<D> {
        type D = D;
        fn sc(self) -> StyledContent<D> {
            self
        }
    }
    impl<'a> Paint for &'a str {
        type D = &'a str;
        fn sc(self) -> StyledContent<&'a str> {
            style(self)
        }
    }
    impl Paint for String {
        type D = String;
        fn sc(self) -> StyledContent<String> {
            style(self)
        }
    }
}

fn main() {
    {
        use std::io::IsTerminal;
        let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        paint::PLAIN.store(no_color || !std::io::stdout().is_terminal(), std::sync::atomic::Ordering::Relaxed);
    }
    // Вывод обрезали (`pult info | Select-Object -First 2`, `| head`): канал закрыт, println! паникует.
    // Это не ошибка программы - выйти молча с кодом 0, как делают git и ls.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let p = info.payload();
        let msg = p.downcast_ref::<String>().map(String::as_str).or_else(|| p.downcast_ref::<&str>().copied()).unwrap_or("");
        if msg.contains("failed printing to stdout")
            && (msg.contains("os error 232") || msg.contains("os error 109") || msg.contains("Broken pipe"))
        {
            std::process::exit(0);
        }
        default_hook(info);
    }));
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let dd = argv.iter().position(|a| a == "--");
    let pass = dd
        .map(|i| argv[i + 1..].iter().map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() }).collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    let head = &argv[..dd.unwrap_or(argv.len())];
    let (mut root, mut yes, mut stage, mut fresh) = (None, false, None, false);
    let mut pos: Vec<String> = Vec::new();
    let mut i = 0;
    while i < head.len() {
        match head[i].as_str() {
            "-C" | "--path" => {
                root = head.get(i + 1).cloned();
                i += 1;
            }
            "--yes" | "-y" => yes = true,
            "--stage" => {
                stage = head.get(i + 1).cloned();
                i += 1;
            }
            "--fresh" => fresh = true,
            a => pos.push(a.to_string()),
        }
        i += 1;
    }
    const CMDS: [&str; 11] = ["help", "-h", "--help", "info", "ls", "explain", "run", "ci", "learn", "ai", "ui"];
    if let Some(first) = pos.first().cloned() {
        if !CMDS.contains(&first.as_str()) && Path::new(&first).is_dir() {
            root = Some(first);
            pos.remove(0);
        }
    }
    let cmd = pos.first().cloned().unwrap_or_default();
    let rest: Vec<String> = pos.iter().skip(1).cloned().collect();
    match cmd.as_str() {
        "help" | "-h" | "--help" => help(),
        "ui" => cmd_ui(rest.first().cloned().or(root).map(|r| canon(&r).to_string_lossy().into_owned())),
        "" => {
            let r = resolve_root(root.as_deref());
            if let Err(e) = tui(r) {
                fail(&e.to_string());
            }
        }
        _ => {
            let r = resolve_root(root.as_deref());
            let p = load(&r, fresh || cmd == "run");
            match cmd.as_str() {
                "info" => cmd_info(&p),
                "ls" => cmd_ls(&p, &rest.join(" "), stage.as_deref()),
                "explain" => cmd_explain(&p, rest.first().map(String::as_str).unwrap_or_else(|| fail("Какой скрипт? pult explain <имя>"))),
                "run" => {
                    let q = rest.first().cloned().unwrap_or_else(|| fail("Какой скрипт? pult run <имя>"));
                    let args = [rest[1..].join(" "), pass].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" ");
                    std::process::exit(cmd_run(&p, &q, &args, yes));
                }
                "ci" => cmd_ci(&p, &rest),
                "learn" => cmd_learn(&p, rest.first().map(String::as_str)),
                "ai" => cmd_ai(&p, &rest),
                _ => {
                    help();
                    std::process::exit(2);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{dur, parse_iso};

    #[test]
    fn dur_shows_tenths_under_ten_seconds() {
        assert_eq!(dur(1_200), "1,2 с");
    }

    #[test]
    fn dur_rounds_before_splitting_minutes() {
        assert_eq!(dur(119_600), "2 м 00 с");
    }

    #[test]
    fn dur_rounds_59_and_a_half_seconds_up_to_a_minute() {
        assert_eq!(dur(59_600), "1 м 00 с");
    }

    #[test]
    fn dur_keeps_seconds_with_leading_zero() {
        assert_eq!(dur(65_000), "1 м 05 с");
    }

    #[test]
    fn parse_iso_reads_utc_timestamp() {
        assert_eq!(parse_iso("2026-10-13T19:51:21Z"), 1_791_921_081_000);
    }

    #[test]
    fn parse_iso_gives_zero_for_go_zero_time() {
        assert_eq!(parse_iso("0001-01-01T00:00:00Z"), 0);
    }

    #[test]
    fn parse_iso_gives_zero_for_out_of_calendar_parts() {
        assert_eq!(parse_iso("2026-13-40T99:99:99Z"), 0);
    }
}
