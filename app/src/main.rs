//! Пульт - окно. Тонкая оболочка над pult-core: команды для интерфейса и события.
//! Нет HTTP-сервера, порта и токена: интерфейс говорит с ядром через IPC Tauri.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use notify::{RecursiveMode, Watcher};
use pult_core::model::{Project, Script};
use pult_core::runner::{self, Live};
use pult_core::store::{self, HistEntry, Settings};
use pult_core::util::{inside, is_unc, now_ms, resolve_inside};
use pult_core::{ai, ci, env, git, helpcmd, kb, scan};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;

mod tray;

type R<T> = Result<T, String>;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct RunPub {
    id: String,
    project: String,
    script_id: String,
    title: String,
    cmd: String,
    #[serde(rename = "where")]
    where_: String,
    args: String,
    status: String,
    code: Option<i32>,
    started_at: u64,
    ended_at: Option<u64>,
    kind: String,
}

struct RunRec {
    p: RunPub,
    log: String,
    live: Option<Arc<Live>>,
}

struct Proj {
    data: Project,
    fp: u64,
    refreshing: bool,
    again: bool,
}

#[derive(Default)]
struct Inner {
    projects: HashMap<String, Proj>,
    runs: HashMap<String, RunRec>,
    watchers: HashMap<String, (u64, notify::RecommendedWatcher)>,
}

struct App {
    inner: Mutex<Inner>,
    log_tx: Mutex<mpsc::Sender<(String, String)>>,
    open_path: Mutex<Option<String>>,
}

type St<'a> = State<'a, Arc<App>>;

/// Замок, который не паникует на «отравлении». Паника в одном потоке (вывод запуска, сверка
/// проекта) иначе роняла бы каждую следующую команду окна, и Пульт переставал отвечать целиком.
/// Под замком только вставки и присваивания: прерванная на середине правка данные не ломает.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Фоновый поток с именем: в сообщении о панике и в отладчике видно, чей он. Не запустился
/// (система исчерпала потоки) - пишем об этом, а не роняем команду окна, как thread::spawn.
fn spawn_named(name: &str, f: impl FnOnce() + Send + 'static) {
    if let Err(e) = std::thread::Builder::new().name(name.into()).spawn(f) {
        eprintln!("поток {name} не запустился: {e}");
    }
}

fn key(p: &str) -> String {
    p.to_lowercase()
}

/// Путь без префикса \\?\, который добавляет canonicalize на Windows.
fn canon(p: &str) -> R<PathBuf> {
    let raw = p.trim().trim_matches('"');
    if raw.is_empty() {
        return Err("Укажите путь к папке".into());
    }
    if is_unc(raw) {
        return Err("Сетевые папки (\\\\сервер\\...) не открываю: обращение к ним отдаёт данные входа Windows".into());
    }
    let c = std::fs::canonicalize(raw).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "Папка не найдена".to_string(),
        std::io::ErrorKind::PermissionDenied => "Нет доступа к папке".to_string(),
        _ => e.to_string(),
    })?;
    if !c.is_dir() {
        return Err("Это файл, а не папка".into());
    }
    let s = c.to_string_lossy().to_string();
    Ok(PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s)))
}

/// JSON проекта идёт прямо в хеш: без копии проекта и без строки в полмегабайта
/// (замер в docs/optimizations.md).
struct HashWriter(DefaultHasher);

impl std::io::Write for HashWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.write(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Отпечаток проекта без полей, которые меняются при каждом разборе или зависят от окна.
/// Эти поля на время хеша обнуляются и возвращаются на место.
fn fingerprint(p: &mut Project) -> u64 {
    let keep = (p.scanned_at, p.scan_ms, p.cached, std::mem::take(&mut p.favorites));
    (p.scanned_at, p.scan_ms, p.cached) = (0, 0, false);
    let mut h = HashWriter(DefaultHasher::new());
    let _ = serde_json::to_writer(&mut h, &*p);
    (p.scanned_at, p.scan_ms, p.cached, p.favorites) = keep;
    h.0.finish()
}

fn for_client(mut p: Project, cached: bool) -> Project {
    p.cached = cached;
    p.favorites = store::config().favorites.get(&p.path).cloned().unwrap_or_default();
    p
}

/// Событие `project`. Проект сериализуется по ссылке сразу в строку: `json!` сначала строил из
/// него дерево Value (для большого проекта - мегабайты кучи), и только потом строку.
#[derive(Serialize, Clone)]
struct ProjectEvent<'a> {
    reason: &'a str,
    data: &'a Project,
}

fn register(app: &App, mut p: Project) {
    let fp = fingerprint(&mut p);
    register_fp(app, p, fp);
}

fn register_fp(app: &App, p: Project, fp: u64) {
    let mut g = lock(&app.inner);
    g.projects.insert(key(&p.path), Proj { data: p, fp, refreshing: false, again: false });
}

fn find_script(app: &App, id: &str) -> Option<(String, Script)> {
    let g = lock(&app.inner);
    g.projects.values().find_map(|p| p.data.scripts.iter().find(|s| s.id == id).map(|s| (p.data.path.clone(), s.clone())))
}

