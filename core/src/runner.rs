//! Запуск процессов. Никакой оболочки: программа и аргументы передаются списком,
//! поэтому имя файла или аргумент не может стать второй командой.
//! Остановка - через объект задания Windows: гаснет всё дерево потомков.
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ---------- поиск программы ----------
static WHICH: OnceLock<Mutex<HashMap<String, Option<PathBuf>>>> = OnceLock::new();

fn pathexts() -> Vec<String> {
    if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .filter(|s| !s.is_empty())
            .map(|s| s.to_lowercase())
            .filter(|s| [".com", ".exe", ".bat", ".cmd"].contains(&s.as_str()))
            .collect()
    } else {
        vec![String::new()]
    }
}

/// Полный путь к программе по имени (с учётом .cmd/.bat на Windows). Результат кэшируется.
pub fn which(name: &str) -> Option<PathBuf> {
    if name.contains(['/', '\\']) {
        let p = PathBuf::from(name);
        return p.is_file().then_some(p);
    }
    let key = name.to_lowercase();
    let cache = WHICH.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(v) = cache.lock().ok().and_then(|c| c.get(&key).cloned()) {
        return v;
    }
    let exts = pathexts();
    let has_ext = exts.iter().any(|e| !e.is_empty() && key.ends_with(e.as_str()));
    let mut found = None;
    if let Some(path) = std::env::var_os("PATH") {
        'outer: for dir in std::env::split_paths(&path) {
            // bash из System32 запускает Linux (WSL), а не Git Bash - его не берём
            if key == "bash" && dir.to_string_lossy().to_lowercase().contains("system32") {
                continue;
            }
            if has_ext {
                let p = dir.join(name);
                if p.is_file() {
                    found = Some(p);
                    break;
                }
            }
            for e in &exts {
                let p = dir.join(format!("{name}{e}"));
                if p.is_file() {
                    found = Some(p);
                    break 'outer;
                }
            }
        }
    }
    if found.is_none() && key == "bash" {
        found = git_bash();
    }
    if let Ok(mut c) = cache.lock() {
        c.insert(key, found.clone());
    }
    found
}

pub fn forget_which() {
    if let Some(c) = WHICH.get() {
        if let Ok(mut c) = c.lock() {
            c.clear();
        }
    }
}

pub(crate) fn git_bash() -> Option<PathBuf> {
    if !cfg!(windows) {
        return Some(PathBuf::from("bash"));
    }
    let mut cands = Vec::new();
    for v in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(p) = std::env::var(v) {
            cands.push(PathBuf::from(p).join("Git").join("bin").join("bash.exe"));
        }
    }
    if let Ok(p) = std::env::var("LOCALAPPDATA") {
        cands.push(PathBuf::from(p).join("Programs").join("Git").join("bin").join("bash.exe"));
    }
    cands.into_iter().find(|p| p.is_file())
}

fn command(argv: &[String], cwd: &Path) -> io::Result<Command> {
    let prog = argv.first().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "пустая команда"))?;
    let resolved = which(prog).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("не найдена программа «{prog}»")))?;
    let mut c = Command::new(resolved);
    c.args(&argv[1..]).current_dir(cwd);
    #[cfg(windows)]
    c.creation_flags(CREATE_NO_WINDOW);
    Ok(c)
}

// ---------- декодирование вывода ----------
/// Склеивает куски: буква, разрезанная между кусками, не превращается в «�».
/// Если вывод не UTF-8 (консольные программы Windows пишут в cp866) - перекодирует.
///
/// Кодировка потока запоминается, как только появилось надёжное свидетельство.
/// Без этого cp866 ломался на кусках, которые случайно оказываются допустимым UTF-8:
/// «р»-«я» в cp866 - это 0xE0-0xEF, начало трёхбайтового символа UTF-8, а «а»-«п»
/// (0xA0-0xAF) - его продолжение. «тем» (E2 A5 AC) или «р|аз», разрезанное после «р»,
/// склеивались в один чужой символ вместо трёх русских букв.
///
/// Свидетельства выбраны так, чтобы не путаться:
/// - UTF-8: кириллица в UTF-8 (U+0400-U+04FF, байты D0/D1 + продолжение). Любой другой
///   двухбайтовый символ не годится: псевдографика cp866 (0xC1-0xDF) перед русской буквой
///   (0x80-0xBF) - тоже допустимый UTF-8. Трёхбайтовые не годятся тем более: «тем» в cp866
///   выглядит как один из них, а «✓» в UTF-8 - как три буквы cp866;
/// - cp866: кусок с байтами, недопустимыми в UTF-8, которые в cp866 дают русские буквы, и
///   до этого в потоке не было ни одного допустимого многобайтового символа UTF-8.
///
/// Без надёжного свидетельства кодировка решается для каждого куска отдельно: битый кусок
/// с символами UTF-8 до порчи - UTF-8 (битое - «�»), без них - cp866.
///
/// Чего декодер не умеет: кусок cp866, который целиком оказался допустимым UTF-8 без
/// кириллицы («тем», «═Итог»), до первого свидетельства показывается как UTF-8, то есть
/// мусором. Придержать такой кусок до свидетельства нельзя: поток UTF-8 с одними «✓» и
/// значками ждал бы свидетельства вечно, и живой вывод стоял бы.
pub struct Decoder {
    pending: Vec<u8>,
    mode: Mode,
    /// в потоке уже был допустимый многобайтовый символ UTF-8 - запирать cp866 нельзя
    seen_multibyte: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Unknown,
    Utf8,
    Cp866,
}

