use anyhow::{Context, Result};
use peeroxide::KeyPair;
use std::path::{Path, PathBuf};

/// `~/.config/hyperssh` (or the platform equivalent).
pub fn config_dir() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("Could not determine config directory")?
        .join("hyperssh"))
}

pub fn default_roost_key_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("roost.key"))
}

pub fn public_key_hex(key_pair: &KeyPair) -> String {
    hex::encode(key_pair.public_key)
}

pub async fn save(path: &Path, key_pair: &KeyPair) -> Result<()> {
    let seed = &key_pair.secret_key[..32];

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .context("Failed to create parent directories")?;
    }

    tokio::fs::write(path, seed)
        .await
        .context("Failed to write identity file")?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .await
            .context("Failed to set secure file permissions")?;
    }

    Ok(())
}

pub async fn load(path: &Path) -> Result<KeyPair> {
    let seed = tokio::fs::read(path)
        .await
        .with_context(|| format!("Could not read identity file {}", path.display()))?;

    let seed: [u8; 32] = seed
        .try_into()
        .map_err(|_| anyhow::anyhow!("Identity seed must contain exactly 32 bytes"))?;

    Ok(KeyPair::from_seed(seed))
}

/// Load the key at `path`, or generate and persist a new one if it doesn't exist.
/// Returns the key pair and whether it was freshly created.
pub async fn load_or_create(path: &Path) -> Result<(KeyPair, bool)> {
    if tokio::fs::try_exists(path).await.unwrap_or(false) {
        Ok((load(path).await?, false))
    } else {
        let key_pair = KeyPair::generate();
        save(path, &key_pair).await?;
        Ok((key_pair, true))
    }
}

pub fn decode_public_key(s: &str) -> Result<[u8; 32]> {
    let decoded = hex::decode(s.trim()).context("Invalid hexadecimal public key")?;
    decoded
        .try_into()
        .map_err(|_| anyhow::anyhow!("Public key must be 32 bytes (64 hex characters)"))
}