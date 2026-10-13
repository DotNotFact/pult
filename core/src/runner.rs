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

pub fn git_bash() -> Option<PathBuf> {
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
pub struct Decoder {
    pending: Vec<u8>,
}
impl Decoder {
    pub fn new() -> Self {
        Self { pending: Vec::new() }
    }
    pub fn push(&mut self, chunk: &[u8]) -> String {
        self.pending.extend_from_slice(chunk);
        match std::str::from_utf8(&self.pending) {
            Ok(s) => {
                let out = s.to_string();
                self.pending.clear();
                out
            }
            Err(e) if e.error_len().is_none() => {
                let n = e.valid_up_to();
                let out = String::from_utf8_lossy(&self.pending[..n]).into_owned();
                self.pending.drain(..n);
                out
            }
            Err(_) => {
                let (s, _, _) = encoding_rs::IBM866.decode(&self.pending);
                let out = s.into_owned();
                self.pending.clear();
                out
            }
        }
    }
    pub fn finish(&mut self) -> String {
        let out = String::from_utf8_lossy(&self.pending).into_owned();
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
    for stream in [child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>), child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>)].into_iter().flatten() {
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

pub fn capture_env(argv: &[String], cwd: &Path, timeout: Duration, envs: &[(&str, &str)]) -> io::Result<Captured> {
    let mut c = command(argv, cwd)?;
    for (k, v) in envs {
        c.env(k, v);
    }
    c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn()?;
    let mut so = child.stdout.take();
    let mut se = child.stderr.take();
    let t1 = thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(s) = so.as_mut() {
            let _ = s.read_to_end(&mut v);
        }
        v
    });
    let t2 = thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(s) = se.as_mut() {
            let _ = s.read_to_end(&mut v);
        }
        v
    });
    let start = Instant::now();
    let mut timed_out = false;
    let code = loop {
        if let Some(st) = child.try_wait()? {
            break st.code().unwrap_or(-1);
        }
        if start.elapsed() > timeout {
            timed_out = true;
            let _ = child.kill();
            let _ = child.wait();
            break -1;
        }
        thread::sleep(Duration::from_millis(15));
    };
    let dec = |v: Vec<u8>| {
        let mut d = Decoder::new();
        let mut s = d.push(&v);
        s.push_str(&d.finish());
        s
    };
    Ok(Captured { code, out: dec(t1.join().unwrap_or_default()), err: dec(t2.join().unwrap_or_default()), timed_out })
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
        if let Some(wt) = which("wt") {
            return Command::new(wt).arg("-d").arg(dir).spawn().map(|_| ());
        }
        let ps = which("pwsh").or_else(|| which("powershell")).unwrap_or_else(|| PathBuf::from("powershell.exe"));
        Command::new(ps).arg("-NoExit").current_dir(dir).creation_flags(0x0000_0010).spawn().map(|_| ())
    }
    #[cfg(not(windows))]
    {
        Command::new("x-terminal-emulator").current_dir(dir).spawn().map(|_| ())
    }
}
