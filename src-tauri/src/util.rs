use std::path::PathBuf;

pub fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        let home = std::env::var_os("HOME")
            .or_else(|| {
                directories::BaseDirs::new().map(|b| b.home_dir().as_os_str().to_os_string())
            });
        if let Some(home) = home {
            return PathBuf::from(home).join(rest).display().to_string();
        }
    }

    path.to_string()
}
