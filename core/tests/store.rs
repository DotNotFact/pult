//! Сценарии в настройках: новый получает id, сохранение по тому же id заменяет, удаление убирает.
mod common;

use common::TempDir;
use pult_core::store::{self, Scenario, Step};

#[test]
fn scenario_save_replace_delete() {
    let d = TempDir::new("store");
    // папка данных - своя на этот тест-бинарник: paths::data_dir запоминает её при первом обращении
    std::env::set_var("PULT_DATA", d.path());

    let s = store::save_scenario(Scenario {
        name: "  Утро  ".into(),
        project: d.path().to_string_lossy().into(),
        steps: vec![Step { script_id: "a1".into(), name: "git pull".into(), args: String::new() }],
        ..Default::default()
    });
    assert!(!s.id.is_empty(), "новый сценарий получает id");
    assert_eq!(s.name, "Утро", "имя без пробелов по краям");
    assert!(s.stop_on_error && s.in_tray, "по умолчанию: стоп на ошибке, есть в трее");

    let mut changed = s.clone();
    changed.steps.push(Step { script_id: "b2".into(), name: "pnpm test".into(), args: "--run".into() });
    store::save_scenario(changed);
    let all = store::config().scenarios;
    assert_eq!(all.len(), 1, "тот же id заменяет, а не добавляет");
    assert_eq!(all[0].steps.len(), 2);

    store::delete_scenario(&s.id);
    assert!(store::config().scenarios.is_empty());
}
