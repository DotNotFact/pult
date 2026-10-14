//! Замер ядра: cargo run -p pult-core --example perf --release -- <папка> [повторов]
//! Печатает время базы знаний, каждой части разбора (первый прогон и следующие), память
//! процесса (рабочий набор, пик, частная), выделения кучи и размеры того, что уходит в окно.
//! Пол замера - пустая папка: всё, что она показывает, - цена самого прибора и базы.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

struct Counting;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(l.size(), Relaxed);
        let now = LIVE.fetch_add(l.size(), Relaxed) + l.size();
        PEAK.fetch_max(now, Relaxed);
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Relaxed);
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        BYTES.fetch_add(n, Relaxed);
        if n >= l.size() {
            let now = LIVE.fetch_add(n - l.size(), Relaxed) + (n - l.size());
            PEAK.fetch_max(now, Relaxed);
        } else {
            LIVE.fetch_sub(l.size() - n, Relaxed);
        }
        System.realloc(p, l, n)
    }
}

#[global_allocator]
static A: Counting = Counting;

#[cfg(windows)]
mod mem {
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        cb: u32,
        page_faults: u32,
        peak_ws: usize,
        ws: usize,
        q1: usize,
        q2: usize,
        q3: usize,
        q4: usize,
        private: usize,
        peak_private: usize,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(h: isize, c: *mut Counters, cb: u32) -> i32;
    }
    /// (рабочий набор, пик рабочего набора, частная память) в КБ
    pub fn now() -> (usize, usize, usize) {
        let mut c = Counters { cb: std::mem::size_of::<Counters>() as u32, ..Default::default() };
        // SAFETY: структура того размера, который передан в cb; дескриптор текущего процесса
        unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
        (c.ws / 1024, c.peak_ws / 1024, c.private / 1024)
    }
}
#[cfg(not(windows))]
mod mem {
    pub fn now() -> (usize, usize, usize) {
        (0, 0, 0)
    }
}

fn mem_line(label: &str) {
    let (ws, peak, private) = mem::now();
    println!(
        "mem {label}: ws {} КБ, peak ws {} КБ, private {} КБ | куча: живая {} КБ, пик {} КБ",
        ws,
        peak,
        private,
        LIVE.load(Relaxed) / 1024,
        PEAK.load(Relaxed) / 1024
    );
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    if v.is_empty() {
        0.0
    } else {
        v[v.len() / 2]
    }
}

