//! Сценарии (свои цепочки команд), значок в трее с их меню и управление размером окна.
//! Тексты меню - из knowledge/ui.json (раздел tray), а не из кода.
use super::{find_project, lock, start_run, App, R};
use pult_core::kb;
use pult_core::model::{Project, Script};
use pult_core::store::{self, Scenario};
use pult_core::util::now_ms;
use serde_json::json;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, LogicalSize, Manager};
use tauri_plugin_notification::NotificationExt;

pub const TRAY_ID: &str = "pult";

fn text(key: &str) -> String {
    kb::get().ui["tray"][key].as_str().unwrap_or(key).to_string()
}

/// Идущие сценарии: id -> флаг «остановить». Второй запуск того же сценария не начинается.
fn running() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static R: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

/// Пометка «сценарий идёт» снимается при любом конце его потока: и при панике, и если поток
/// не запустился. Иначе сценарий до перезапуска Пульта отвечал бы «уже идёт». Снимает только
/// свою пометку (тот же флаг): после `finish` тот же сценарий мог уже начаться заново.
struct RunningMark(String, Arc<AtomicBool>);

impl Drop for RunningMark {
    fn drop(&mut self) {
        let mut g = lock(running());
        if g.get(&self.0).is_some_and(|f| Arc::ptr_eq(f, &self.1)) {
            g.remove(&self.0);
        }
    }
}

/// Скрипт шага: по id, а если проект поменялся и id другой - по имени.
fn resolve<'a>(p: &'a Project, step: &store::Step) -> Option<&'a Script> {
    p.scripts.iter().find(|s| s.id == step.script_id).or_else(|| p.scripts.iter().find(|s| !step.name.is_empty() && s.name == step.name))
}

fn notify(handle: &AppHandle, title: String, body: String) {
    if store::config().settings.notify {
        let _ = handle.notification().builder().title(title).body(body).show();
    }
}

/// Запустить сценарий в фоне: шаги по очереди, каждый ждёт конца предыдущего.
pub fn run_scenario(handle: AppHandle, app: Arc<App>, sc: Scenario) -> R<()> {
    if sc.steps.is_empty() {
        return Err(text("noSteps"));
    }
    let stop = Arc::new(AtomicBool::new(false));
    {
        let mut g = lock(running());
        if g.contains_key(&sc.id) {
            return Err(text("already"));
        }
        g.insert(sc.id.clone(), stop.clone());
    }
    // пометка «идёт» в меню трея: и при запуске из окна, и при конце сценария, а не только из трея
    refresh_tray(&handle);
    let mark = RunningMark(sc.id.clone(), stop.clone());
    super::spawn_named("pult-scenario", move || {
        let _mark = mark;
        let started = now_ms();
        let total = sc.steps.len();
        let emit = |step: usize, status: &str, msg: &str, run_id: &str| {
            let _ = handle.emit(
                "scenario",
                json!({ "id": sc.id, "name": sc.name, "step": step, "total": total, "status": status, "msg": msg, "runId": run_id }),
            );
        };
        let finish = |status: &str, msg: String| {
            lock(running()).remove(&sc.id);
            refresh_tray(&handle);
            emit(total, status, &msg, "");
            // часы Windows могли перевести назад за время сценария
            let took = now_ms().saturating_sub(started) / 1000;
            notify(
                &handle,
                format!("{}: {}", text("scenario"), sc.name),
                if status == "ok" { format!("{} {took} с", text("done")) } else { msg },
            );
        };
        let project = match find_project(&handle, &app, &sc.project) {
            Ok(p) => p,
            Err(e) => return finish("fail", e),
        };
        let mut failed: Option<String> = None;
        for (i, step) in sc.steps.iter().enumerate() {
            if stop.load(Ordering::Relaxed) {
                return finish("stopped", text("stopped"));
            }
            let Some(s) = resolve(&project, step) else {
                let m = format!("{} «{}»", text("missing"), step.name);
                if sc.stop_on_error {
                    return finish("fail", m);
                }
                failed.get_or_insert(m);
                continue;
            };
            let run = match start_run(&handle, &app, &project.path, s, &step.args) {
                Ok(r) => r,
                Err(e) => {
                    if sc.stop_on_error {
                        return finish("fail", e);
                    }
                    failed.get_or_insert(e);
                    continue;
                }
            };
            emit(i, "running", &s.name, &run.id);
            // ждать конца шага; «Стоп» сценария останавливает и идущий шаг
            let status = loop {
                std::thread::sleep(Duration::from_millis(250));
                let g = lock(&app.inner);
                let Some(r) = g.runs.get(&run.id) else { break "fail".to_string() };
                if r.p.status != "running" {
                    break r.p.status.clone();
                }
                if stop.load(Ordering::Relaxed) {
                    if let Some(l) = r.live.clone() {
                        drop(g);
                        l.stop();
                    }
                }
            };
            if status == "stopped" || stop.load(Ordering::Relaxed) {
                return finish("stopped", text("stopped"));
            }
            if status != "ok" {
                let m = format!("{} {} «{}»", text("failedAt"), i + 1, s.name);
                if sc.stop_on_error {
                    return finish("fail", m);
                }
                failed.get_or_insert(m);
            }
        }
        match failed {
            Some(m) => finish("fail", m),
            None => finish("ok", String::new()),
        }
    });
    Ok(())
}

