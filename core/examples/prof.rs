fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    let t = std::time::Instant::now();
    let kb = pult_core::kb::get();
    println!("kb {} мс ({} программ)", t.elapsed().as_millis(), kb.tools.len());
    let t = std::time::Instant::now();
    let g = pult_core::git::info(&root, &[]);
    println!("git {} мс ({})", t.elapsed().as_millis(), g.is_some());
    let t = std::time::Instant::now();
    let c = pult_core::ci_config::detect(&root, &kb);
    println!("ci {} мс ({})", t.elapsed().as_millis(), c.len());
    for i in 0..3 {
        let t = std::time::Instant::now();
        let p = pult_core::scan::scan(&root, &Default::default()).unwrap();
        println!("scan #{i} {} мс (внутри {} мс)", t.elapsed().as_millis(), p.scan_ms);
    }
}
