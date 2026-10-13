//! Проверка разбора: cargo run -p pult-core --example scan --release -- <папка> [имя-скрипта]
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let root = std::path::PathBuf::from(a.get(1).cloned().unwrap_or_else(|| ".".into()));
    let t = std::time::Instant::now();
    let kb = pult_core::kb::get();
    println!("kb: {} программ, {} уроков, ошибки: {:?} ({} мс)", kb.tools.len(), kb.topics.len(), kb.errors, t.elapsed().as_millis());
    let p = pult_core::scan::scan(&root, &pult_core::scan::ScanOpts { trusted_hosts: pult_core::store::trusted_hosts() }).expect("scan");
    let known = p.scripts.iter().filter(|s| s.info.known).count();
    println!("{} · {} скриптов, распознано {known}, разбор {} мс", p.name, p.scripts.len(), p.scan_ms);
    println!("стек: {}", p.stacks.join(", "));
    println!("уроки: {}", p.topics.iter().map(|t| format!("{} ({})", t.id, t.why)).collect::<Vec<_>>().join("; "));
    if let Some(g) = &p.git {
        println!("git: {} → {:?} ↑{} ↓{} изм {} stash {} prefix '{}' remotes {:?}", g.branch, g.upstream, g.ahead, g.behind, g.dirty, g.stash, g.prefix, g.remotes.iter().map(|r| format!("{}:{}{}", r.name, r.host.clone().unwrap_or_default(), if r.trusted { "✓" } else { "" })).collect::<Vec<_>>());
    }
    for c in &p.ci {
        println!("ci {} {}: {} джоб, этапы {}", c.kind, c.file, c.job_count, c.stages.iter().map(|s| format!("{}({})", s.name, s.jobs.len())).collect::<Vec<_>>().join(" "));
    }
    println!("health: {:?}", p.health);
    let filter = a.get(2);
    for s in &p.scripts {
        if filter.is_some_and(|f| !s.name.contains(f.as_str())) {
            continue;
        }
        println!("[{}] {} :: {} | fx={:?}{}{} src={} tools={:?}", s.stage, s.name, s.info.summary, s.info.fx, if s.info.long { " LONG" } else { "" }, if s.danger { " DANGER" } else { "" }, s.info.source, s.info.tools);
        if filter.is_some() {
            println!("   argv {:?}", s.argv);
            for pt in &s.info.parts {
                println!("   part [{}] {} - {}", pt.kind, pt.text, pt.note);
            }
            println!("   flags {:?}", s.info.flags.iter().map(|f| format!("{}{}", f.flag, if f.used { "*" } else { "" })).collect::<Vec<_>>());
            println!("   how {:?}", s.info.how);
            println!("   about {}", s.info.about);
        }
    }
}
