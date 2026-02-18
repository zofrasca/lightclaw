mod agent;
mod bus;
mod channels;
mod config;
mod configure;
mod cron;
mod memory;
mod providers;
mod service;
mod session_compaction;
mod skills;
mod tools;
mod transcription;
mod uninstall;

use anyhow::{anyhow, Result};
use clap::{CommandFactory, Parser, Subcommand};
use tokio::io::{self, AsyncBufReadExt};
use tracing::{info, warn};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[command(name = "lightclaw", version, about = "lightclaw CLI")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    Run,
    Tui,
    Configure,
    Uninstall,
    Skills {
        #[command(subcommand)]
        command: skills::cli::SkillsCommands,
    },
    Cron {
        /// Admin cron operations (tool-driven scheduling is preferred)
        #[command(subcommand)]
        command: CronCommands,
    },
    Service {
        /// Manage lightclaw as a background service
        #[command(subcommand)]
        command: ServiceCommands,
    },
}

#[derive(Subcommand)]
enum CronCommands {
    List,
    Status,
    Remove {
        #[arg(long)]
        id: String,
    },
}

#[derive(Subcommand)]
enum ServiceCommands {
    Install {
        /// Use the system service level (admin/root)
        #[arg(long, default_value_t = false)]
        system: bool,
    },
    Uninstall {
        /// Use the system service level (admin/root)
        #[arg(long, default_value_t = false)]
        system: bool,
    },
    Start {
        /// Use the system service level (admin/root)
        #[arg(long, default_value_t = false)]
        system: bool,
    },
    Stop {
        /// Use the system service level (admin/root)
        #[arg(long, default_value_t = false)]
        system: bool,
    },
    Restart {
        /// Use the system service level (admin/root)
        #[arg(long, default_value_t = false)]
        system: bool,
    },
    Status {
        /// Use the system service level (admin/root)
        #[arg(long, default_value_t = false)]
        system: bool,
    },
    Logs {
        /// Follow logs live (like tail -f)
        #[arg(short = 'f', long, default_value_t = false)]
        follow: bool,
        /// Number of lines to print before following
        #[arg(long, default_value_t = 200)]
        lines: usize,
    },
}

pub async fn run_cli() -> Result<()> {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        let mut cmd = Cli::command();
        cmd.print_help()?;
        println!();
        return Ok(());
    };
    let write_runtime_logs = matches!(&command, Commands::Run | Commands::Tui);
    init_logging(write_runtime_logs);

    match command {
        Commands::Run => run().await,
        Commands::Tui => run_tui().await,
        Commands::Configure => configure::run(),
        Commands::Uninstall => uninstall::run(),
        Commands::Skills { command } => {
            tokio::task::spawn_blocking(move || skills::cli::handle_skills(command))
                .await
                .map_err(|err| anyhow!("skills command task failed: {err}"))?
        }
        Commands::Cron { command } => handle_cron(command).await,
        Commands::Service { command } => handle_service(command).await,
    }
}

async fn run() -> Result<()> {
    let cfg = config::AppConfig::load()?;

    let bus = bus::MessageBus::new();

    // Start Cron Service
    let cron_service = cron::CronService::new(&cfg, bus.clone());
    cron_service.start().await;

    let agent = agent::AgentLoop::new(cfg.clone(), bus.clone(), cron_service.clone());
    tokio::spawn(async move {
        agent.run().await;
    });

    let mut enabled_channels = 0usize;

    if cfg.telegram_enabled() {
        enabled_channels += 1;
        let telegram_cfg = cfg.clone();
        let telegram_bus = bus.clone();
        tokio::spawn(async move {
            if let Err(err) = channels::telegram::start(telegram_cfg, telegram_bus).await {
                warn!("telegram disabled: {err}");
            }
        });
    } else {
        info!("Telegram token not configured; running without Telegram input/output");
        info!("Set TELOXIDE_TOKEN or channels.telegram.token to enable Telegram");
    }

    if cfg.discord_enabled() {
        enabled_channels += 1;
        let discord_cfg = cfg.clone();
        let discord_bus = bus.clone();
        tokio::spawn(async move {
            if let Err(err) = channels::discord::start(discord_cfg, discord_bus).await {
                warn!("discord disabled: {err}");
            }
        });
    } else {
        info!("Discord token not configured; running without Discord input/output");
        info!("Set DISCORD_BOT_TOKEN or channels.discord.token to enable Discord");
    }

    if enabled_channels == 0 {
        warn!("lightclaw is running without chat input/output; press Ctrl+C to exit");
    }
    wait_for_shutdown().await?;

    Ok(())
}

async fn wait_for_shutdown() -> Result<()> {
    tokio::signal::ctrl_c().await?;
    Ok(())
}