/// Проект по пути: уже открытый - из памяти, иначе разбор (для сценариев из трея, когда окно его не открывало).
fn find_project(handle: &AppHandle, app: &Arc<App>, path: &str) -> R<Project> {
    let root = canon(path)?.to_string_lossy().to_string();
    if let Some(p) = lock(&app.inner).projects.get(&key(&root)) {
        return Ok(p.data.clone());
    }
    let p = scan::scan(Path::new(&root), &opts())?;
    store::save_scan(&p);
    register(app, p.clone());
    let data = for_client(p.clone(), false);
    let _ = handle.emit("project", ProjectEvent { reason: "scenario", data: &data });
    Ok(p)
}

fn opts() -> scan::ScanOpts {
    scan::ScanOpts { trusted_hosts: store::trusted_hosts() }
}

// ---------- фоновая сверка и слежение за файлами ----------
fn refresh(handle: AppHandle, app: Arc<App>, root: String, reason: &'static str) {
    {
        let mut g = lock(&app.inner);
        if let Some(p) = g.projects.get_mut(&key(&root)) {
            if p.refreshing {
                p.again = true;
                return;
            }
            p.refreshing = true;
        }
    }
    spawn_named("pult-refresh", move || {
        match scan::scan(Path::new(&root), &opts()) {
            Ok(mut p) => {
                let has_ai = !p.ai_markers.is_empty();
                let fp = fingerprint(&mut p);
                let changed = lock(&app.inner).projects.get(&key(&root)).is_none_or(|x| x.fp != fp);
                if changed {
                    // кэш на диске переписывается только при изменениях: прежде каждая сверка
                    // писала полмегабайта JSON ради одного нового scannedAt
                    store::save_scan(&p);
                    let data = for_client(p.clone(), false);
                    register_fp(&app, p, fp);
                    let _ = handle.emit("project", ProjectEvent { reason, data: &data });
                } else if let Some(x) = lock(&app.inner).projects.get_mut(&key(&root)) {
                    x.data.scanned_at = p.scanned_at;
                }
                if reason == "files" && has_ai {
                    ai_refresh(handle.clone(), app.clone(), root.clone());
                }
            }
            Err(e) => {
                let _ = handle.emit("project-error", json!({ "path": root, "error": e }));
            }
        }
        let again = match lock(&app.inner).projects.get_mut(&key(&root)) {
            Some(x) => {
                x.refreshing = false;
                std::mem::take(&mut x.again)
            }
            None => false,
        };
        if again {
            std::thread::sleep(Duration::from_millis(300));
            refresh(handle, app, root, reason);
        }
    });
}

/// Пересобрать карту ИИ-части в фоне; окно получает событие `ai`, только если карта изменилась.
fn ai_refresh(handle: AppHandle, app: Arc<App>, root: String) {
    spawn_named("pult-ai", move || {
        let Some(project) = lock(&app.inner).projects.get(&key(&root)).map(|p| p.data.clone()) else { return };
        let fresh = ai::build(Path::new(&root), &project, &kb::get());
        if store::cached_ai(&root).is_none_or(|old| old.fp != fresh.fp) {
            store::save_ai(&fresh);
            let _ = handle.emit("ai", json!({ "path": root, "data": fresh }));
        }
    });
}

fn ensure_watch(handle: &AppHandle, app: &Arc<App>, root: &str) {
    if !store::config().settings.auto_refresh {
        return;
    }
    let k = key(root);
    {
        let mut g = lock(&app.inner);
        if let Some(w) = g.watchers.get_mut(&k) {
            w.0 = now_ms();
            return;
        }
        // не больше четырёх наблюдаемых проектов - самый давний отпускаем;
        // при повторном открытии он снова попадёт сюда
        while g.watchers.len() >= 4 {
            let Some(oldest) = g.watchers.iter().min_by_key(|(_, v)| v.0).map(|(k, _)| k.clone()) else {
                break;
            };
            g.watchers.remove(&oldest);
        }
    }
    let (tx, rx) = mpsc::channel::<u8>();
    let rootp = PathBuf::from(root);
    let r2 = rootp.clone();
    let Ok(mut w) = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            for p in &ev.paths {
                let kind = scan::watch_kind(&r2, p, &kb::get());
                if kind > 0 {
                    let _ = tx.send(kind);
                }
            }
        }
    }) else {
        return;
    };
    if w.watch(&rootp, RecursiveMode::Recursive).is_err() {
        return;
    }
    let (h2, a2, root2) = (handle.clone(), app.clone(), root.to_string());
    spawn_named("pult-watch", move || {
        while let Ok(first) = rx.recv() {
            let mut scan_needed = first == 1;
            let mut git_needed = first == 2;
            let mut ai_needed = first == 3;
            // ждём тишины: пачка событий от одной правки - одно обновление
            loop {
                match rx.recv_timeout(Duration::from_millis(if scan_needed { 1200 } else { 700 })) {
                    Ok(1) => scan_needed = true,
                    Ok(3) => ai_needed = true,
                    Ok(_) => git_needed = true,
                    Err(mpsc::RecvTimeoutError::Timeout) => break,
                    Err(_) => return,
                }
            }
            // правка детали ИИ-части не требует нового разбора проекта: пересобирается только её карта
            if ai_needed && !scan_needed {
                ai_refresh(h2.clone(), a2.clone(), root2.clone());
            }
            if scan_needed {
                refresh(h2.clone(), a2.clone(), root2.clone(), "files");
            } else if git_needed {
                let g = git::info(Path::new(&root2), &store::trusted_hosts());
                let mut changed = false;
                if let Some(p) = lock(&a2.inner).projects.get_mut(&key(&root2)) {
                    if p.data.git != g {
                        p.data.git = g.clone();
                        changed = true;
                    }
                }
                if changed {
                    let _ = h2.emit("git", json!({ "path": root2, "git": g }));
                }
            }
        }
    });
    lock(&app.inner).watchers.insert(k, (now_ms(), w));
}

