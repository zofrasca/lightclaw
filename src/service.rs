use crate::config;
use anyhow::{anyhow, Context, Result};
use service_manager::{
    RestartPolicy, ServiceInstallCtx, ServiceLabel, ServiceLevel, ServiceManager, ServiceStartCtx,
    ServiceStatus, ServiceStatusCtx, ServiceStopCtx, ServiceUninstallCtx,
};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::time::Duration;

const SERVICE_LABEL: &str = "io.lightclaw.agent";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeStatus {
    NotInstalled,
    Running,
    Stopped(Option<String>),
}

#[derive(Clone, Copy, Debug)]
pub enum Scope {
    User,
    System,
}

impl Scope {
    fn level(self) -> ServiceLevel {
        match self {
            Self::User => ServiceLevel::User,
            Self::System => ServiceLevel::System,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::System => "system",
        }
    }
}

pub fn install(scope: Scope) -> Result<()> {
    let label = service_label()?;
    let manager = service_manager_for(scope)?;
    let executable = env::current_exe().context("failed to resolve current lightclaw binary")?;
    warn_if_dev_binary(&executable);
    let working_directory = dirs::home_dir().or_else(|| env::current_dir().ok());

    manager
        .install(ServiceInstallCtx {
            label: label.clone(),
            program: executable,
            args: vec![OsString::from("run")],
            contents: None,
            username: None,
            working_directory,
            environment: service_environment(),
            autostart: true,
            restart_policy: RestartPolicy::OnFailure {
                delay_secs: Some(5),
                max_retries: None,
                reset_after_secs: None,
            },
        })
        .with_context(|| format!("failed to install service '{label}'"))?;

    manager
        .start(ServiceStartCtx {
            label: label.clone(),
        })
        .with_context(|| format!("service '{label}' installed but failed to start"))?;

    println!(
        "Installed and started '{label}' at {} level.",
        scope.as_str()
    );
    println!("Logs: {}", config::log_file_path().display());
    Ok(())
}

pub fn uninstall(scope: Scope) -> Result<()> {
    let label = service_label()?;
    let manager = service_manager_for(scope)?;

    let _ = manager.stop(ServiceStopCtx {
        label: label.clone(),
    });

    manager
        .uninstall(ServiceUninstallCtx {
            label: label.clone(),
        })
        .with_context(|| format!("failed to uninstall service '{label}'"))?;

    println!("Uninstalled '{label}' from {} level.", scope.as_str());
    Ok(())
}

pub fn start(scope: Scope) -> Result<()> {
    let label = service_label()?;
    let manager = service_manager_for(scope)?;
    manager
        .start(ServiceStartCtx {
            label: label.clone(),
        })
        .with_context(|| format!("failed to start service '{label}'"))?;
    println!("Started '{label}' at {} level.", scope.as_str());
    Ok(())
}

pub fn stop(scope: Scope) -> Result<()> {
    let label = service_label()?;
    let manager = service_manager_for(scope)?;
    manager
        .stop(ServiceStopCtx {
            label: label.clone(),
        })
        .with_context(|| format!("failed to stop service '{label}'"))?;
    println!("Stopped '{label}' at {} level.", scope.as_str());
    Ok(())
}

pub fn restart(scope: Scope) -> Result<()> {
    let label = service_label()?;
    let manager = service_manager_for(scope)?;

    if let Err(err) = manager.stop(ServiceStopCtx {
        label: label.clone(),
    }) {
        eprintln!("warning: stop before restart returned: {err}");
    }

    manager
        .start(ServiceStartCtx {
            label: label.clone(),
        })
        .with_context(|| format!("failed to restart service '{label}'"))?;
    println!("Restarted '{label}' at {} level.", scope.as_str());
    Ok(())
}

pub fn status(scope: Scope) -> Result<()> {
    let label = service_label()?;
    let status = query_status(scope)?;

    match status {
        RuntimeStatus::Running => println!("'{label}' is running at {} level.", scope.as_str()),
        RuntimeStatus::Stopped(reason) => {
            if let Some(reason) = reason {
                println!(
                    "'{label}' is stopped at {} level: {}",
                    scope.as_str(),
                    reason
                );
            } else {
                println!("'{label}' is stopped at {} level.", scope.as_str());
            }
        }
        RuntimeStatus::NotInstalled => {
            println!("'{label}' is not installed at {} level.", scope.as_str());
        }
    }

    Ok(())
}

