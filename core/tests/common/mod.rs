//! Общее для интеграционных тестов ядра: база знаний из этого репозитория.
#![allow(dead_code)] // каждый файл тестов берёт отсюда только своё

use std::path::{Path, PathBuf};
use std::sync::{Arc, Once};

/// Корень репозитория: рядом с ним лежит knowledge/.
pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("core/ лежит в корне репозитория").to_path_buf()
}

/// База знаний именно из knowledge/ репозитория, а не вшитая копия.
/// PULT_HOME ставится до первого обращения к путям - paths::home() запоминает ответ навсегда.
pub fn kb() -> Arc<pult_core::kb::Kb> {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        assert!(repo_root().join("knowledge").join("effects.json").is_file(), "нет knowledge/ в корне репозитория");
        std::env::set_var("PULT_HOME", repo_root());
    });
    pult_core::kb::get()
}

/// Временная папка теста; удаляется при выходе из области видимости.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let name = format!("pult-test-{tag}-{}-{}-{}", std::process::id(), pult_core::util::now_ms(), N.fetch_add(1, Ordering::Relaxed));
        let p = std::env::temp_dir().join(name);
        std::fs::create_dir_all(&p).expect("временная папка");
        TempDir(p)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Записать файл (папки создаются сами).
    pub fn write(&self, rel: &str, text: &str) {
        let p = self.0.join(rel);
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).expect("папка файла");
        }
        std::fs::write(p, text).expect("запись файла");
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