fn cp866(bytes: &[u8]) -> String {
    encoding_rs::IBM866.decode(bytes).0.into_owned()
}

fn has_cyrillic(s: &str) -> bool {
    s.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c))
}

/// Длина незаконченного символа UTF-8 в конце (0 - его нет).
fn incomplete_tail(b: &[u8]) -> usize {
    (1..=3.min(b.len()))
        .find(|&k| matches!(std::str::from_utf8(&b[b.len() - k..]), Err(e) if e.valid_up_to() == 0 && e.error_len().is_none()))
        .unwrap_or(0)
}

impl Decoder {
    pub fn new() -> Self {
        Self { pending: Vec::new(), mode: Mode::Unknown, seen_multibyte: false }
    }

    pub fn push(&mut self, chunk: &[u8]) -> String {
        if self.mode == Mode::Cp866 {
            return cp866(chunk);
        }
        self.pending.extend_from_slice(chunk);
        let (valid, broken) = match std::str::from_utf8(&self.pending) {
            Ok(_) => (self.pending.len(), false),
            // хвост - начало символа, продолжение придёт следующим куском
            Err(e) if e.error_len().is_none() => (e.valid_up_to(), false),
            Err(e) => (e.valid_up_to(), true),
        };
        // valid байт проверены from_utf8 выше
        let head = std::str::from_utf8(&self.pending[..valid]).unwrap_or_default();
        let head_multibyte = !head.is_ascii();
        if has_cyrillic(head) {
            self.mode = Mode::Utf8;
        }
        if !broken {
            self.seen_multibyte |= head_multibyte;
            let out = head.to_string();
            self.pending.drain(..valid);
            return out;
        }
        if self.mode == Mode::Utf8 || head_multibyte {
            // в UTF-8 битая последовательность - порча; незаконченный символ в конце ждёт продолжения
            self.seen_multibyte |= head_multibyte;
            let cut = self.pending.len() - incomplete_tail(&self.pending);
            let out = String::from_utf8_lossy(&self.pending[..cut]).into_owned();
            self.pending.drain(..cut);
            return out;
        }
        let s = cp866(&self.pending);
        if has_cyrillic(&s) && !self.seen_multibyte {
            self.mode = Mode::Cp866;
        }
        self.pending.clear();
        s
    }

    pub fn finish(&mut self) -> String {
        let out = match (self.mode, std::str::from_utf8(&self.pending)) {
            (_, Ok(s)) => s.to_string(),
            (Mode::Utf8, Err(_)) => String::from_utf8_lossy(&self.pending).into_owned(),
            (_, Err(_)) if self.seen_multibyte => String::from_utf8_lossy(&self.pending).into_owned(),
            // поток оборвался на «начале символа»: в cp866 это целая буква («р»-«я»)
            (Mode::Unknown | Mode::Cp866, Err(_)) => cp866(&self.pending),
        };
        self.pending.clear();
        out
    }
}
impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