/// Сколько кучи занимает каждая часть базы, разобранная в serde_json::Value, и программы
/// (структуры Tool) - чтобы видеть, куда уходят мегабайты базы.
fn kb_heap() {
    let dir = pult_core::paths::knowledge_dir();
    let mut rows: Vec<(String, usize, usize)> = Vec::new();
    let mut keep: Vec<serde_json::Value> = Vec::new();
    for sub in ["", "tools", "topics"] {
        let Ok(rd) = std::fs::read_dir(dir.join(sub)) else { continue };
        let (mut text, mut heap) = (0, 0);
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "json") {
                continue;
            }
            let t = std::fs::read_to_string(&p).unwrap_or_default();
            let before = LIVE.load(Relaxed);
            let v: serde_json::Value = serde_json::from_str(t.trim_start_matches('\u{feff}')).unwrap_or_default();
            heap += LIVE.load(Relaxed).saturating_sub(before);
            text += t.len();
            keep.push(v);
        }
        rows.push((if sub.is_empty() { "корень".into() } else { sub.into() }, text, heap));
    }
    // регулярки базы - так же, как их собирает kb.rs (без учёта регистра, многострочные)
    let heur: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("heuristics.json")).unwrap_or_default()).unwrap_or_default();
    let stages: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("stages.json")).unwrap_or_default()).unwrap_or_default();
    let mut pats: Vec<String> = Vec::new();
    for v in heur["wrappers"]["re"].as_array().into_iter().flatten() {
        pats.extend(v.as_str().map(str::to_string));
    }
    pats.extend(heur["skipSegments"]["re"].as_str().map(str::to_string));
    pats.extend(heur["danger"]["re"].as_str().map(str::to_string));
    for (k, v) in heur["fileFx"].as_object().into_iter().flatten() {
        if k != "_" {
            pats.extend(v.as_str().map(str::to_string));
        }
    }
    for s in stages["stages"].as_array().into_iter().flatten() {
        let alts: Vec<String> = s["words"].as_array().into_iter().flatten().filter_map(|w| w.as_str()).map(regex::escape).collect();
        if !alts.is_empty() {
            pats.push(format!(r"(?:^|[:\-_ ./])(?:{})(?:$|[:\-_ ./])", alts.join("|")));
        }
    }
    let before = LIVE.load(Relaxed);
    let t = Instant::now();
    let res: Vec<regex::Regex> = pats
        .iter()
        .filter_map(|p| regex::RegexBuilder::new(p).case_insensitive(true).multi_line(true).size_limit(1 << 22).build().ok())
        .collect();
    println!(
        "регулярки базы: {} шт, куча {} КБ, сборка {:.1} мс",
        res.len(),
        LIVE.load(Relaxed).saturating_sub(before) / 1024,
        ms(t.elapsed())
    );
    println!(
        "куча базы как Value: {}",
        rows.iter().map(|(k, t, h)| format!("{k}: файлы {} КБ -> куча {} КБ", t / 1024, h / 1024)).collect::<Vec<_>>().join("; ")
    );
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let root = std::path::PathBuf::from(a.get(1).cloned().unwrap_or_else(|| ".".into()));
    let n: usize = a.get(2).and_then(|x| x.parse().ok()).unwrap_or(5);
    mem_line("старт");

    let t = Instant::now();
    let kb = pult_core::kb::get();
    let kb_first = t.elapsed();
    println!(
        "kb первая загрузка {:.1} мс (с диска: {}, программ {}, уроков {})",
        ms(kb_first),
        kb.from_disk,
        kb.tools.len(),
        kb.topics.len()
    );
    mem_line("после базы");
    // старую копию отпускаем: иначе в куче живут две базы сразу и замер врёт вдвое
    drop(kb);
    let mut kb_re = Vec::new();
    for _ in 0..n {
        let t = Instant::now();
        let _ = pult_core::kb::reload();
        kb_re.push(ms(t.elapsed()));
    }
    println!("kb перечитать: медиана {:.1} мс из {n}", median(kb_re));
    let kb = pult_core::kb::get();
    mem_line("после перечитывания базы");
    // разбор кучи базы сам выделяет мегабайты - только по просьбе, чтобы не портить замер памяти
    if std::env::var_os("PERF_KB_HEAP").is_some() {
        kb_heap();
    }

    let t = Instant::now();
    let bundle = kb.client_bundle();
    let b_ms = ms(t.elapsed());
    let t = Instant::now();
    let bundle_s = serde_json::to_string(&bundle).unwrap_or_default();
    println!("client_bundle: собрать {:.2} мс, в JSON {:.2} мс, {} байт", b_ms, ms(t.elapsed()), bundle_s.len());
    if let Some(o) = bundle.as_object() {
        let mut parts: Vec<(String, usize)> =
            o.iter().map(|(k, v)| (k.clone(), serde_json::to_string(v).map(|s| s.len()).unwrap_or(0))).collect();
        parts.sort_by_key(|x| std::cmp::Reverse(x.1));
        println!("  по ключам: {}", parts.iter().map(|(k, s)| format!("{k} {s}")).collect::<Vec<_>>().join(", "));
    }
    let topics_full: usize = kb.topics.iter().map(|t| serde_json::to_string(t).map(|s| s.len()).unwrap_or(0)).sum();
    println!("  уроки целиком (команда topic, по одному): всего {topics_full} байт");
    drop(bundle);
    drop(bundle_s);

    let opts = pult_core::scan::ScanOpts { trusted_hosts: pult_core::store::trusted_hosts() };
    // (всего мс, части, выделений, байт)
    type Run = (f64, Vec<(&'static str, f64)>, usize, usize);
    let mut runs: Vec<Run> = Vec::new();
    let mut last = None;
    for i in 0..=n {
        let (a0, b0) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
        let t = Instant::now();
        let (p, ph) = pult_core::scan::scan_timed(&root, &opts).expect("scan");
        let total = ms(t.elapsed());
        let allocs = ALLOCS.load(Relaxed) - a0;
        let bytes = BYTES.load(Relaxed) - b0;
        if i == 0 {
            mem_line("после первого разбора");
        }
        runs.push((total, ph.iter().map(|(k, d)| (*k, ms(*d))).collect(), allocs, bytes));
        last = Some(p);
    }
    let p = last.expect("разбор");
    let (t0, ph0, al0, by0) = &runs[0];
    println!(
        "scan первый: {:.1} мс, выделений {al0}, {} КБ | {}",
        t0,
        by0 / 1024,
        ph0.iter().map(|(k, v)| format!("{k} {v:.1}")).collect::<Vec<_>>().join(", ")
    );
    let warm = &runs[1..];
    let names: Vec<&'static str> = ph0.iter().map(|x| x.0).collect();
    println!(
        "scan повторный (медиана из {}): {:.1} мс, выделений {}, {} КБ | {}",
        warm.len(),
        median(warm.iter().map(|r| r.0).collect()),
        median(warm.iter().map(|r| r.2 as f64).collect()) as usize,
        median(warm.iter().map(|r| r.3 as f64).collect()) as usize / 1024,
        names
            .iter()
            .enumerate()
            .map(|(i, k)| format!("{k} {:.1}", median(warm.iter().map(|r| r.1.get(i).map(|x| x.1).unwrap_or(0.0)).collect())))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "проект: {} скриптов, git: {}",
        p.scripts.len(),
        p.git.as_ref().map(|g| if g.untrusted { "не доверен" } else { "есть" }).unwrap_or("нет")
    );

    // git отдельно и как у доверенного репозитория: в разборе выше недоверенный git не запускается
    let mut gt = Vec::new();
    let mut ok = false;
    for _ in 0..n {
        let t = Instant::now();
        ok = pult_core::git::info_with(&root, &opts.trusted_hosts, |_| true).is_some_and(|g| !g.untrusted);
        gt.push(ms(t.elapsed()));
    }
    println!("git как доверенный: медиана {:.1} мс из {n} (git запускался: {ok})", median(gt));

    // правила подкоманд (fancy-regex) компилируются лениво, при первом совпадении: сколько стоит
    // собрать их все на свежей базе - это потолок того, что ленивость экономит первому разбору
    {
        let fresh = pult_core::kb::reload();
        let (mut count, mut fail) = (0, 0);
        let before = LIVE.load(Relaxed);
        let t = Instant::now();
        for tool in &fresh.tools {
            for s in &tool.sub {
                count += 1;
                if s.re().is_none() {
                    fail += 1;
                }
            }
        }
        println!(
            "правила подкоманд: {count} шт (не собралось {fail}), все сразу {:.1} мс, куча +{} КБ",
            ms(t.elapsed()),
            LIVE.load(Relaxed).saturating_sub(before) / 1024
        );
        let kb_wrappers = fresh.wrappers.len();
        std::hint::black_box(kb_wrappers);
    }

    // снимок результата для сверки до и после правки (время разбора обнулено)
    if let Ok(out) = std::env::var("PERF_DUMP") {
        let mut c = p.clone();
        c.scanned_at = 0;
        c.scan_ms = 0;
        let _ = std::fs::write(out, serde_json::to_string_pretty(&c).unwrap_or_default());
    }

    // что уходит в окно
    let t = Instant::now();
    let js = serde_json::to_string(&p).unwrap_or_default();
    println!("Project в JSON: {} байт за {:.2} мс", js.len(), ms(t.elapsed()));
    let v = serde_json::to_value(&p).unwrap_or_default();
    if let Some(o) = v.as_object() {
        let mut parts: Vec<(String, usize)> =
            o.iter().map(|(k, v)| (k.clone(), serde_json::to_string(v).map(|s| s.len()).unwrap_or(0))).collect();
        parts.sort_by_key(|x| std::cmp::Reverse(x.1));
        println!("  по ключам: {}", parts.iter().take(8).map(|(k, s)| format!("{k} {s}")).collect::<Vec<_>>().join(", "));
    }
    let mut fields: std::collections::BTreeMap<String, usize> = Default::default();
    for s in v["scripts"].as_array().into_iter().flatten() {
        for (k, x) in s.as_object().into_iter().flatten() {
            if k == "info" {
                for (ik, ix) in x.as_object().into_iter().flatten() {
                    *fields.entry(format!("info.{ik}")).or_default() +=
                        serde_json::to_string(ix).map(|s| s.len()).unwrap_or(0) + ik.len() + 3;
                }
            } else {
                *fields.entry(k.clone()).or_default() += serde_json::to_string(x).map(|s| s.len()).unwrap_or(0) + k.len() + 3;
            }
        }
    }
    let mut f: Vec<_> = fields.into_iter().collect();
    f.sort_by_key(|x| std::cmp::Reverse(x.1));
    println!("  скрипты по полям: {}", f.iter().take(12).map(|(k, s)| format!("{k} {s}")).collect::<Vec<_>>().join(", "));

    // что делает окно с проектом: отпечаток (клон + JSON + хеш) и копия для ответа
    let mut fp = Vec::new();
    let mut cl = Vec::new();
    for _ in 0..n.max(3) {
        let t = Instant::now();
        let mut c = p.clone();
        c.scanned_at = 0;
        let s = serde_json::to_string(&c).unwrap_or_default();
        std::hint::black_box(s.len());
        fp.push(ms(t.elapsed()));
        let t = Instant::now();
        std::hint::black_box(p.clone());
        cl.push(ms(t.elapsed()));
    }
    println!("окно: отпечаток как в app (клон+JSON) {:.2} мс, клон проекта {:.2} мс", median(fp), median(cl));
    // отпечаток потоком в хеш, без клона и без строки
    struct HashWriter(std::collections::hash_map::DefaultHasher);
    impl std::io::Write for HashWriter {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            std::hash::Hasher::write(&mut self.0, b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut fp2 = Vec::new();
    let (a0, b0) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
    for _ in 0..n.max(3) {
        let t = Instant::now();
        let mut h = HashWriter(Default::default());
        let _ = serde_json::to_writer(&mut h, &p);
        std::hint::black_box(std::hash::Hasher::finish(&h.0));
        fp2.push(ms(t.elapsed()));
    }
    let runs = n.max(3);
    println!(
        "окно: отпечаток потоком {:.2} мс, выделений на раз {}, {} КБ",
        median(fp2),
        (ALLOCS.load(Relaxed) - a0) / runs,
        (BYTES.load(Relaxed) - b0) / runs / 1024
    );
    // событие project: json!({..., "data": проект}) строит дерево Value, потом строку; структура по ссылке - сразу строку
    let (a0, b0) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
    let peak0 = LIVE.load(Relaxed);
    PEAK.store(peak0, Relaxed);
    let t = Instant::now();
    let v = serde_json::json!({ "reason": "files", "data": p.clone() });
    let s = serde_json::to_string(&v).unwrap_or_default();
    let took = ms(t.elapsed());
    let peak_tree = PEAK.load(Relaxed) - peak0;
    drop((v, s));
    println!(
        "событие через json!: {:.2} мс, выделений {}, {} КБ, пик кучи +{} КБ",
        took,
        ALLOCS.load(Relaxed) - a0,
        (BYTES.load(Relaxed) - b0) / 1024,
        peak_tree / 1024
    );
    #[derive(serde::Serialize)]
    struct Ev<'a> {
        reason: &'a str,
        data: &'a pult_core::model::Project,
    }
    let (a0, b0) = (ALLOCS.load(Relaxed), BYTES.load(Relaxed));
    let peak0 = LIVE.load(Relaxed);
    PEAK.store(peak0, Relaxed);
    let t = Instant::now();
    let s = serde_json::to_string(&Ev { reason: "files", data: &p }).unwrap_or_default();
    let took = ms(t.elapsed());
    let peak_ref = PEAK.load(Relaxed) - peak0;
    drop(s);
    println!(
        "событие структурой по ссылке: {:.2} мс, выделений {}, {} КБ, пик кучи +{} КБ",
        took,
        ALLOCS.load(Relaxed) - a0,
        (BYTES.load(Relaxed) - b0) / 1024,
        peak_ref / 1024
    );
    mem_line("конец");
}
