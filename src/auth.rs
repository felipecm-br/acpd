use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

pub fn get_token_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(xdg).join("waymux").join("token")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".cache")
            .join("waymux")
            .join("token")
    } else {
        let uid = nix::unistd::getuid();
        PathBuf::from(format!("/tmp/waymux-{}", uid)).join("token")
    }
}

pub fn get_legacy_token_path() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        PathBuf::from(xdg).join("acpd").join("token")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".cache")
            .join("acpd")
            .join("token")
    } else {
        let uid = nix::unistd::getuid();
        PathBuf::from(format!("/tmp/acpd-{}", uid)).join("token")
    }
}

fn write_token_file(token_path: &std::path::Path, token: &str) -> anyhow::Result<()> {
    if let Some(parent) = token_path.parent() {
        fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
    }

    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);

    let mut token_file = options.open(token_path)?;
    token_file.write_all(token.as_bytes())?;
    Ok(())
}

pub fn generate_and_save_token_to(token_path: &std::path::Path) -> anyhow::Result<String> {
    let mut bytes = [0u8; 16];
    let mut file = fs::File::open("/dev/urandom")?;
    file.read_exact(&mut bytes)?;

    let token = bytes
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();

    write_token_file(token_path, &token)?;
    tracing::info!("Generated session token at {}", token_path.display());

    Ok(token)
}

pub fn generate_and_save_token() -> anyhow::Result<String> {
    let primary = get_token_path();
    let token = generate_and_save_token_to(&primary)?;
    let legacy = get_legacy_token_path();
    let _ = write_token_file(&legacy, &token);
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_and_save_token() {
        let test_dir = std::env::temp_dir().join(format!("acpd-test-{}", std::process::id()));
        let token_path = test_dir.join("token");
        let token = generate_and_save_token_to(&token_path).unwrap();
        assert_eq!(token.len(), 32);

        assert!(token_path.exists());

        let read_back = fs::read_to_string(&token_path).unwrap();
        assert_eq!(read_back, token);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = fs::metadata(&token_path).unwrap();
            let mode = metadata.permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        let _ = fs::remove_dir_all(test_dir);
    }
}