// ---------- команды: проект и знания ----------
#[tauri::command]
async fn state(st: St<'_>) -> R<Value> {
    let c = store::config();
    let recent: Vec<Value> = c
        .recent
        .iter()
        .map(|r| json!({ "path": r.path, "name": r.name, "at": r.at, "pinned": r.pinned, "exists": Path::new(&r.path).is_dir() }))
        .collect();
    let runs: Vec<RunPub> = lock(&st.inner).runs.values().map(|r| r.p.clone()).collect();
    let open = lock(&st.open_path).take();
    Ok(
        json!({ "recent": recent, "last": c.last, "settings": c.settings, "runs": runs, "kb": kb::get().client_bundle(), "openPath": open, "version": env!("CARGO_PKG_VERSION") }),
    )
}

#[tauri::command]
async fn kb_reload() -> R<Value> {
    tauri::async_runtime::spawn_blocking(|| kb::reload().client_bundle()).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn topic(id: String) -> R<Value> {
    kb::get().topic(&id).cloned().ok_or_else(|| format!("Урока «{id}» нет"))
}

#[tauri::command]
async fn scan_project(handle: AppHandle, st: St<'_>, path: String, force: Option<bool>, background: Option<bool>) -> R<Project> {
    let app = st.inner().clone();
    let root = canon(&path)?;
    let root_s = root.to_string_lossy().to_string();
    let force = force.unwrap_or(false);
    let mem = lock(&app.inner).projects.get(&key(&root_s)).map(|p| p.data.clone());
    let (data, cached) = match (mem, force) {
        (Some(d), false) => {
            if now_ms().saturating_sub(d.scanned_at) > 30_000 {
                refresh(handle.clone(), app.clone(), root_s.clone(), "stale");
            }
            (d, true)
        }
        _ => match if force { None } else { store::cached_scan(&root_s) } {
            Some(d) => {
                register(&app, d.clone());
                refresh(handle.clone(), app.clone(), root_s.clone(), "cache");
                (d, true)
            }
            None => {
                let r2 = root.clone();
                let d = tauri::async_runtime::spawn_blocking(move || scan::scan(&r2, &opts())).await.map_err(|e| e.to_string())??;
                store::save_scan(&d);
                register(&app, d.clone());
                (d, false)
            }
        },
    };
    // фоновое переподключение окна не должно менять «последний проект»
    if !background.unwrap_or(false) {
        store::touch_recent(&data.path, &data.name);
    }
    ensure_watch(&handle, &app, &data.path);
    Ok(for_client(data, cached))
}

/// Карта ИИ-части проекта: из кэша сразу (и пересборка в фоне), иначе собирается здесь. Файлы проекта
/// только читаются: ни хук, ни MCP-сервер, ни скрипт не запускаются.
#[tauri::command]
async fn ai_map(handle: AppHandle, st: St<'_>, path: String, force: Option<bool>) -> R<ai::AiMap> {
    let app = st.inner().clone();
    let root = canon(&path)?.to_string_lossy().to_string();
    if !force.unwrap_or(false) {
        if let Some(mut m) = store::cached_ai(&root) {
            if now_ms().saturating_sub(m.built_at) > 30_000 {
                ai_refresh(handle, app, root);
            }
            m.cached = true;
            return Ok(m);
        }
    }
    tauri::async_runtime::spawn_blocking(move || {
        let project = find_project(&handle, &app, &root)?;
        let m = ai::build(Path::new(&root), &project, &kb::get());
        store::save_ai(&m);
        Ok(m)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn pick(handle: AppHandle, initial: Option<String>) -> R<Option<String>> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut d = handle.dialog().file().set_title("Выберите папку проекта");
        if let Some(i) = initial.filter(|i| Path::new(i).is_dir()) {
            d = d.set_directory(i);
        }
        d.blocking_pick_folder().and_then(|f| f.into_path().ok()).map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn env_info(names: Vec<String>, docker: Option<bool>, force: Option<bool>) -> R<Value> {
    tauri::async_runtime::spawn_blocking(move || {
        if force.unwrap_or(false) {
            runner::forget_which();
        }
        let tools = env::tools(names.iter().map(String::as_str));
        let d = docker.unwrap_or(false).then(|| env::docker(force.unwrap_or(false)));
        json!({ "tools": tools, "docker": d })
    })
    .await
    .map_err(|e| e.to_string())
}

/// Справка программы. Файл из папки проекта запускается, только если окно передало его путь
/// в `confirmed` - то есть пользователь увидел путь и согласился.
#[tauri::command]
async fn help(st: St<'_>, script_id: String, confirmed: Option<String>) -> R<helpcmd::Help> {
    let (root, s) = find_script(&st, &script_id).ok_or("Скрипт не найден - обновите проект")?;
    tauri::async_runtime::spawn_blocking(move || helpcmd::ask_confirmed(&s, Path::new(&root), &kb::get(), confirmed.as_deref()))
        .await
        .map_err(|e| e.to_string())
}

/// «Доверять этому репозиторию»: git для него запускается, хотя его настройки могут запускать
/// программы. Сетевой путь в настройках не доверяется никогда.
#[tauri::command]
async fn trust_repo(handle: AppHandle, st: St<'_>, path: String) -> R<Option<pult_core::model::GitInfo>> {
    let root = lock(&st.inner).projects.get(&key(&path)).map(|p| p.data.path.clone()).ok_or("Проект не открыт")?;
    let app = st.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let loc = git::locate(Path::new(&root)).ok_or("Это не git-репозиторий")?;
        if loc.verdict.as_ref().is_some_and(|v| v.refused) {
            return Err("В настройках репозитория сетевой путь - git для него не запускаю".to_string());
        }
        store::trust_repo(&loc.work);
        let g = git::info(Path::new(&root), &store::trusted_hosts());
        if let Some(p) = lock(&app.inner).projects.get_mut(&key(&root)) {
            p.data.git = g.clone();
        }
        let _ = handle.emit("git", json!({ "path": root, "git": g }));
        Ok(g)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------- команды: запуск ----------
fn build_argv(s: &Script, args: &str) -> R<Vec<String>> {
    let mut argv = s.argv.clone();
    let extra = shell_words::split(args).map_err(|_| "Аргументы не разобрались: проверьте кавычки".to_string())?;
    if !extra.is_empty() {
        // npm передаёт аргументы скрипту только после «--»
        if s.source == "package.json" && argv.first().is_some_and(|p| p == "npm") {
            argv.push("--".into());
        }
        argv.extend(extra);
    }
    Ok(argv)
}

fn new_id() -> String {
    let mut h = DefaultHasher::new();
    std::time::Instant::now().hash(&mut h);
    now_ms().hash(&mut h);
    std::thread::current().id().hash(&mut h);
    format!("{:012x}", h.finish() & 0xffff_ffff_ffff)
}

fn start_run(handle: &AppHandle, app: &Arc<App>, root: &str, s: &Script, args: &str) -> R<RunPub> {
    let argv = build_argv(s, args)?;
    let id = new_id();
    let p = RunPub {
        id: id.clone(),
        project: root.into(),
        script_id: s.id.clone(),
        title: s.name.clone(),
        cmd: pult_core::util::display_argv(&argv),
        where_: s.where_.clone(),
        args: args.trim().into(),
        status: "running".into(),
        code: None,
        started_at: now_ms(),
        ended_at: None,
        kind: "script".into(),
    };
    let settings = store::config().settings;
    let mut envs: Vec<(String, String)> =
        vec![("PYTHONIOENCODING".into(), "utf-8".into()), ("PYTHONUNBUFFERED".into(), "1".into()), ("PULT".into(), "1".into())];
    if settings.colors {
        envs.extend([("FORCE_COLOR".into(), "1".into()), ("CLICOLOR_FORCE".into(), "1".into()), ("NO_COLOR".into(), String::new())]);
    } else {
        envs.extend([("NO_COLOR".into(), "1".into()), ("FORCE_COLOR".into(), String::new())]);
    }
    let cwd = Path::new(root).join(if s.where_ == "." { "" } else { &s.where_ });
    lock(&app.inner).runs.insert(id.clone(), RunRec { p: p.clone(), log: String::new(), live: None });
    let (a1, id1) = (app.clone(), id.clone());
    let tx = lock(&app.log_tx).clone();
    let on_text = move |t: String| {
        if let Some(r) = lock(&a1.inner).runs.get_mut(&id1) {
            r.log.push_str(&t);
            if r.log.len() > 1_500_000 {
                let cut = r.log.len() - 1_000_000;
                let cut = (cut..r.log.len()).find(|i| r.log.is_char_boundary(*i)).unwrap_or(cut);
                r.log.drain(..cut);
            }
        }
        let _ = tx.send((id1.clone(), t));
    };
    let (a2, h2, id2) = (app.clone(), handle.clone(), id.clone());
    let on_exit = move |code: i32, live: Arc<Live>| {
        // дать сборщику вывода отправить хвост раньше, чем придёт «завершено»
        std::thread::sleep(Duration::from_millis(80));
        // запись истории (чтение и запись history.json) - после замка: под ним ждали бы диска
        // все команды окна, которым нужен список проектов или запусков
        let (pub_, hist) = {
            let mut g = lock(&a2.inner);
            let Some(r) = g.runs.get_mut(&id2) else { return };
            r.p.code = Some(code);
            r.p.status = if live.was_stopped() {
                "stopped"
            } else if code == 0 {
                "ok"
            } else {
                "fail"
            }
            .into();
            r.p.ended_at = Some(now_ms());
            r.live = None;
            let tail: Vec<&str> = r.log.lines().rev().take(80).collect();
            let tail = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
            let hist = HistEntry {
                id: r.p.id.clone(),
                project: r.p.project.clone(),
                script_id: r.p.script_id.clone(),
                title: r.p.title.clone(),
                cmd: r.p.cmd.clone(),
                where_: r.p.where_.clone(),
                args: r.p.args.clone(),
                status: r.p.status.clone(),
                code: r.p.code,
                started_at: r.p.started_at,
                ended_at: r.p.ended_at,
                kind: "script".into(),
                tail,
            };
            (r.p.clone(), hist)
        };
        store::push_history(hist);
        let _ = h2.emit("run", &pub_);
        let took = pub_.ended_at.unwrap_or(0).saturating_sub(pub_.started_at);
        let focused = h2.get_webview_window("main").and_then(|w| w.is_focused().ok()).unwrap_or(true);
        if took > 8000 && !focused && store::config().settings.notify {
            let ok = pub_.status == "ok";
            let _ = h2
                .notification()
                .builder()
                .title(format!("{} {}", if ok { "Готово:" } else { "Ошибка:" }, pub_.title))
                .body(format!("{} за {} с", if ok { "закончилось" } else { "упало" }, took / 1000))
                .show();
            if let Some(w) = h2.get_webview_window("main") {
                let _ = w.request_user_attention(Some(tauri::UserAttentionType::Informational));
            }
        }
    };
    match runner::spawn_live(&argv, &cwd, &envs, on_text, on_exit) {
        Ok(live) => {
            if let Some(r) = lock(&app.inner).runs.get_mut(&id) {
                r.live = Some(live);
            }
            let _ = handle.emit("run", &p);
            Ok(p)
        }
        Err(e) => {
            lock(&app.inner).runs.remove(&id);
            Err(format!("Не запустилось: {e}"))
        }
    }
}

#[tauri::command]
async fn run(handle: AppHandle, st: St<'_>, script_id: String, args: Option<String>, force: Option<bool>) -> R<RunPub> {
    let app = st.inner().clone();
    let (root, s) = find_script(&app, &script_id).ok_or("Скрипт не найден - обновите проект (F5)")?;
    if !force.unwrap_or(false) {
        if let Some(r) = lock(&app.inner).runs.values().find(|r| r.p.script_id == script_id && r.p.status == "running") {
            return Err(format!("RUNNING:{}", r.p.id));
        }
    }
    start_run(&handle, &app, &root, &s, args.as_deref().unwrap_or(""))
}

/// Перезапуск. Новые аргументы главнее старых: «Перезапустить» в панели не теряет то, что вы ввели.
#[tauri::command]
async fn restart(handle: AppHandle, st: St<'_>, run_id: String, args: Option<String>) -> R<RunPub> {
    let app = st.inner().clone();
    let (script_id, old_args, live) = {
        let g = lock(&app.inner);
        let r = g.runs.get(&run_id).ok_or("Запуск не найден")?;
        (r.p.script_id.clone(), r.p.args.clone(), r.live.clone())
    };
    if let Some(l) = live {
        // пауза - в том же блокирующем потоке: thread::sleep в async-команде занял бы поток
        // среды выполнения, на котором ждут ответа другие команды окна
        tauri::async_runtime::spawn_blocking(move || {
            l.stop();
            std::thread::sleep(Duration::from_millis(150));
        })
        .await
        .map_err(|e| e.to_string())?;
    }
    let (root, s) = find_script(&app, &script_id).ok_or("Скрипт больше не найден в проекте")?;
    start_run(&handle, &app, &root, &s, args.as_deref().unwrap_or(&old_args))
}

#[tauri::command]
async fn stop(st: St<'_>, run_id: String) -> R<bool> {
    let live = lock(&st.inner).runs.get(&run_id).and_then(|r| r.live.clone());
    match live {
        Some(l) => {
            tauri::async_runtime::spawn_blocking(move || l.stop()).await.map_err(|e| e.to_string())?;
            Ok(true)
        }
        None => Ok(false),
    }
}

#[tauri::command]
async fn stdin(st: St<'_>, run_id: String, text: Option<String>, eof: Option<bool>) -> R<()> {
    let live = lock(&st.inner).runs.get(&run_id).and_then(|r| r.live.clone()).ok_or("Процесс уже завершился")?;
    if eof.unwrap_or(false) {
        live.close_stdin();
        return Ok(());
    }
    let t = text.unwrap_or_default();
    if t.contains(['\r', '\n']) {
        return Err("Ввод - одной строкой".into());
    }
    live.write_line(&t).map_err(|e| format!("Процесс не принимает ввод: {e}"))?;
    let echo = format!("\x1b[36m› {t}\x1b[0m\n");
    if let Some(r) = lock(&st.inner).runs.get_mut(&run_id) {
        r.log.push_str(&echo);
    }
    // тем же сборщиком, что и вывод программы: прямое событие обгоняло ещё не отправленную пачку
    let _ = lock(&st.log_tx).send((run_id, echo));
    Ok(())
}

#[tauri::command]
async fn run_log(st: St<'_>, run_id: String) -> R<Value> {
    let g = lock(&st.inner);
    let r = g.runs.get(&run_id).ok_or("Запуск не найден")?;
    Ok(json!({ "run": r.p, "log": r.log }))
}

/// Убрать завершённые запуски: один (вкладка консоли) или все.
#[tauri::command]
async fn forget(st: St<'_>, run_id: Option<String>) -> R<()> {
    let mut g = lock(&st.inner);
    match run_id {
        Some(id) => {
            if g.runs.get(&id).is_some_and(|r| r.p.status != "running") {
                g.runs.remove(&id);
            }
        }
        None => g.runs.retain(|_, r| r.p.status == "running"),
    }
    Ok(())
}

#[tauri::command]
async fn history(path: String) -> R<Vec<HistEntry>> {
    let mut h: Vec<HistEntry> = store::history().into_iter().filter(|x| x.project.eq_ignore_ascii_case(&path)).collect();
    h.reverse();
    h.truncate(300);
    Ok(h)
}

#[tauri::command]
async fn history_clear(path: String) -> R<()> {
    store::clear_history(&path);
    Ok(())
}

// ---------- команды: настройки ----------
#[tauri::command]
async fn settings(handle: AppHandle, st: St<'_>, patch: Value) -> R<Settings> {
    let c = store::update_config(|c| {
        let mut v = serde_json::to_value(&c.settings).unwrap_or(Value::Null);
        if let (Some(o), Some(p)) = (v.as_object_mut(), patch.as_object()) {
            for (k, x) in p {
                if o.contains_key(k) {
                    o.insert(k.clone(), x.clone());
                }
            }
        }
        if let Ok(s) = serde_json::from_value(v) {
            c.settings = s;
        }
    });
    if !c.settings.auto_refresh {
        lock(&st.inner).watchers.clear();
    } else {
        let paths: Vec<String> = lock(&st.inner).projects.values().map(|p| p.data.path.clone()).collect();
        for p in paths {
            ensure_watch(&handle, st.inner(), &p);
        }
    }
    Ok(c.settings)
}

#[tauri::command]
async fn fav(path: String, id: String, on: bool) -> R<Vec<String>> {
    let c = store::update_config(|c| {
        let list = c.favorites.entry(path.clone()).or_default();
        list.retain(|x| x != &id);
        if on {
            list.push(id.clone());
        }
    });
    Ok(c.favorites.get(&path).cloned().unwrap_or_default())
}

#[tauri::command]
async fn recent(path: String, action: String) -> R<()> {
    store::update_config(|c| match action.as_str() {
        "remove" => c.recent.retain(|r| r.path != path),
        "pin" | "unpin" => {
            for r in &mut c.recent {
                if r.path == path {
                    r.pinned = action == "pin";
                }
            }
        }
        _ => {}
    });
    Ok(())
}

// ---------- команды: CI ----------
fn remote_of(app: &App, path: &str, name: &str) -> R<(String, pult_core::model::Remote)> {
    let g = lock(&app.inner);
    let p = g.projects.get(&key(path)).ok_or("Проект не открыт")?;
    let r = p.data.git.as_ref().and_then(|g| g.remotes.iter().find(|r| r.name == name)).cloned().ok_or("Не найден remote")?;
    Ok((p.data.path.clone(), r))
}

fn ci_cached<F>(cache_key: &str, max_age: u64, force: bool, f: F) -> Value
where
    F: FnOnce() -> R<Value>,
{
    let hit = store::ci_cache_get(cache_key);
    if let Some((at, v)) = &hit {
        if !force && now_ms().saturating_sub(*at) < max_age {
            return json!({ "data": v, "at": at, "cached": true });
        }
    }
    match f() {
        Ok(v) => {
            store::ci_cache_put(cache_key, &v);
            json!({ "data": v, "at": now_ms(), "cached": false })
        }
        Err(e) => match hit {
            Some((at, v)) => json!({ "data": v, "at": at, "cached": true, "stale": true, "error": e }),
            None => json!({ "error": e }),
        },
    }
}

#[tauri::command]
async fn ci_runs(st: St<'_>, path: String, remote: String, git_ref: Option<String>, force: Option<bool>) -> R<Value> {
    let (root, r) = remote_of(&st, &path, &remote)?;
    let k = format!(
        "runs|{}/{}|{}",
        r.host.as_deref().unwrap_or_default(),
        r.slug.as_deref().unwrap_or_default(),
        git_ref.as_deref().unwrap_or_default()
    );
    tauri::async_runtime::spawn_blocking(move || {
        ci_cached(&k, 45_000, force.unwrap_or(false), || {
            ci::runs(Path::new(&root), &r, git_ref.as_deref().filter(|x| !x.is_empty())).map(|v| json!(v))
        })
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn ci_jobs(st: St<'_>, path: String, remote: String, id: u64, done: Option<bool>, force: Option<bool>) -> R<Value> {
    let (root, r) = remote_of(&st, &path, &remote)?;
    let k = format!("jobs|{}/{}|{id}", r.host.as_deref().unwrap_or_default(), r.slug.as_deref().unwrap_or_default());
    let age = if done.unwrap_or(false) { 7 * 86_400_000 } else { 15_000 };
    tauri::async_runtime::spawn_blocking(move || {
        ci_cached(&k, age, force.unwrap_or(false), || ci::jobs(Path::new(&root), &r, id).map(|v| json!(v)))
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn ci_log(st: St<'_>, path: String, remote: String, job: u64) -> R<Value> {
    let (root, r) = remote_of(&st, &path, &remote)?;
    tauri::async_runtime::spawn_blocking(move || match ci::job_log(Path::new(&root), &r, job) {
        Ok(t) => json!({ "data": t, "at": now_ms() }),
        Err(e) => json!({ "error": e }),
    })
    .await
    .map_err(|e| e.to_string())
}

// ---------- команды: открыть ----------
fn target(root: &str, rel: Option<&str>) -> R<PathBuf> {
    let root = PathBuf::from(root);
    let Some(rel) = rel.filter(|r| !r.is_empty() && *r != ".") else {
        return Ok(root);
    };
    if is_unc(rel) || Path::new(rel).is_absolute() {
        return Err("Только пути внутри проекта".into());
    }
    let t = root.join(rel);
    if !inside(&root, &t) {
        return Err("За пределами проекта".into());
    }
    // ссылка внутри проекта может вести наружу или на сетевой путь
    resolve_inside(&root, &t).ok_or_else(|| "Файла нет или он ведёт за пределы проекта".into())
}

#[tauri::command]
async fn open(st: St<'_>, what: String, path: Option<String>, rel: Option<String>, line: Option<u32>, url: Option<String>) -> R<()> {
    if what == "url" {
        let u = url.unwrap_or_default();
        if !(u.starts_with("http://") || u.starts_with("https://")) || u.contains(['"', '\n', '\r', ' ']) {
            return Err("Открываю только адреса http и https".into());
        }
        return runner::open_default(&u).map_err(|e| e.to_string());
    }
    let root = lock(&st.inner).projects.get(&key(&path.unwrap_or_default())).map(|p| p.data.path.clone()).ok_or("Проект не открыт")?;
    let t = target(&root, rel.as_deref())?;
    let res = match what.as_str() {
        // проводник с файлом его запустит, а не покажет: файлы - только через «reveal» (/select)
        "explorer" if t.is_dir() => runner::open_default(&t.to_string_lossy()),
        "explorer" => runner::reveal(&t),
        "reveal" => runner::reveal(&t),
        "terminal" => runner::open_terminal(&t),
        "code" | "edit" => runner::open_in_editor(&t, line),
        "file" => {
            // программой по умолчанию - только папки и текстовые документы: readme.js или license.bat не запустим
            let ext = t.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default();
            if t.is_dir() || kb::get().str_list("docExtensions").contains(&ext) {
                runner::open_default(&t.to_string_lossy())
            } else {
                runner::open_in_editor(&t, None)
            }
        }
        _ => return Err("Неизвестное действие".into()),
    };
    res.map_err(|e| e.to_string())
}

fn main() {
    let (tx, rx) = mpsc::channel::<(String, String)>();
    let open_path = std::env::args().nth(1).filter(|a| !a.starts_with('-'));
    let app_state = Arc::new(App { inner: Mutex::new(Inner::default()), log_tx: Mutex::new(tx), open_path: Mutex::new(open_path) });

    // Тестовое окно (отладочная сборка с PULT_DEBUG_PORT) живёт рядом с обычным под своим
    // идентификатором: своя защита от второго экземпляра, свой файл места и размера окна, своя папка
    // WebView2. Иначе проверки перехватывают окно, в котором работает человек, и сдвигают его окно.
    let mut context = tauri::generate_context!();
    let test_port = std::env::var("PULT_DEBUG_PORT").ok().filter(|_| cfg!(debug_assertions));
    if let Some(port) = &test_port {
        let id = format!("{}.test-{port}", context.config().identifier);
        context.config_mut().identifier = id;
    }
    // Второй запуск отдаёт папку первому окну. Относительный путь («pult ui .») считается от папки,
    // где запустили второй, а не первый Пульт.
    let builder = tauri::Builder::default().plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
        tray::show_main(app);
        if let Some(p) = argv.get(1).filter(|a| !a.starts_with('-')) {
            let full = Path::new(&cwd).join(p);
            let _ = app.emit("open-path", full.to_string_lossy());
        }
    }));
    let app = builder
        // окно помнит место и размер, но не рамку: иначе старое сохранение возвращает системный
        // заголовок поверх своего (decorations(false)); видимость и «поверх окон» решает Пульт сам
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::all()
                        - tauri_plugin_window_state::StateFlags::DECORATIONS
                        - tauri_plugin_window_state::StateFlags::VISIBLE,
                )
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .on_window_event(|w, e| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = e {
                // без значка в трее спрятанное окно не вернуть ничем - тогда закрываем как обычно
                if store::config().settings.close_to_tray && w.app_handle().tray_by_id(tray::TRAY_ID).is_some() {
                    api.prevent_close();
                    let _ = w.hide();
                }
            }
        })
        .manage(app_state.clone())
        .invoke_handler(tauri::generate_handler![
            state,
            kb_reload,
            topic,
            scan_project,
            ai_map,
            pick,
            env_info,
            help,
            run,
            restart,
            stop,
            stdin,
            run_log,
            forget,
            history,
            history_clear,
            settings,
            fav,
            recent,
            ci_runs,
            ci_jobs,
            ci_log,
            open,
            trust_repo,
            tray::scenarios,
            tray::scenario_save,
            tray::scenario_delete,
            tray::scenario_run,
            tray::scenario_stop,
            tray::scenarios_running,
            tray::window_set,
            tray::window_bg,
            tray::mouse_down
        ])
        .setup(move |app| {
            // Ключи WebView2. Свои ключи заменяют ключи wry целиком, поэтому его три (без мини-меню,
            // без SmartScreen, автозапуск звука) повторены здесь. Своё одно: сетевая служба внутри
            // процесса браузера, а не отдельным процессом - окно не ходит в сеть, а процесс стоил
            // 25-35 МБ рабочего набора (замер в docs/optimizations.md).
            #[allow(unused_mut)]
            let mut browser_args = String::from(
                "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required \
                 --enable-features=NetworkServiceInProcess2",
            );
            #[allow(unused_mut)]
            let mut win = tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("index.html".into()))
                .title("Пульт")
                .inner_size(1320.0, 900.0)
                .min_inner_size(280.0, 200.0)
                // свой заголовок в ui/titlebar.js: у системного двойной щелчок по значку закрывает окно
                .decorations(false)
                .shadow(true)
                .center();
            // порт отладки - только в отладочной сборке и только по явной просьбе: в релизе этого кода нет
            #[cfg(debug_assertions)]
            if let Ok(port) = std::env::var("PULT_DEBUG_PORT") {
                browser_args.push_str(&format!(" --remote-debugging-port={port}"));
                win = win.data_directory(std::env::temp_dir().join(format!("pult-test-webview-{port}")));
            }
            let w = win.additional_browser_args(&browser_args).build()?;
            tray::initial_bg(&w);
            // значок в трее с меню сценариев; без трея (старая оболочка Windows) Пульт работает как раньше
            if let Err(e) = tray::build_tray(app.handle()) {
                eprintln!("трей не создан: {e}");
            }
            // сборщик вывода: пачка не позже 50 мс от первого куска вместо события на каждую строку.
            // Срок - от первого куска пачки, а не от последнего: программа, которая пишет чаще раза
            // в 50 мс (полоска хода, журнал сервера), иначе молчала до 64 КБ или до самого конца.
            let h = app.handle().clone();
            spawn_named("pult-log", move || {
                const TICK: Duration = Duration::from_millis(50);
                let mut pending: HashMap<String, String> = HashMap::new();
                let mut since: Option<std::time::Instant> = None;
                loop {
                    let wait = since.map_or(Duration::from_secs(3600), |s| TICK.saturating_sub(s.elapsed()));
                    match rx.recv_timeout(wait) {
                        Ok((id, t)) => {
                            pending.entry(id).or_default().push_str(&t);
                            let first = *since.get_or_insert_with(std::time::Instant::now);
                            if first.elapsed() < TICK && pending.values().map(String::len).sum::<usize>() < 64 * 1024 {
                                continue;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(_) => return,
                    }
                    since = None;
                    for (id, text) in pending.drain() {
                        let _ = h.emit("log", json!({ "runId": id, "text": text }));
                    }
                }
            });
            Ok(())
        })
        .build(context)
        .expect("не удалось запустить окно");

    app.run(move |_h, ev| {
        if let RunEvent::Exit = ev {
            // закрыли окно - останавливаем то, что запускали из Пульта
            let lives: Vec<Arc<Live>> = lock(&app_state.inner).runs.values().filter_map(|r| r.live.clone()).collect();
            for l in lives {
                l.stop();
            }
        }
    });
}