// ---------- команды окна ----------

#[tauri::command]
pub async fn scenarios() -> R<Vec<Scenario>> {
    Ok(store::config().scenarios)
}

#[tauri::command]
pub async fn scenario_save(handle: AppHandle, scenario: Scenario) -> R<Scenario> {
    if scenario.name.trim().is_empty() {
        return Err(text("noName"));
    }
    if !Path::new(&scenario.project).is_dir() {
        return Err(text("noProject"));
    }
    let s = store::save_scenario(scenario);
    refresh_tray(&handle);
    Ok(s)
}

#[tauri::command]
pub async fn scenario_delete(handle: AppHandle, id: String) -> R<()> {
    store::delete_scenario(&id);
    refresh_tray(&handle);
    Ok(())
}

#[tauri::command]
pub async fn scenario_run(handle: AppHandle, st: tauri::State<'_, Arc<App>>, id: String) -> R<()> {
    let sc = store::config().scenarios.into_iter().find(|s| s.id == id).ok_or(text("notFound"))?;
    run_scenario(handle, st.inner().clone(), sc)
}

#[tauri::command]
pub async fn scenario_stop(id: String) -> R<bool> {
    Ok(lock(running()).get(&id).map(|f| f.store(true, Ordering::Relaxed)).is_some())
}

#[tauri::command]
pub async fn scenarios_running() -> R<Vec<String>> {
    Ok(lock(running()).keys().cloned().collect())
}

/// Размер окна в логических пикселях, по центру экрана, поверх всех окон или нет.
/// Так работают «Рекомендованный размер», «Свернуть в виджет» и «Развернуть».
#[tauri::command]
pub async fn window_set(handle: AppHandle, width: f64, height: f64, center: Option<bool>, on_top: Option<bool>) -> R<()> {
    let w = handle.get_webview_window("main").ok_or("Нет окна")?;
    let _ = w.unmaximize();
    let (width, height) = (width.clamp(280.0, 4000.0), height.clamp(200.0, 3000.0));
    w.set_size(LogicalSize::new(width, height)).map_err(|e| e.to_string())?;
    if center.unwrap_or(false) {
        let _ = w.center();
    } else {
        keep_on_screen(&w, width, height);
    }
    if let Some(t) = on_top {
        let _ = w.set_always_on_top(t);
    }
    Ok(())
}

/// Окно с новым размером остаётся в рабочей области своего экрана (без панели задач): виджет у
/// нижнего края растёт вверх, а не уходит под панель задач; «Вернуть прежний размер» у правого края
/// сдвигает окно влево. Считается в физических пикселях: set_size ещё не применён, поэтому новый
/// размер - логический размер, умноженный на масштаб экрана, а не outer_size().
fn keep_on_screen(w: &tauri::WebviewWindow, width: f64, height: f64) {
    let (Ok(Some(m)), Ok(pos), Ok(inner)) = (w.current_monitor(), w.outer_position(), w.inner_position()) else { return };
    let sf = w.scale_factor().unwrap_or(1.0);
    let area = m.work_area();
    let (aw, ah) = (area.size.width as i32, area.size.height as i32);
    let (iw, ih) = ((width * sf).round() as i32, (height * sf).round() as i32);
    // невидимые края тени (слева, справа, снизу) не считаются: на экране держится сама страница
    let nx = fit_axis(inner.x, iw, area.position.x, aw);
    let ny = fit_axis(inner.y, ih, area.position.y, ah);
    if (nx, ny) != (inner.x, inner.y) {
        let _ = w.set_position(tauri::PhysicalPosition::new(pos.x + nx - inner.x, pos.y + ny - inner.y));
    }
}

/// Начало окна по одной оси: `p` - где оно сейчас, `size` - новый размер, `start` и `len` - рабочая
/// область экрана. Сдвиг - только когда окно вылезло за правый или нижний край из-за нового
/// размера; окно больше экрана прижимается к началу области; окно, которое человек сам задвинул
/// за левый или верхний край, остаётся где было.
fn fit_axis(p: i32, size: i32, start: i32, len: i32) -> i32 {
    if p + size > start + len {
        (start + len - size).max(start).min(p)
    } else {
        p
    }
}

