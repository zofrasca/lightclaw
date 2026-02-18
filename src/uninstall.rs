use crate::service::{self, RuntimeStatus, Scope};
use anyhow::{Context, Result};
use cliclack::{confirm, input, intro, log, outro, outro_cancel};
use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

struct UninstallPaths {
    config: PathBuf,
    data: PathBuf,
    workspace: PathBuf,
    logs: PathBuf,
}

pub fn run() -> Result<()> {
    let paths = uninstall_paths();

    intro("lightclaw uninstall")?;
    let proceed = confirm(
        "This will remove the background service and may delete local files/binary. Continue?",
    )
    .initial_value(false)
    .interact()?;
    if !proceed {
        outro_cancel("Uninstall cancelled.")?;
        return Ok(());
    }

    log::step("Service").ok();
    log::info("Removing user-level service if present.").ok();
    remove_user_service();

    let remove_local_files = confirm("Delete local files too?")
        .initial_value(false)
        .interact()?;
    if remove_local_files {
        confirm_delete_phrase()?;
        remove_local_paths(&paths)?;
    } else {
        log::info("Keeping config/data/workspace files.").ok();
    }

    let remove_binary = confirm("Remove lightclaw binary from PATH too?")
        .initial_value(false)
        .interact()?;
    if remove_binary {
        remove_binary_file()?;
    } else {
        log::info("Keeping binary installed.").ok();
    }

    outro("Uninstall complete.")?;
    Ok(())
}

fn uninstall_paths() -> UninstallPaths {
    let cfg = crate::config::AppConfig::load_relaxed();
    UninstallPaths {
        config: crate::config::config_path(),
        data: cfg.data_dir,
        workspace: cfg.workspace_dir,
        logs: crate::config::logs_dir(),
    }
}

fn remove_user_service() {
    let scope = Scope::User;
    match service::query_status(scope) {
        Ok(RuntimeStatus::NotInstalled) => {
            log::info("No user-level service found.").ok();
        }
        Ok(_) => {
            if let Err(err) = service::stop(scope) {
                log::info(&format!("Service stop returned: {err}")).ok();
            }
            if let Err(err) = service::uninstall(scope) {
                log::info(&format!("Service uninstall returned: {err}")).ok();
            } else {
                log::success("User-level service removed.").ok();
            }
        }
        Err(err) => {
            log::info(&format!("Could not inspect service state: {err}")).ok();
            if let Err(stop_err) = service::stop(scope) {
                log::info(&format!("Service stop returned: {stop_err}")).ok();
            }
            if let Err(uninstall_err) = service::uninstall(scope) {
                log::info(&format!("Service uninstall returned: {uninstall_err}")).ok();
            }
        }
    }
}

fn confirm_delete_phrase() -> Result<()> {
    let token: String = input("Type DELETE to confirm file removal")
        .validate(|value: &String| {
            if value.trim() == "DELETE" {
                Ok(())
            } else {
                Err("Type DELETE exactly to continue".to_string())
            }
        })
        .interact()?;
    let _ = token;
    Ok(())
}

fn remove_local_paths(paths: &UninstallPaths) -> Result<()> {
    log::step("Local files").ok();
    let mut seen = HashSet::new();

    for (label, path) in [
        ("Config file", paths.config.clone()),
        ("Workspace", paths.workspace.clone()),
        ("Logs", paths.logs.clone()),
        ("Data", paths.data.clone()),
    ] {
        if !seen.insert(path.clone()) {
            continue;
        }
        remove_path(label, &path)?;
    }

    Ok(())
}

fn remove_path(label: &str, path: &Path) -> Result<()> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            log::info(&format!("{label}: not found ({})", path.display())).ok();
            return Ok(());
        }
        Err(err) => {
            return Err(err)
                .with_context(|| format!("failed to read metadata for {}", path.display()));
        }
    };
    let file_type = meta.file_type();

    if file_type.is_symlink() || file_type.is_file() {
        fs::remove_file(path)
            .with_context(|| format!("failed to remove file {}", path.display()))?;
    } else if file_type.is_dir() {
        fs::remove_dir_all(path)
            .with_context(|| format!("failed to remove directory {}", path.display()))?;
    } else {
        fs::remove_file(path)
            .with_context(|| format!("failed to remove path {}", path.display()))?;
    }

    log::success(&format!("{label}: removed ({})", path.display())).ok();
    Ok(())
}

fn remove_binary_file() -> Result<()> {
    log::step("Binary").ok();
    let Some(path) = detect_binary_path() else {
        log::info("Could not detect binary path automatically.").ok();
        return Ok(());
    };

    log::info(&format!("Detected binary: {}", path.display())).ok();
    if is_dev_binary(&path) {
        log::info("Detected development binary path; skipping automatic deletion.").ok();
        log::info("Remove manually if you really want to delete that build.").ok();
        return Ok(());
    }

    remove_path("Binary", &path)
}

fn detect_binary_path() -> Option<PathBuf> {
    let from_path = find_binary_in_path();
    let home_bin = dirs::home_dir().map(|home| home.join(".local").join("bin").join(bin_name()));
    let current = env::current_exe().ok();

    for candidate in [&from_path, &home_bin, &current] {
        if let Some(path) = candidate {
            if path.exists() && !is_dev_binary(path) {
                return Some(path.clone());
            }
        }
    }

    from_path.or(home_bin).or(current)
}

fn find_binary_in_path() -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    for segment in env::split_paths(&path_var) {
        let candidate = segment.join(bin_name());
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn bin_name() -> &'static str {
    if cfg!(windows) {
        "lightclaw.exe"
    } else {
        "lightclaw"
    }
}

fn is_dev_binary(path: &Path) -> bool {
    let raw = path.to_string_lossy();
    raw.contains("/target/debug/")
        || raw.contains("/target/release/")
        || raw.contains("\\target\\debug\\")
        || raw.contains("\\target\\release\\")
}
