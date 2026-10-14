//! Декодер вывода процессов и мелкие функции util.
use pult_core::runner::Decoder;
use pult_core::util::{display_argv, inside, is_unc};
use std::path::Path;

/// «привет, мир» в cp866 (так пишут консольные программы Windows с русской локалью).
fn cp866(s: &str) -> Vec<u8> {
    let (b, _, bad) = encoding_rs::IBM866.encode(s);
    assert!(!bad, "строка кодируется в cp866 без потерь");
    b.into_owned()
}

#[test]
fn decoder_glues_utf8_letter_split_between_chunks() {
    let bytes = "Сборка готова".as_bytes();
    // «С» - два байта; режем посередине буквы
    let mut d = Decoder::new();
    let a = d.push(&bytes[..1]);
    let b = d.push(&bytes[1..]);
    assert_eq!(a, "", "половину буквы не отдаём");
    assert_eq!(format!("{a}{b}{}", d.finish()), "Сборка готова");
    assert!(!b.contains('\u{fffd}'));
}

#[test]
fn decoder_glues_every_split_point() {
    let text = "ok: ёжик и щука\n";
    let bytes = text.as_bytes();
    for cut in 0..=bytes.len() {
        let mut d = Decoder::new();
        let out = format!("{}{}{}", d.push(&bytes[..cut]), d.push(&bytes[cut..]), d.finish());
        assert_eq!(out, text, "разрез на байте {cut}");
    }
}

#[test]
fn decoder_converts_cp866() {
    let mut d = Decoder::new();
    let out = d.push(&cp866("Ошибка: файл не найден\r\n"));
    assert_eq!(out, "Ошибка: файл не найден\r\n");
}

#[test]
fn decoder_keeps_cp866_across_chunks() {
    // «р» в cp866 - 0xE0, это же начало трёхбайтового символа UTF-8; «аз» - 0xA0 0xA7.
    // Кусок « р» + кусок «аз» раньше склеивались в допустимый UTF-8 (U+0827) - мусор вместо «раз»
    let chunks = [cp866("Готово"), cp866(" р"), cp866("аз\r\n"), cp866("тем\r\n")];
    let mut d = Decoder::new();
    let out: String = chunks.iter().map(|c| d.push(c)).collect::<String>() + &d.finish();
    assert_eq!(out, "Готово раз\r\nтем\r\n", "«тем» (E2 A5 AC) - тоже допустимый UTF-8, но поток уже опознан как cp866");
}

#[test]
fn decoder_cp866_split_at_every_byte() {
    let bytes = cp866("Готово раз\r\n");
    for cut in 0..=bytes.len() {
        let mut d = Decoder::new();
        let out = format!("{}{}{}", d.push(&bytes[..cut]), d.push(&bytes[cut..]), d.finish());
        assert_eq!(out, "Готово раз\r\n", "разрез на байте {cut}");
    }
}

#[test]
fn decoder_cp866_stream_ending_mid_letter() {
    // поток кончился на «р» (0xE0): в UTF-8 это обрывок, в cp866 - целая буква
    let mut d = Decoder::new();
    let out = format!("{}{}", d.push(&cp866("ok: мир")), d.finish());
    assert_eq!(out, "ok: мир");
    let mut d = Decoder::new();
    let out = format!("{}{}", d.push(&cp866("ok р")), d.finish());
    assert_eq!(out, "ok р");
}

#[test]
fn decoder_does_not_switch_utf8_stream_to_cp866() {
    let mut d = Decoder::new();
    assert_eq!(d.push("готово ".as_bytes()), "готово ");
    let out = d.push(b"\xff end");
    assert!(out.ends_with(" end") && out.contains('\u{fffd}'), "одна битая последовательность - порча, а не cp866: {out:?}");
    assert_eq!(d.push("ещё".as_bytes()), "ещё");
}

