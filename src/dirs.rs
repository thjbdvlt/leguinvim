use std::path::*;

use once_cell::sync::Lazy;

pub fn app_config_dir_create() -> Result<PathBuf, String> {
    let config_dir = app_config_dir();

    std::fs::create_dir_all(config_dir).map_err(|e| format!("{e}"))?;

    Ok(config_dir.to_path_buf())
}

pub fn app_config_dir() -> &'static Path {
    static DIR: Lazy<PathBuf> = Lazy::new(|| glib::user_config_dir().join("leguinvim"));
    DIR.as_path()
}
