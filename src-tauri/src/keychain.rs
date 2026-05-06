use anyhow::{anyhow, Context, Result};
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;

pub async fn save_password(
    app: &AppHandle,
    service: &str,
    account: &str,
    password: &str,
) -> Result<()> {
    let output = app
        .shell()
        .command("/usr/bin/security")
        .args([
            "add-generic-password",
            "-U",
            "-s",
            service,
            "-a",
            account,
            "-w",
            password,
        ])
        .output()
        .await
        .context("failed to execute security add-generic-password")?;

    if !output.status.success() {
        return Err(anyhow!(
            "keychain write failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(())
}

pub async fn read_password(
    app: &AppHandle,
    service: &str,
    account: &str,
) -> Result<Option<String>> {
    let output = app
        .shell()
        .command("/usr/bin/security")
        .args(["find-generic-password", "-s", service, "-a", account, "-w"])
        .output()
        .await
        .context("failed to execute security find-generic-password")?;

    if !output.status.success() {
        return Ok(None);
    }

    let password = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if password.is_empty() {
        Ok(None)
    } else {
        Ok(Some(password))
    }
}

pub async fn delete_password(app: &AppHandle, service: &str, account: &str) -> Result<()> {
    let output = app
        .shell()
        .command("/usr/bin/security")
        .args(["delete-generic-password", "-s", service, "-a", account])
        .output()
        .await
        .context("failed to execute security delete-generic-password")?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("could not be found") {
        return Ok(());
    }

    Err(anyhow!("keychain delete failed: {}", stderr.trim()))
}
