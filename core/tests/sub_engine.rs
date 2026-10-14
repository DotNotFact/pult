//! Правила подкоманд собираются крейтом regex, а fancy-regex берётся только для заглядывания
//! вперёд и назад (kb::SubRe). Ради памяти: 26 КБ на правило против 70 КБ. Здесь проверяется, что
//! ответ при этом не изменился: каждое правило базы на строках, похожих на команды той же
//! программы, совпадает там же и так же далеко, как совпадало через fancy-regex.
mod common;

use pult_core::kb::SubRe;

/// Строка, похожая на команду, по тексту правила: «^migrate\s+dev(?:\s|$)» -> «migrate dev».
fn sample(pat: &str) -> String {
    let mut s = pat.to_string();
    for (a, b) in [
        ("(?:\\s|$)", ""),
        ("\\s+", " "),
        ("\\s*", " "),
        ("\\s", " "),
        ("\\b", ""),
        (".*", " "),
        ("(?:", ""),
        ("^", ""),
        ("$", ""),
        ("(", ""),
        (")", ""),
        ("?", ""),
        ("\\", ""),
    ] {
        s = s.replace(a, b);
    }
    s.split('|').next().unwrap_or("").trim().to_string()
}

#[test]
fn plain_engine_matches_like_fancy_on_every_rule() {
    let kb = common::kb();
    let (mut rules, mut plain, mut checks) = (0, 0, 0);
    for tool in &kb.tools {
        let mut lines: Vec<String> = vec![String::new(), "--help".into(), "x".into()];
        for s in &tool.sub {
            let base = sample(&s.pat);
            lines.push(base.to_uppercase());
            lines.push(format!("{base} --force a b"));
            lines.push(format!("{base}x"));
            lines.push(base);
        }
        for s in &tool.sub {
            rules += 1;
            let old = fancy_regex::RegexBuilder::new(&format!("(?i){}", s.pat)).backtrack_limit(100_000).build().ok();
            let new = SubRe::build(&s.pat);
            assert_eq!(old.is_some(), new.is_some(), "{}: правило «{}» собирается по-разному", tool.id, s.pat);
            let (Some(old), Some(new)) = (old, new) else { continue };
            if matches!(new, SubRe::Plain(_)) {
                plain += 1;
            }
            for l in &lines {
                checks += 1;
                let was = old.find(l).ok().flatten().map(|m| m.end());
                assert_eq!(new.find_end(l), was, "{}: «{}» на «{l}»", tool.id, s.pat);
                assert_eq!(new.is_match(l), old.is_match(l).unwrap_or(false), "{}: «{}» на «{l}»", tool.id, s.pat);
            }
        }
    }
    assert!(rules > 0 && checks > rules, "правил {rules}, проверок {checks}");
    // заглядывание вперёд встречается в немногих правилах - остальные обязаны уйти в regex
    assert!(plain * 10 > rules * 8, "через regex собрано {plain} из {rules}");
}
