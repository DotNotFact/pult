//! Где лежат знания и данные.
//! Знания: PULT_HOME, затем knowledge/ рядом с exe или в корне репозитория при сборке из него
//! (`home_candidates`), иначе - копия, вшитая в exe.
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
            if let Some(dir) = home_candidates(&exe).into_iter().find(|d| d.join("knowledge").join("effects.json").is_file()) {
                return dir;
            }
            if let Some(p) = exe.parent() {
                return p.to_path_buf();
            }
        }
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    })
}

/// Где искать knowledge/ рядом с программой: папка exe и, для сборки из репозитория, корень
/// репозитория (exe в target/<профиль>/, а тесты и примеры - ещё на уровень глубже).
/// Выше не идём: knowledge/ в любой папке над программой (Загрузки, чужой проект) подменила бы базу.
pub fn home_candidates(exe: &Path) -> Vec<PathBuf> {
    let Some(dir) = exe.parent() else { return vec![] };
    let mut out = vec![dir.to_path_buf()];
    let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut profile = dir;
    if ["deps", "examples"].contains(&name(dir).as_str()) {
        if let Some(p) = dir.parent() {
            profile = p;
        }
    }
    if let Some(target) = profile.parent().filter(|t| name(t) == "target") {
        if let Some(repo) = target.parent() {
            out.push(repo.to_path_buf());
        }
    }
    out
}

pub fn knowledge_dir() -> PathBuf {
    home().join("knowledge")
}

pub fn data_dir() -> PathBuf {
    DATA.get_or_init(|| {
        let d = std::env::var("PULT_DATA").map(PathBuf::from).unwrap_or_else(|_| {
            let base = std::env::var("LOCALAPPDATA")
                .or_else(|_| std::env::var("XDG_DATA_HOME"))
                .map(PathBuf::from)
                .unwrap_or_else(|_| std::env::temp_dir());
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
