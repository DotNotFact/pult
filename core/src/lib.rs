//! Ядро Пульта: разбор проекта, объяснения по базе знаний, git, CI, запуск процессов.
//! Окно (app) и терминал (cli) - тонкие оболочки над этим ядром.

pub mod ci;
pub mod ci_config;
pub mod env;
pub mod explain;
pub mod git;
pub mod helpcmd;
pub mod kb;
pub mod model;
pub mod paths;
pub mod runner;
pub mod scan;
pub mod store;
pub mod topics;
pub mod util;

pub use model::*;