// ---------- объект задания (Windows) ----------
#[cfg(windows)]
mod job {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW, TerminateJobObject};

    pub struct Job(HANDLE);
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}
    impl Job {
        pub fn for_process(process: HANDLE) -> Option<Job> {
            unsafe {
                let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if h.is_null() {
                    return None;
                }
                if AssignProcessToJobObject(h, process) == 0 {
                    CloseHandle(h);
                    return None;
                }
                Some(Job(h))
            }
        }
        pub fn kill(&self) -> bool {
            unsafe { TerminateJobObject(self.0, 1) != 0 }
        }
    }
    impl Drop for Job {
        fn drop(&mut self) {
            // без KILL_ON_JOB_CLOSE: закрытие не убивает тех, кто должен жить после скрипта
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

// ---------- живой процесс ----------
pub struct Live {
    pub pid: u32,
    stdin: Mutex<Option<ChildStdin>>,
    #[cfg(windows)]
    job: Option<job::Job>,
    stopped: Mutex<bool>,
}

impl Live {
    pub fn write_line(&self, text: &str) -> io::Result<()> {
        let mut g = self.stdin.lock().map_err(|_| io::Error::other("lock"))?;
        let s = g.as_mut().ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "ввод закрыт"))?;
        let eol = if cfg!(windows) { "\r\n" } else { "\n" };
        s.write_all(format!("{text}{eol}").as_bytes())?;
        s.flush()
    }
    pub fn close_stdin(&self) {
        if let Ok(mut g) = self.stdin.lock() {
            g.take();
        }
    }
    pub fn was_stopped(&self) -> bool {
        self.stopped.lock().map(|g| *g).unwrap_or(false)
    }
    /// Остановить процесс и всех его потомков.
    pub fn stop(&self) {
        if let Ok(mut g) = self.stopped.lock() {
            *g = true;
        }
        #[cfg(windows)]
        {
            if self.job.as_ref().is_some_and(|j| j.kill()) {
                return;
            }
            let _ = Command::new("taskkill").args(["/pid", &self.pid.to_string(), "/T", "/F"]).creation_flags(CREATE_NO_WINDOW).status();
        }
        #[cfg(not(windows))]
        {
            let _ = Command::new("kill").args(["-TERM", &format!("-{}", self.pid)]).status();
        }
    }
}

/// Запустить процесс. `on_text` получает вывод кусками, `on_exit` - код выхода.
pub fn spawn_live<F, E>(argv: &[String], cwd: &Path, envs: &[(String, String)], on_text: F, on_exit: E) -> io::Result<Arc<Live>>
where
    F: Fn(String) + Send + Sync + 'static,
    E: FnOnce(i32, Arc<Live>) + Send + 'static,
{
    let mut c = command(argv, cwd)?;
    c.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for (k, v) in envs {
        if v.is_empty() {
            c.env_remove(k);
        } else {
            c.env(k, v);
        }
    }
    let mut child: Child = c.spawn()?;
    #[cfg(windows)]
    let job = {
        use std::os::windows::io::AsRawHandle;
        job::Job::for_process(child.as_raw_handle() as _)
    };
    let live = Arc::new(Live {
        pid: child.id(),
        stdin: Mutex::new(child.stdin.take()),
        #[cfg(windows)]
        job,
        stopped: Mutex::new(false),
    });
    let on_text = Arc::new(on_text);
    let mut readers = Vec::new();
    for stream in
        [child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>), child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>)]
            .into_iter()
            .flatten()
    {
        let cb = on_text.clone();
        readers.push(thread::spawn(move || {
            let mut stream = stream;
            let mut dec = Decoder::new();
            let mut buf = [0u8; 16 * 1024];
            loop {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let s = dec.push(&buf[..n]);
                        if !s.is_empty() {
                            cb(s);
                        }
                    }
                }
            }
            let tail = dec.finish();
            if !tail.is_empty() {
                cb(tail);
            }
        }));
    }
    let l2 = live.clone();
    thread::spawn(move || {
        let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
        for r in readers {
            let _ = r.join();
        }
        on_exit(code, l2);
    });
    Ok(live)
}

/// Выполнить и собрать вывод, с ограничением по времени.
pub struct Captured {
    pub code: i32,
    pub out: String,
    pub err: String,
    pub timed_out: bool,
}

pub fn capture(argv: &[String], cwd: &Path, timeout: Duration) -> io::Result<Captured> {
    capture_env(argv, cwd, timeout, &[])
}

pub(crate) fn capture_env(argv: &[String], cwd: &Path, timeout: Duration, envs: &[(&str, &str)]) -> io::Result<Captured> {
    let mut c = command(argv, cwd)?;
    for (k, v) in envs {
        c.env(k, v);
    }
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn()?;
    // потомки (git.exe за обёрткой Git for Windows, node за .cmd) держат трубы открытыми и после
    // смерти прямого потомка: по сроку гасим всё дерево через объект задания
    #[cfg(windows)]
    let job = {
        use std::os::windows::io::AsRawHandle;
        job::Job::for_process(child.as_raw_handle() as _)
    };
    let kill_tree = |child: &mut Child| {
        #[cfg(windows)]
        if let Some(j) = &job {
            j.kill();
        }
        let _ = child.kill();
    };
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let out_buf = read_into(child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>), tx.clone());
    let err_buf = read_into(child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>), tx);
    let start = Instant::now();
    let mut timed_out = false;
    let code = loop {
        if let Some(st) = child.try_wait()? {
            break st.code().unwrap_or(-1);
        }
        if start.elapsed() > timeout {
            timed_out = true;
            kill_tree(&mut child);
            let _ = child.wait();
            break -1;
        }
        thread::sleep(Duration::from_millis(15));
    };
    // чтение ждём не дольше срока и ещё двух секунд: внук, который держит трубу, не повесит Пульт
    let deadline = start + timeout + Duration::from_secs(2);
    for _ in 0..2 {
        if rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).is_err() {
            timed_out = true;
            kill_tree(&mut child);
            break;
        }
    }
    let dec = |b: &Mutex<Vec<u8>>| {
        // буфер забирается, а не копируется: вывод git log или справки - до сотен килобайт
        let v = b.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default();
        let mut d = Decoder::new();
        let mut s = d.push(&v);
        s.push_str(&d.finish());
        s
    };
    Ok(Captured { code, out: dec(&out_buf), err: dec(&err_buf), timed_out })
}