// ---------- трей ----------

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn tray_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    menu.append(&MenuItem::with_id(app, "open", text("open"), true, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    let list: Vec<Scenario> = store::config().scenarios.into_iter().filter(|s| s.in_tray).collect();
    if list.is_empty() {
        menu.append(&MenuItem::with_id(app, "none", text("empty"), false, None::<&str>)?)?;
    }
    let busy = lock(running()).keys().cloned().collect::<Vec<_>>();
    for s in list {
        let folder = Path::new(&s.project).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        let mark = if busy.contains(&s.id) { format!(" ({})", text("runningMark")) } else { String::new() };
        menu.append(&MenuItem::with_id(app, format!("sc:{}", s.id), format!("{} · {folder}{mark}", s.name), true, None::<&str>)?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "quit", text("quit"), true, None::<&str>)?)?;
    Ok(menu)
}

/// Пересобрать меню: сценарии добавились, удалились, начались или закончились.
pub fn refresh_tray(app: &AppHandle) {
    if let (Some(t), Ok(m)) = (app.tray_by_id(TRAY_ID), tray_menu(app)) {
        let _ = t.set_menu(Some(m));
    }
}

pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let mut b = TrayIconBuilder::with_id(TRAY_ID).tooltip("Пульт").menu(&tray_menu(app)?).show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon() {
        b = b.icon(icon.clone());
    }
    b.on_menu_event(|app, ev| match ev.id().as_ref() {
        "open" => show_main(app),
        "quit" => app.exit(0),
        id => {
            if let Some(sid) = id.strip_prefix("sc:") {
                let state = app.state::<Arc<App>>().inner().clone();
                if let Some(sc) = store::config().scenarios.into_iter().find(|s| s.id == sid) {
                    let name = sc.name.clone();
                    // меню пересобирает сам run_scenario: при запуске и при конце
                    match run_scenario(app.clone(), state, sc) {
                        Ok(()) => notify(app, format!("{}: {name}", text("scenario")), text("started")),
                        Err(e) => notify(app, format!("{}: {name}", text("scenario")), e),
                    }
                }
            }
        }
    })
    .on_tray_icon_event(|tray, ev| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
            show_main(tray.app_handle());
        }
    })
    .build(app)?;
    Ok(())
}

/// Цвет фона окна под тему страницы: без него при изменении размера и открытии меню
/// WebView2 на мгновение показывает свой белый фон - белые полосы в тёмной теме.
#[tauri::command]
pub async fn window_bg(handle: AppHandle, color: String) -> R<()> {
    let w = handle.get_webview_window("main").ok_or("Нет окна")?;
    let c = parse_hex(&color).ok_or("Цвет - #rrggbb")?;
    w.set_background_color(Some(c)).map_err(|e| e.to_string())
}

/// `#rrggbb` в цвет. Только шесть шестнадцатеричных цифр: from_str_radix сам по себе принял бы
/// и знак («#+fffff»).
fn parse_hex(s: &str) -> Option<tauri::window::Color> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some(tauri::window::Color(byte(0)?, byte(2)?, byte(4)?, 255))
}

/// Фон до первой отрисовки страницы - по теме системы (страница уточнит его через window_bg).
pub fn initial_bg(w: &tauri::WebviewWindow) {
    let dark = matches!(w.theme(), Ok(tauri::Theme::Dark));
    let _ = w.set_background_color(parse_hex(if dark { "#0f1117" } else { "#f3f0e9" }));
}

/// Зажата ли кнопка мыши: виджет ждёт, пока край окна отпустят (pult_core::util::mouse_down).
#[tauri::command]
pub async fn mouse_down() -> bool {
    pult_core::util::mouse_down()
}

#[cfg(test)]
mod tests {
    use super::{fit_axis, parse_hex};

    fn rgba(s: &str) -> Option<(u8, u8, u8, u8)> {
        parse_hex(s).map(|c| (c.0, c.1, c.2, c.3))
    }

    #[test]
    fn parse_hex_reads_rrggbb_with_spaces_around() {
        assert_eq!(rgba(" #0f1117 "), Some((0x0f, 0x11, 0x17, 255)));
    }

    #[test]
    fn parse_hex_rejects_sign_short_form_and_missing_hash() {
        for bad in ["#+fffff", "#-00000", "#fff", "0f1117", "#0f11171", "#0g1117", "#ффф"] {
            assert_eq!(rgba(bad), None, "{bad}");
        }
    }

    // рабочая область экрана: x от 0, ширина 1920
    #[test]
    fn fit_axis_keeps_window_that_fits() {
        assert_eq!(fit_axis(100, 800, 0, 1920), 100);
    }

    #[test]
    fn fit_axis_moves_window_back_from_right_edge() {
        assert_eq!(fit_axis(1500, 800, 0, 1920), 1120);
    }

    #[test]
    fn fit_axis_pins_window_larger_than_screen_to_area_start() {
        assert_eq!(fit_axis(300, 2500, 0, 1920), 0);
    }

    #[test]
    fn fit_axis_leaves_window_pushed_past_left_edge_where_it_is() {
        assert_eq!(fit_axis(-200, 2500, 0, 1920), -200);
    }

    #[test]
    fn fit_axis_respects_area_offset_of_second_monitor() {
        // второй экран справа: область с 1920, ширина 1280
        assert_eq!(fit_axis(2900, 400, 1920, 1280), 2800);
    }
}