async fn handle_cron(cmd: CronCommands) -> Result<()> {
    let cfg = config::AppConfig::load()?;
    // We don't need a real bus for CLI operations acting on the store
    let bus = bus::MessageBus::new();
    let service = cron::CronService::new(&cfg, bus);

    match cmd {
        CronCommands::List => {
            let jobs = service.list_jobs().await?;
            if jobs.is_empty() {
                println!("No cron jobs found.");
            } else {
                println!(
                    "{:<10} {:<20} {:<20} {:<10} {:<20}",
                    "ID", "Name", "Schedule", "Status", "Next Run"
                );
                println!("{:-<80}", "");
                for job in jobs {
                    let next = job
                        .state
                        .next_run_at_ms
                        .map(|ms| {
                            chrono::DateTime::<chrono::Utc>::from(
                                std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms as u64),
                            )
                            .to_rfc3339()
                        })
                        .unwrap_or_else(|| "N/A".to_string());
                    let schedule_str = if job.schedule.kind == "every" {
                        format!("every {}ms", job.schedule.every_ms.unwrap_or(0))
                    } else if job.schedule.kind == "at" {
                        "at specific time".to_string()
                    } else {
                        job.schedule.expr.clone().unwrap_or("?".to_string())
                    };

                    println!(
                        "{:<10} {:<20} {:<20} {:<10} {:<20}",
                        job.id,
                        job.name,
                        schedule_str,
                        if job.enabled { "Enabled" } else { "Disabled" },
                        next
                    );
                }
            }
        }
        CronCommands::Status => {
            let status = service.status().await?;
            let next = status
                .next_wake_at_ms
                .map(|ms| {
                    chrono::DateTime::<chrono::Utc>::from(
                        std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms as u64),
                    )
                    .to_rfc3339()
                })
                .unwrap_or_else(|| "N/A".to_string());
            println!("Jobs: {}", status.jobs);
            println!("Enabled jobs: {}", status.enabled_jobs);
            println!("Next wake: {}", next);
        }
        CronCommands::Remove { id } => match service.remove_job(&id).await {
            Ok(true) => println!("Job removed."),
            Ok(false) => println!("Job not found."),
            Err(e) => println!("Error removing job: {}", e),
        },
    }
    Ok(())
}

async fn handle_service(cmd: ServiceCommands) -> Result<()> {
    let scope = |system: bool| {
        if system {
            service::Scope::System
        } else {
            service::Scope::User
        }
    };

    match cmd {
        ServiceCommands::Install { system } => service::install(scope(system)),
        ServiceCommands::Uninstall { system } => service::uninstall(scope(system)),
        ServiceCommands::Start { system } => service::start(scope(system)),
        ServiceCommands::Stop { system } => service::stop(scope(system)),
        ServiceCommands::Restart { system } => service::restart(scope(system)),
        ServiceCommands::Status { system } => service::status(scope(system)),
        ServiceCommands::Logs { follow, lines } => service::logs(lines, follow).await,
    }
}

async fn run_tui() -> Result<()> {
    let cfg = config::AppConfig::load()?;
    let bus = bus::MessageBus::new();

    let cron_service = cron::CronService::new(&cfg, bus.clone());
    cron_service.start().await;

    let agent = agent::AgentLoop::new(cfg, bus.clone(), cron_service);
    tokio::spawn(async move {
        agent.run().await;
    });

    let bus_for_outbound = bus.clone();
    tokio::spawn(async move {
        let mut outbound_rx = bus_for_outbound.subscribe_outbound();
        loop {
            let msg = match outbound_rx.recv().await {
                Ok(msg) => msg,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            };
            if msg.channel != "tui" {
                continue;
            }
            println!("\nassistant> {}\n", msg.content.trim());
        }
    });

    println!("lightclaw TUI mode");
    println!("Type messages and press Enter. Type /exit to quit.\n");

    let mut lines = io::BufReader::new(io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let content = line.trim().to_string();
        if content.is_empty() {
            continue;
        }
        if content == "/exit" {
            break;
        }
        bus.publish_inbound(bus::InboundMessage {
            channel: "tui".to_string(),
            chat_id: "local".to_string(),
            sender_id: "local".to_string(),
            content,
        })
        .await;
    }

    Ok(())
}

fn init_logging(write_runtime_logs: bool) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .compact();

    if write_runtime_logs {
        let log_path = config::log_file_path();
        if let Some(log_dir) = log_path.parent() {
            if let Err(err) = std::fs::create_dir_all(log_dir) {
                eprintln!(
                    "warning: failed to create log directory {}: {}",
                    log_dir.display(),
                    err
                );
            } else {
                let file_appender = tracing_appender::rolling::never(log_dir, "lightclaw.log");
                let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
                keep_logging_guard(guard);

                tracing_subscriber::registry()
                    .with(filter)
                    .with(stdout_layer)
                    .with(
                        tracing_subscriber::fmt::layer()
                            .with_ansi(false)
                            .with_target(true)
                            .compact()
                            .with_writer(non_blocking),
                    )
                    .init();
                return;
            }
        }
    }

    tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .init();
}

fn keep_logging_guard(guard: WorkerGuard) {
    use std::sync::{Mutex, OnceLock};
    static GUARDS: OnceLock<Mutex<Vec<WorkerGuard>>> = OnceLock::new();
    let bucket = GUARDS.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut guards) = bucket.lock() {
        guards.push(guard);
    }
}
