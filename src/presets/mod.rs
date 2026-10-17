use std::fs;
use std::path::PathBuf;

pub const COCKPIT_PICKER_TOML_TEMPLATE: &str = include_str!("cockpit-picker.toml");

pub fn runtime_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(xdg).join("waymux")
    } else {
        let uid = nix::unistd::getuid();
        PathBuf::from(format!("/tmp/waymux-{}", uid))
    }
}

pub fn materialize_cockpit_toml() -> PathBuf {
    let dir = runtime_dir();
    let _ = fs::create_dir_all(&dir);
    let target = dir.join("cockpit-picker.toml");

    let exe_path = std::env::current_exe()
        .ok()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "waymux".to_string());

    let rendered = COCKPIT_PICKER_TOML_TEMPLATE.replace("__WAYMUX_BIN__", &exe_path);

    let needs_write = match fs::read_to_string(&target) {
        Ok(existing) => existing != rendered,
        Err(_) => true,
    };

    if needs_write {
        let _ = fs::write(&target, rendered);
    }

    target
}
