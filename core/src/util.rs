/// Регулярное выражение, скомпилированное один раз на весь запуск.
#[macro_export]
macro_rules! re {
    ($s:expr) => {{
        static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        // clippy видит Regex::new внутри цикла вызывающего кода, но здесь он выполняется
        // один раз за весь запуск: результат лежит в static OnceLock
        #[allow(clippy::regex_creation_in_loops)]
        let r: &'static regex::Regex = R.get_or_init(|| regex::Regex::new($s).expect("регулярное выражение в коде"));
        r
    }};
}

/// Регулярное выражение из данных (база знаний) - компилируется один раз на текст выражения.
pub(crate) fn cached_re(pattern: &str) -> Option<regex::Regex> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static C: OnceLock<Mutex<HashMap<String, Option<regex::Regex>>>> = OnceLock::new();
    let m = C.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(r) = m.lock().ok().and_then(|g| g.get(pattern).cloned()) {
        return r;
    }
    let r = regex::Regex::new(pattern).ok();
    if let Ok(mut g) = m.lock() {
        g.insert(pattern.to_string(), r.clone());
    }
    r
}

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub(crate) fn mtime_ms(p: &Path) -> u64 {
    fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Первые `max` байт файла как UTF-8 (с заменой битых символов), без BOM.
/// Ссылка на сетевой путь не читается: открыть её - значит обратиться к \\host\share.
pub(crate) fn read_head(p: &Path, max: usize) -> Option<String> {
    if is_link(p) && fs::read_link(p).map(|t| is_unc(&t.to_string_lossy())).unwrap_or(true) {
        return None;
    }
    let mut f = fs::File::open(p).ok()?;
    let mut buf = Vec::with_capacity(max.min(1 << 20));
    f.by_ref().take(max as u64).read_to_end(&mut buf).ok()?;
    let s = String::from_utf8_lossy(&buf).into_owned();
    Some(s.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(s))
}

pub(crate) fn read_text(p: &Path) -> Option<String> {
    read_head(p, 1_000_000)
}

pub(crate) fn key_of(s: &str) -> String {
    let mut h = sha1_smol::Sha1::new();
    h.update(s.to_lowercase().as_bytes());
    h.digest().to_string()[..16].to_string()
}

pub(crate) fn id_of(parts: &[&str]) -> String {
    let mut h = sha1_smol::Sha1::new();
    h.update(parts.join("|").as_bytes());
    h.digest().to_string()[..12].to_string()
}

/// Путь относительно корня, всегда через «/»; «.» - сам корень.
pub fn rel(root: &Path, p: &Path) -> String {
    match p.strip_prefix(root) {
        Ok(r) if r.as_os_str().is_empty() => ".".into(),
        Ok(r) => r.to_string_lossy().replace('\\', "/"),
        Err(_) => p.to_string_lossy().replace('\\', "/"),
    }
}

/// Сетевые пути (\\host\share) не трогаем: обращение к ним отдаёт хеш пароля Windows.
pub fn is_unc(p: &str) -> bool {
    let s = p.trim_start_matches('"');
    s.starts_with("\\\\") || s.starts_with("//")
}

/// Путь лежит внутри корня (после нормализации «..»).
pub fn inside(root: &Path, p: &Path) -> bool {
    let norm = |x: &Path| -> PathBuf {
        let mut out = PathBuf::new();
        for c in x.components() {
            match c {
                std::path::Component::ParentDir => {
                    out.pop();
                }
                std::path::Component::CurDir => {}
                other => out.push(other.as_os_str()),
            }
        }
        out
    };
    let r = norm(root);
    let q = norm(p);
    let rs = r.to_string_lossy().to_lowercase();
    let qs = q.to_string_lossy().to_lowercase();
    qs == rs || qs.starts_with(&(rs.trim_end_matches(['\\', '/']).to_string() + std::path::MAIN_SEPARATOR_STR))
}

/// Как показать список аргументов человеку: с кавычками там, где пробелы.
pub fn display_argv(argv: &[String]) -> String {
    argv.iter()
        .map(|a| if a.is_empty() || a.contains([' ', '\t', '&', '|', '(', ')', '^', '<', '>']) { format!("\"{a}\"") } else { a.clone() })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Содержимое папки без ссылок (symlink, junction): по ссылке разбор ушёл бы за пределы
/// проекта или на сетевой путь.
pub fn ls(dir: &Path) -> Vec<fs::DirEntry> {
    fs::read_dir(dir).map(|r| r.flatten().filter(|e| !e.file_type().is_ok_and(|t| t.is_symlink())).collect()).unwrap_or_default()
}

/// Путь - ссылка (symlink или junction; на Windows `is_symlink` верно для обеих).
pub(crate) fn is_link(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink())
}

/// Путь внутри корня и после разрешения ссылок: ни одна ссылка по дороге не ведёт на сетевой
/// путь, а настоящий путь не выходит из настоящего корня. None - путь не годится или его нет.
pub fn resolve_inside(root: &Path, p: &Path) -> Option<PathBuf> {
    if is_unc(&p.to_string_lossy()) || !inside(root, p) {
        return None;
    }
    // ссылки проверяем до canonicalize: тот открыл бы сетевой путь сам
    let mut cur = root.to_path_buf();
    for c in p.strip_prefix(root).ok()?.components() {
        cur.push(c);
        if is_link(&cur) && fs::read_link(&cur).map(|t| is_unc(&t.to_string_lossy())).unwrap_or(true) {
            return None;
        }
    }
    // canonicalize на Windows пишет \\?\C:\... или \\?\UNC\host\share\...
    let strip = |x: PathBuf| -> Option<PathBuf> {
        let s = x.to_string_lossy().into_owned();
        let low = s.to_ascii_lowercase();
        if low.starts_with(r"\\?\unc\") {
            return None;
        }
        let s = s.strip_prefix(r"\\?\").unwrap_or(&s);
        (!is_unc(s)).then(|| PathBuf::from(s))
    };
    let real = strip(fs::canonicalize(p).ok()?)?;
    let real_root = strip(fs::canonicalize(root).ok()?)?;
    inside(&real_root, &real).then(|| p.to_path_buf())
}

pub(crate) fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Строчная первая буква, если это не аббревиатура («TypeScript» оставляем, «SQL» тоже).
pub(crate) fn lower_first(s: &str) -> String {
    let mut it = s.chars();
    match (it.next(), it.next()) {
        (Some(a), Some(b))
            if a.is_uppercase()
                && b.is_lowercase()
                && !s.split_whitespace().next().unwrap_or("").chars().skip(1).any(char::is_uppercase) =>
        {
            a.to_lowercase().collect::<String>() + &s[a.len_utf8()..]
        }
        _ => s.to_string(),
    }
}

/// Зажата ли сейчас кнопка мыши (любая из трёх) - по всей системе, не только в окне Пульта.
/// Окно не узнаёт, отпустили ли край при растягивании: событий мыши на рамке у страницы нет.
/// Виджет включается только после того, как кнопку отпустили, иначе он борется с рукой.
#[cfg(windows)]
pub fn mouse_down() -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_MBUTTON, VK_RBUTTON};
    // старший бит - кнопка нажата прямо сейчас
    [VK_LBUTTON, VK_RBUTTON, VK_MBUTTON].iter().any(|&k| unsafe { GetAsyncKeyState(k as i32) } as u16 & 0x8000 != 0)
}
#[cfg(not(windows))]
pub fn mouse_down() -> bool {
    false
}