/// Читать поток в общий буфер; по концу потока - сигнал в `done`.
/// Буфер общий, чтобы при обрыве по сроку взять то, что успело прийти.
fn read_into(stream: Option<Box<dyn Read + Send>>, done: std::sync::mpsc::Sender<()>) -> Arc<Mutex<Vec<u8>>> {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let b2 = buf.clone();
    thread::spawn(move || {
        if let Some(mut s) = stream {
            let mut chunk = [0u8; 16 * 1024];
            loop {
                match s.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(mut v) = b2.lock() {
                            v.extend_from_slice(&chunk[..n]);
                        }
                    }
                }
            }
        }
        let _ = done.send(());
    });
    buf
}

/// Запуск с выводом прямо в терминал (для pult run). Возвращает код выхода.
pub fn run_inherit(argv: &[String], cwd: &Path) -> io::Result<i32> {
    let mut c = command(argv, cwd)?;
    #[cfg(windows)]
    c.creation_flags(0);
    c.stdin(Stdio::inherit()).stdout(Stdio::inherit()).stderr(Stdio::inherit());
    Ok(c.status()?.code().unwrap_or(-1))
}

/// Открыть файл, папку или адрес программой по умолчанию. Без оболочки.
pub fn open_default(target: &str) -> io::Result<()> {
    #[cfg(windows)]
    {
        Command::new("explorer.exe").arg(target).spawn().map(|_| ())
    }
    #[cfg(not(windows))]
    {
        Command::new("xdg-open").arg(target).spawn().map(|_| ())
    }
}

pub fn reveal(target: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        Command::new("explorer.exe").raw_arg(format!("/select,\"{}\"", target.display())).spawn().map(|_| ())
    }
    #[cfg(not(windows))]
    {
        Command::new("xdg-open").arg(target.parent().unwrap_or(target)).spawn().map(|_| ())
    }
}

/// VS Code: запускаем сам Code.exe (рядом с bin\code.cmd), чтобы не проходить через cmd.
pub fn open_in_editor(target: &Path, line: Option<u32>) -> io::Result<()> {
    if let Some(code_cmd) = which("code") {
        let exe = code_cmd.parent().and_then(|bin| bin.parent()).map(|d| d.join("Code.exe")).filter(|p| p.is_file());
        let arg = match line {
            Some(l) => format!("{}:{l}", target.display()),
            None => target.display().to_string(),
        };
        if let Some(exe) = exe {
            let mut c = Command::new(exe);
            c.args(["--goto", &arg]);
            return c.spawn().map(|_| ());
        }
        if code_cmd.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) {
            return Command::new(code_cmd).args(["--goto", &arg]).spawn().map(|_| ());
        }
    }
    #[cfg(windows)]
    {
        Command::new("notepad.exe").arg(target).spawn().map(|_| ())
    }
    #[cfg(not(windows))]
    {
        open_default(&target.to_string_lossy())
    }
}

pub fn open_terminal(dir: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        // «;» для wt - разделитель подкоманд даже внутри -d: папка «a;b» запустила бы вторую команду.
        // Экранирование «\;» ломается на пути вида «x\;y», поэтому такую папку открываем без wt
        if !dir.to_string_lossy().contains(';') {
            if let Some(wt) = which("wt") {
                return Command::new(wt).arg("-d").arg(dir).spawn().map(|_| ());
            }
        }
        let ps = which("pwsh").or_else(|| which("powershell")).unwrap_or_else(|| PathBuf::from("powershell.exe"));
        Command::new(ps).arg("-NoExit").current_dir(dir).creation_flags(0x0000_0010).spawn().map(|_| ())
    }
    #[cfg(not(windows))]
    {
        Command::new("x-terminal-emulator").current_dir(dir).spawn().map(|_| ())
    }
}