#[test]
fn is_unc_detects_network_paths() {
    assert!(is_unc(r"\\server\share\x"));
    assert!(is_unc("//server/share"));
    assert!(is_unc(r#""\\server\share""#), "путь в кавычках из команды");
    assert!(!is_unc(r"\server\share"), "один слеш - корень текущего диска, не сеть");
    assert!(!is_unc(r"C:\projects\x"));
    assert!(!is_unc("/home/user"));
    assert!(!is_unc(r".\scripts\a.ps1"));
}

#[test]
fn inside_normalizes_dots_and_case() {
    let root = if cfg!(windows) { Path::new(r"C:\Projects\App") } else { Path::new("/projects/app") };
    assert!(inside(root, root));
    assert!(inside(root, &root.join("scripts").join("a.ps1")));
    assert!(inside(root, &root.join("scripts").join("..").join("b.js")));
    assert!(inside(root, &root.join(".").join("c.sh")));
    assert!(!inside(root, &root.join("..").join("Other").join("x.js")), "выход наверх через ..");
    assert!(!inside(root, &root.join("..").join("AppX").join("x.js")), "соседняя папка с тем же началом имени - не внутри");
    if cfg!(windows) {
        assert!(inside(root, Path::new(r"c:\projects\app\src\main.ts")), "регистр на Windows не важен");
    }
}

#[test]
fn inside_with_trailing_separator_root() {
    let root = if cfg!(windows) { Path::new(r"C:\Projects\App\") } else { Path::new("/projects/app/") };
    assert!(inside(root, &root.join("x.js")));
}

#[test]
fn display_argv_quotes_only_where_needed() {
    let argv: Vec<String> = ["pnpm", "run", "build", "", "a b", "x&y", "--flag=1"].iter().map(|s| s.to_string()).collect();
    assert_eq!(display_argv(&argv), r#"pnpm run build "" "a b" "x&y" --flag=1"#);
    assert_eq!(display_argv(&[]), "");
}

#[test]
fn decoder_box_char_before_cyrillic_does_not_lock_utf8() {
    // «═» (0xCD) + «И» (0x88) - допустимый двухбайтовый UTF-8 (U+0348), но не кириллица:
    // раньше поток запирался в UTF-8, и следующий кусок cp866 превращался в мусор
    let mut d = Decoder::new();
    let _ = d.push(&cp866("Build ═Итог: 3\r\n"));
    assert_eq!(d.push(&cp866("Готово\r\n")), "Готово\r\n");
}

#[test]
fn decoder_stray_byte_after_utf8_symbols_does_not_lock_cp866() {
    // в потоке UTF-8 только трёхбайтовые «✓», потом одна битая последовательность:
    // раньше она запирала cp866, и русский текст дальше шёл мусором
    let mut d = Decoder::new();
    assert_eq!(d.push("✓ build ok\n".as_bytes()), "✓ build ok\n");
    let _ = d.push(&[0x80]);
    assert_eq!(d.push("Привет\n".as_bytes()), "Привет\n");
}

#[test]
fn decoder_utf8_keeps_split_letter_after_broken_byte() {
    // кусок с битым байтом, который кончается на половине «П» (D0 9F): половину не теряем
    let mut d = Decoder::new();
    assert_eq!(d.push("готово ".as_bytes()), "готово ");
    let a = d.push(b"\xff ok \xd0");
    assert!(a.ends_with(" ok ") && a.contains('\u{fffd}'), "{a:?}");
    assert_eq!(d.push(&[b"\x9f".as_slice(), "ривет\n".as_bytes()].concat()), "Привет\n");
}

#[cfg(windows)]
#[test]
fn capture_timeout_kills_grandchildren_holding_the_pipe() {
    // cmd -> ping: ping наследует трубу вывода и живёт 30 с. Раньше по сроку гас только cmd,
    // а чтение трубы ждало ping до конца
    let argv: Vec<String> = ["cmd", "/c", "ping", "-n", "30", "127.0.0.1"].iter().map(|s| s.to_string()).collect();
    let t0 = std::time::Instant::now();
    let c = pult_core::runner::capture(&argv, &std::env::temp_dir(), std::time::Duration::from_millis(800)).expect("запуск");
    assert!(c.timed_out);
    // срок 0,8 с; запасной предел чтения - срок плюс 2 с. Меньше 2,4 с - значит дерево погасило задание
    assert!(t0.elapsed() < std::time::Duration::from_millis(2400), "ждали {:?}", t0.elapsed());
}