pub fn query_status(scope: Scope) -> Result<RuntimeStatus> {
    let label = service_label()?;
    let manager = service_manager_for(scope)?;
    let status = manager
        .status(ServiceStatusCtx { label })
        .context("failed to read service status")?;

    Ok(match status {
        ServiceStatus::NotInstalled => RuntimeStatus::NotInstalled,
        ServiceStatus::Running => RuntimeStatus::Running,
        ServiceStatus::Stopped(reason) => RuntimeStatus::Stopped(reason),
    })
}

pub async fn logs(lines: usize, follow: bool) -> Result<()> {
    let path = config::log_file_path();
    if !path.exists() {
        println!("No log file found at {}", path.display());
        return Ok(());
    }

    print_last_lines(&path, lines)?;

    if !follow {
        return Ok(());
    }

    println!("\nFollowing logs (Ctrl+C to stop)...");
    let mut offset = fs::metadata(&path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("\nStopped following logs.");
                break;
            }
            _ = tokio::time::sleep(Duration::from_millis(800)) => {
                if !path.exists() {
                    continue;
                }

                let current_len = match fs::metadata(&path) {
                    Ok(metadata) => metadata.len(),
                    Err(_) => continue,
                };

                if current_len < offset {
                    offset = 0;
                }

                if current_len > offset {
                    let (chunk, new_offset) = read_from_offset(&path, offset)?;
                    if !chunk.is_empty() {
                        print!("{chunk}");
                        std::io::stdout().flush()?;
                    }
                    offset = new_offset;
                }
            }
        }
    }

    Ok(())
}

fn service_label() -> Result<ServiceLabel> {
    SERVICE_LABEL
        .parse::<ServiceLabel>()
        .map_err(|err| anyhow!("invalid service label '{SERVICE_LABEL}': {err}"))
}

fn service_manager_for(scope: Scope) -> Result<Box<dyn ServiceManager>> {
    let mut manager =
        <dyn ServiceManager>::native().context("failed to detect native service manager")?;

    manager.set_level(scope.level()).with_context(|| {
        format!(
            "service manager does not support {}-level services on this platform",
            scope.as_str()
        )
    })?;

    if !manager
        .available()
        .context("failed to verify service manager availability")?
    {
        return Err(anyhow!(
            "native service manager is unavailable on this platform"
        ));
    }

    Ok(manager)
}

fn service_environment() -> Option<Vec<(String, String)>> {
    let mut vars = Vec::new();
    for key in ["RUST_LOG", "LIGHTCLAW_DATA_DIR", "LIGHTCLAW_WORKSPACE_DIR"] {
        if let Ok(value) = env::var(key) {
            vars.push((key.to_string(), value));
        }
    }

    if vars.is_empty() {
        None
    } else {
        Some(vars)
    }
}

fn print_last_lines(path: &Path, lines: usize) -> Result<()> {
    if lines == 0 {
        return Ok(());
    }

    let file = fs::File::open(path)
        .with_context(|| format!("failed to open log file at {}", path.display()))?;
    let mut selected = std::collections::VecDeque::with_capacity(lines);

    for line in BufReader::new(file).lines() {
        let line =
            line.with_context(|| format!("failed to read log file at {}", path.display()))?;
        if selected.len() == lines {
            selected.pop_front();
        }
        selected.push_back(line);
    }

    if !selected.is_empty() {
        let output = selected.into_iter().collect::<Vec<String>>().join("\n");
        println!("{output}");
    }

    Ok(())
}

fn read_from_offset(path: &Path, offset: u64) -> Result<(String, u64)> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open log file at {}", path.display()))?;
    file.seek(SeekFrom::Start(offset))
        .with_context(|| format!("failed to seek log file at {}", path.display()))?;

    let mut out = String::new();
    file.read_to_string(&mut out)
        .with_context(|| format!("failed to read log file at {}", path.display()))?;
    let new_offset = file.stream_position()?;

    Ok((out, new_offset))
}

fn warn_if_dev_binary(path: &Path) {
    let raw = path.to_string_lossy();
    if raw.contains("/target/debug/") || raw.contains("\\target\\debug\\") {
        eprintln!(
            "warning: installing service from a debug build path ({}). Use a release binary for production.",
            path.display()
        );
    }
}
