//! Authentication — token-based auth for IPC connections.

use anyhow::{Context, Result};
use rand::Rng;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::info;

const TOKEN_LENGTH: usize = 32;

/// Generate a cryptographically random auth token.
pub fn generate_token() -> String {
    let mut rng = rand::rng();
    let bytes: Vec<u8> = (0..TOKEN_LENGTH).map(|_| rng.random::<u8>()).collect();
    hex::encode(bytes)
}

/// Get the path to the auth token file.
pub fn token_path(data_dir: &Path) -> PathBuf {
    data_dir.join("auth_token")
}

/// Write the auth token to disk with restricted permissions.
pub fn write_token(data_dir: &Path, token: &str) -> Result<()> {
    fs::create_dir_all(data_dir)
        .with_context(|| format!("Failed to create data directory: {}", data_dir.display()))?;

    let path = token_path(data_dir);
    fs::write(&path, token)
        .with_context(|| format!("Failed to write auth token to {}", path.display()))?;

    // On Windows, file is already user-scoped by default
    // On Unix, we'd set permissions to 0o600
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }

    info!(path = %path.display(), "Auth token written");
    Ok(())
}

/// Read the auth token from disk.
pub fn read_token(data_dir: &Path) -> Result<String> {
    let path = token_path(data_dir);
    let token = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read auth token from {}", path.display()))?;
    Ok(token.trim().to_string())
}
