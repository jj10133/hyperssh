use anyhow::{Context, Result, bail};
use std::path::PathBuf;

fn config_path() -> Result<PathBuf> {
    Ok(dirs::home_dir()
        .context("Could not determine home directory")?
        .join(".ssh")
        .join("config"))
}

fn begin(name: &str) -> String {
    format!("# >>> hyperssh:{name}")
}
fn end(name: &str) -> String {
    format!("# <<< hyperssh:{name}")
}

fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !ok {
        bail!("Route name may only contain letters, digits, '-', '_' and '.'");
    }
    Ok(())
}

fn exe_for_proxy() -> String {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "hyperssh".to_string());
    if exe.contains(' ') {
        format!("\"{exe}\"")
    } else {
        exe
    }
}

async fn read_config() -> Result<(PathBuf, String)> {
    let path = config_path()?;
    let content = match tokio::fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).context("Failed to read ~/.ssh/config"),
    };
    Ok((path, content))
}

async fn write_config(path: &PathBuf, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ =
                tokio::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).await;
        }
    }
    tokio::fs::write(path, content)
        .await
        .context("Failed to write ~/.ssh/config")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await;
    }
    Ok(())
}

/// Remove a managed block, returning the new content and whether one was removed.
fn strip_block(content: &str, name: &str) -> (String, bool) {
    let (b, e) = (begin(name), end(name));
    let mut out = Vec::new();
    let mut skipping = false;
    let mut removed = false;
    for line in content.lines() {
        if line.trim() == b {
            skipping = true;
            removed = true;
            continue;
        }
        if skipping {
            if line.trim() == e {
                skipping = false;
            }
            continue;
        }
        // Collapse runs of blank lines left behind by earlier add/remove cycles.
        if line.trim().is_empty() && out.last().is_some_and(|l: &&str| l.trim().is_empty()) {
            continue;
        }
        out.push(line);
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    let mut s = out.join("\n");
    if !s.is_empty() {
        s.push('\n');
    }
    (s, removed)
}

pub async fn add(name: &str, id_hex: &str) -> Result<()> {
    validate_name(name)?;
    crate::identity::decode_public_key(id_hex)?;

    let (path, content) = read_config().await?;
    // Re-adding replaces the existing route rather than duplicating it.
    let (mut content, _) = strip_block(&content, name);

    if !content.is_empty() {
        content.push('\n'); // one blank line between entries
    }
    content.push_str(&format!(
        "{}\nHost {name}\n    ProxyCommand {} connect --stdio {id_hex}\n{}\n",
        begin(name),
        exe_for_proxy(),
        end(name),
    ));

    write_config(&path, &content).await?;
    println!("Route '{name}' added to {}", path.display());
    println!("Connect with: ssh user@{name}");
    Ok(())
}

pub async fn list() -> Result<()> {
    let (_, content) = read_config().await?;
    let mut found = false;
    let mut current: Option<String> = None;
    for line in content.lines() {
        let t = line.trim();
        if let Some(name) = t.strip_prefix("# >>> hyperssh:") {
            current = Some(name.to_string());
        } else if let Some(name) = &current {
            // Also recognise entries written by earlier versions (`fly --stdio`).
            for marker in ["connect --stdio ", "fly --stdio "] {
                if let Some(idx) = t.find(marker) {
                    println!("{name}\t{}", &t[idx + marker.len()..]);
                    found = true;
                    break;
                }
            }
        } else if t.starts_with("# <<< hyperssh:") {
            current = None;
        }
    }
    if !found {
        println!("No hyperssh routes.");
    }
    Ok(())
}

pub async fn remove(name: &str) -> Result<()> {
    let (path, content) = read_config().await?;
    let (new_content, removed) = strip_block(&content, name);
    if !removed {
        bail!("No hyperssh route named '{name}'");
    }
    write_config(&path, &new_content).await?;
    println!("Route '{name}' removed.");
    Ok(())
}
