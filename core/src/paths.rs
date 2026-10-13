//! Где лежат знания и данные.
//! Знания: PULT_HOME, затем вверх от exe до папки с knowledge/ (рядом с exe или в репозитории),
//! иначе - копия, вшитая в exe.
//! Данные (настройки, история, кэш): PULT_DATA, иначе %LOCALAPPDATA%\dev.dotnotfact.pult\data - не в папке программы,
//! чтобы обновление или удаление программы их не трогало и в репозиторий они не попадали.
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

static HOME: OnceLock<PathBuf> = OnceLock::new();
static DATA: OnceLock<PathBuf> = OnceLock::new();

pub fn home() -> &'static Path {
    HOME.get_or_init(|| {
        if let Ok(h) = std::env::var("PULT_HOME") {
            let p = PathBuf::from(h);
            if p.is_dir() {
                return p;
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            let mut d = exe.parent().map(Path::to_path_buf);
            while let Some(dir) = d {
                if dir.join("knowledge").join("effects.json").is_file() {
                    return dir;
                }
                d = dir.parent().map(Path::to_path_buf);
            }
            if let Some(p) = exe.parent() {
                return p.to_path_buf();
            }
        }
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    })
}

pub fn knowledge_dir() -> PathBuf {
    home().join("knowledge")
}

pub fn data_dir() -> PathBuf {
    DATA.get_or_init(|| {
        let d = std::env::var("PULT_DATA").map(PathBuf::from).unwrap_or_else(|_| {
            let base = std::env::var("LOCALAPPDATA").or_else(|_| std::env::var("XDG_DATA_HOME")).map(PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
            base.join("dev.dotnotfact.pult").join("data")
        });
        let _ = std::fs::create_dir_all(d.join("cache"));
        // один раз забираем настройки и историю из старого места (папка data рядом с программой)
        for f in ["config.json", "history.json"] {
            let old = home().join("data").join(f);
            let new = d.join(f);
            if old.is_file() && !new.exists() {
                let _ = std::fs::copy(&old, &new);
            }
        }
        d
    })
    .clone()
}
