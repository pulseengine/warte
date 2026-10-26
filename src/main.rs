//! warte — the PulseEngine control tower.
//!
//! A CI dashboard with an ask feature, agent orchestration, access control,
//! multi-instance sync, and mobile access.
//!
//! German: "watchtower" — and "bitte warte auf mich", "please wait for me".

use clap::Parser;
use warte::cli::{Cli, Commands, OutputFormat};
use warte::{db::Database, server, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warte=info,tower_http=info".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Serve { bind } => {
            let mut config = Config::load()?;
            if let Some(bind) = bind {
                config.bind = bind;
            }
            let db = Database::connect(&config.db_path).await?;
            server::serve(config, db).await?;
        }

        Commands::Status => {
            let config = Config::load()?;
            let client = reqwest::Client::new();
            let url = format!("http://{}/api/health", config.bind);
            match client.get(&url).send().await {
                Ok(resp) => {
                    let health: serde_json::Value = resp.json().await?;
                    match cli.format {
                        OutputFormat::Json => {
                            println!("{}", serde_json::to_string_pretty(&health)?)
                        }
                        OutputFormat::Text => {
                            println!("warte control tower");
                            println!("  status:    {}", health["status"]);
                            println!("  instance:  {}", health["instance_id"]);
                            println!("  version:   {}", health["version"]);
                            println!("  timestamp: {}", health["timestamp"]);
                        }
                    }
                }
                Err(_) => {
                    eprintln!("warte is not running at {}", config.bind);
                    std::process::exit(1);
                }
            }
        }

        Commands::Agents { health } => {
            let config = Config::load()?;
            let client = reqwest::Client::new();
            let url = format!("http://{}/api/agents", config.bind);
            let resp = client.get(&url).send().await?;
            let agents: Vec<serde_json::Value> = resp.json().await?;

            match cli.format {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&agents)?),
                OutputFormat::Text => {
                    println!(
                        "{:<38} {:<12} {:<10} LAST SEEN",
                        "AGENT ID", "NAME", "HEALTH"
                    );
                    for agent in &agents {
                        if let Some(ref h) = health {
                            if agent["health_status"].as_str() != Some(h) {
                                continue;
                            }
                        }
                        println!(
                            "{:<38} {:<12} {:<10} {}",
                            agent["id"].as_str().unwrap_or(""),
                            agent["name"].as_str().unwrap_or(""),
                            agent["health_status"].as_str().unwrap_or(""),
                            agent["last_heartbeat"].as_str().unwrap_or("never"),
                        );
                    }
                }
            }
        }

        Commands::Schedule => {
            println!("Weekly cron schedule");
            println!("  (not yet implemented — see https://github.com/pulseengine/warte/issues)");
        }

        Commands::Watch => {
            let config = Config::load()?;
            let url = format!("http://{}/dashboard", config.bind);
            println!("Opening {} in browser...", url);
            // TODO: open browser
        }

        Commands::Access { action } => {
            use warte::cli::AccessCommands;
            match action {
                AccessCommands::Grant { agent_id, rights } => {
                    println!("Granting {} to agent {}", rights, agent_id);
                    // TODO: call API
                }
                AccessCommands::Revoke { agent_id, rights } => {
                    println!("Revoking {} from agent {}", rights, agent_id);
                    // TODO: call API
                }
                AccessCommands::List { agent_id } => {
                    let config = Config::load()?;
                    let client = reqwest::Client::new();
                    let url = format!("http://{}/api/access/list/{}", config.bind, agent_id);
                    let resp = client.get(&url).send().await?;
                    let rights: Vec<serde_json::Value> = resp.json().await?;
                    match cli.format {
                        OutputFormat::Json => {
                            println!("{}", serde_json::to_string_pretty(&rights)?)
                        }
                        OutputFormat::Text => {
                            for right in &rights {
                                println!("  {}:{}", right["resource"], right["action"]);
                            }
                        }
                    }
                }
            }
        }

        Commands::Migrate { source, path } => {
            println!("Migrating from {} (path: {})", source, path);
            match source.as_str() {
                "smithy" => {
                    println!("  Importing agent fleet data from smithy...");
                    // TODO: implement smithy migration
                }
                "temper" => {
                    println!("  Importing repo hardening config from temper...");
                    // TODO: implement temper migration
                }
                _ => {
                    eprintln!("Unknown source: {}. Expected 'smithy' or 'temper'.", source);
                    std::process::exit(1);
                }
            }
        }

        Commands::Artifact { action } => {
            use warte::cli::ArtifactCommands;
            match action {
                ArtifactCommands::List => {
                    let config = Config::load()?;
                    let client = reqwest::Client::new();
                    let url = format!("http://{}/api/artifacts", config.bind);
                    let resp = client.get(&url).send().await?;
                    let artifacts: Vec<serde_json::Value> = resp.json().await?;
                    match cli.format {
                        OutputFormat::Json => {
                            println!("{}", serde_json::to_string_pretty(&artifacts)?)
                        }
                        OutputFormat::Text => {
                            println!(
                                "{:<38} {:<12} {:<10} HASH",
                                "ARTIFACT ID", "NAME", "VERSION"
                            );
                            for a in &artifacts {
                                println!(
                                    "{:<38} {:<12} {:<10} {}",
                                    a["id"].as_str().unwrap_or(""),
                                    a["name"].as_str().unwrap_or(""),
                                    a["version"].as_str().unwrap_or(""),
                                    &a["content_hash"].as_str().unwrap_or("")[..12],
                                );
                            }
                        }
                    }
                }
                ArtifactCommands::Upload {
                    name,
                    version,
                    path,
                } => {
                    println!("Uploading {} v{} from {}", name, version, path);
                    // TODO: implement upload
                }
                ArtifactCommands::Download {
                    artifact_id,
                    output,
                } => {
                    println!("Downloading {} to {}", artifact_id, output);
                    // TODO: implement download
                }
            }
        }
    }

    Ok(())
}
