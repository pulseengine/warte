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
    // REQ-DEPLOY-003: every command reads the config --config names
    let config_path = cli.config.clone();

    match cli.command {
        Commands::Serve { bind } => {
            let mut config = Config::load_at(config_path.as_deref())?;
            if let Some(bind) = bind {
                config.bind = bind;
            }
            let db = Database::connect(&config.db_path).await?;
            server::serve(config, db).await?;
        }

        Commands::Status => {
            let config = Config::load_at(config_path.as_deref())?;
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
            let config = Config::load_at(config_path.as_deref())?;
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

        Commands::Watch {
            ci: _,
            repo,
            pr,
            branch,
            since,
            timeout,
            url,
            ca_cert,
        } => {
            let base = match url {
                Some(u) => u.trim_end_matches('/').to_string(),
                None => format!("http://{}", Config::load_at(config_path.as_deref())?.bind),
            };
            let code = warte::watch::client::run(
                &base,
                &repo,
                pr,
                branch.as_deref(),
                since,
                timeout,
                ca_cert.as_deref(),
            )
            .await?;
            std::process::exit(code);
        }

        Commands::Fleet { action } => match action {
            warte::cli::FleetCommands::Show { instance } => {
                let text = std::fs::read_to_string(&instance)
                    .map_err(|e| anyhow::anyhow!("{}: {e}", instance.display()))?;
                let f = warte::fleet::from_spar(&text).map_err(anyhow::Error::msg)?;
                match cli.format {
                    OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&f)?),
                    OutputFormat::Text => {
                        println!("{}", f.root);
                        for m in &f.machines {
                            let boards: Vec<&str> =
                                m.boards.iter().map(|b| b.catalog.as_str()).collect();
                            println!(
                                "  {:<20} {:<12} {:<7} {}{}",
                                m.host,
                                m.kind,
                                m.location,
                                if m.ground_station {
                                    "ground station "
                                } else {
                                    ""
                                },
                                boards.join(", ")
                            );
                        }
                        for p in &f.problems {
                            println!("  problem: {p}");
                        }
                    }
                }
                if !f.problems.is_empty() {
                    std::process::exit(1);
                }
            }
        },

        Commands::Token { action } => match action {
            warte::cli::TokenCommands::Mint {
                agent_id,
                valid_hours,
                any_agent,
                mut rights,
            } => {
                let config = Config::load_at(config_path.as_deref())?;
                if any_agent {
                    rights.push(warte::auth::RIGHT_ANY_AGENT.to_string());
                }
                let t =
                    warte::auth::mint_agent_token_with(&config, &agent_id, valid_hours, &rights)?;
                println!("{t}");
            }
        },

        Commands::Probe { action } => match action {
            warte::cli::ProbeCommands::Token { host } => {
                let config = Config::load_at(config_path.as_deref())?;
                let db = Database::connect(&config.db_path).await?;
                let t =
                    warte::probe::enroll::mint(&db, &host, chrono::Utc::now().timestamp()).await?;
                println!("{t}");
                eprintln!(
                    "one-time token for {host}, valid {} min. On {host}: warte probe enroll <token> --ground <url>",
                    warte::probe::enroll::TOKEN_TTL_S / 60
                );
            }
            warte::cli::ProbeCommands::Revoke { host } => {
                let config = Config::load_at(config_path.as_deref())?;
                let db = Database::connect(&config.db_path).await?;
                warte::probe::enroll::revoke(&db, &host, chrono::Utc::now().timestamp()).await?;
                eprintln!(
                    "{host} revoked: its reports and renewals are refused until it enrolls with a new token (warte probe token --host {host})"
                );
            }
            warte::cli::ProbeCommands::Enroll {
                token,
                ground,
                host,
                state_dir,
                ca_cert,
            } => {
                let host = host
                    .or_else(sysinfo::System::host_name)
                    .ok_or_else(|| anyhow::anyhow!("cannot tell this host's name; pass --host"))?;
                warte::probe::enroll::enroll(
                    &ground,
                    &token,
                    &host,
                    &state_dir,
                    ca_cert.as_deref(),
                )
                .await?;
                println!(
                    "enrolled {host}: key, certificate and CA in {}",
                    state_dir.display()
                );
            }
            warte::cli::ProbeCommands::Send {
                ground,
                state_dir,
                ca_cert,
                feeds,
            } => {
                warte::probe::send::send(&ground, &state_dir, ca_cert.as_deref(), &feeds).await?;
            }
            warte::cli::ProbeCommands::Privileged { out } => {
                let (p, errors) =
                    warte::probe::hardware::privileged_collect(chrono::Utc::now().timestamp())
                        .await;
                for e in &errors {
                    eprintln!("warte probe privileged: {e}");
                }
                warte::probe::hardware::write_privileged(&out, &p)?;
                println!("{} disk(s) written to {}", p.disks.len(), out.display());
                // nothing read: fail, so the unit shows it (the file says so too)
                if warte::probe::hardware::privileged_failed(&p) {
                    std::process::exit(1);
                }
            }
            warte::cli::ProbeCommands::InstallHooks { yes } => {
                use warte::probe::hooks;
                let home = std::env::var_os("HOME")
                    .map(std::path::PathBuf::from)
                    .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
                let exe = std::env::current_exe()?.canonicalize()?;
                let changes = hooks::plan(&home, &exe.to_string_lossy())?;
                if changes.is_empty() {
                    println!("Every agent found here already has the warte hook.");
                    return Ok(());
                }
                for c in &changes {
                    println!(
                        "{} — {}{}",
                        c.agent,
                        c.path.display(),
                        if c.before.is_none() {
                            " (new file)"
                        } else {
                            ""
                        }
                    );
                    print!("{}", hooks::diff(c.before.as_deref(), &c.after));
                    println!();
                }
                if !yes {
                    use std::io::Write;
                    print!("Write {} file(s)? [y/N] ", changes.len());
                    std::io::stdout().flush()?;
                    let mut answer = String::new();
                    std::io::stdin().read_line(&mut answer)?;
                    if !matches!(answer.trim(), "y" | "Y" | "yes") {
                        println!("Nothing written.");
                        return Ok(());
                    }
                }
                let now = chrono::Utc::now().timestamp();
                for c in &changes {
                    match hooks::apply(c, now)? {
                        Some(b) => println!("wrote {} (backup: {})", c.path.display(), b.display()),
                        None => println!("wrote {}", c.path.display()),
                    }
                }
            }
            warte::cli::ProbeCommands::Hook { event } => {
                use std::io::Read;
                let mut s = String::new();
                let _ = std::io::stdin().take(1 << 20).read_to_string(&mut s);
                warte::probe::activity::hook_as(
                    &s,
                    event.as_deref(),
                    chrono::Utc::now().timestamp(),
                );
            }
            warte::cli::ProbeCommands::Pressure {
                format,
                measure,
                state_dir,
            } => {
                let state_dir = state_dir.unwrap_or_else(warte::probe::pressure::default_state_dir);
                let out = warte::probe::pressure::local(
                    &state_dir,
                    measure,
                    &warte::probe::pressure::Thresholds::from_env(),
                    chrono::Utc::now().timestamp(),
                );
                match format.as_str() {
                    "json" => println!("{}", serde_json::to_string_pretty(&out)?),
                    "text" => print!("{}", warte::probe::pressure::text(&out)),
                    f => {
                        eprintln!("unknown --format {f:?} (text, json)");
                        std::process::exit(2);
                    }
                }
            }
            warte::cli::ProbeCommands::Report => {
                let r = warte::probe::collect(None).await;
                match cli.format {
                    OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&r)?),
                    OutputFormat::Text => print!("{}", warte::probe::text(&r)),
                }
            }
        },

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
                    let config = Config::load_at(config_path.as_deref())?;
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
            let config = Config::load_at(config_path.as_deref())?;
            let db = Database::connect(&config.db_path).await?;
            let path = std::path::PathBuf::from(&path);

            let result = match source.as_str() {
                "temper" => {
                    println!(
                        "Migrating repo hardening config from temper ({})...",
                        path.display()
                    );
                    warte::migrate::migrate_temper(db.pool(), &path).await?
                }
                "smithy" => {
                    eprintln!(
                        "smithy needs no import: run `warte hub --data-dir <smithy-hub dir>`, which reads it as-is (see `warte docs ground-station`)."
                    );
                    std::process::exit(2);
                }
                _ => {
                    eprintln!("Unknown source: {}. Expected 'temper'.", source);
                    std::process::exit(2);
                }
            };

            match cli.format {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Text => {
                    println!("Migration from {} complete:", result.source);
                    println!("  imported:  {}", result.imported);
                    println!("  skipped:   {}", result.skipped);
                    println!("  errors:    {}", result.errors.len());
                    println!("  checksum:  {}", &result.checksum[..16]);
                    for err in &result.errors {
                        eprintln!("  ERROR: {}", err);
                    }
                }
            }
        }

        Commands::Artifact { action } => {
            use warte::cli::ArtifactCommands;
            match action {
                ArtifactCommands::List => {
                    let config = Config::load_at(config_path.as_deref())?;
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

        Commands::Hub {
            http,
            udp,
            allow,
            data_dir,
            org,
        } => {
            warte::hub::check_allow(&allow).map_err(anyhow::Error::msg)?;
            let hub = warte::hub::Hub::open(&data_dir, &org).map_err(anyhow::Error::msg)?;
            hub.spawn_ingest(&udp, allow).map_err(anyhow::Error::msg)?;
            let listener = tokio::net::TcpListener::bind(&http).await?;
            tracing::info!(%http, %udp, %data_dir, "ground station up");
            axum::serve(listener, hub.router()).await?;
        }

        Commands::Docs { topic, check } => {
            if check {
                if !warte::docs::run_check() {
                    std::process::exit(1);
                }
            } else if let Some(topic) = topic {
                if !warte::docs::print_topic(&topic) {
                    std::process::exit(1);
                }
            } else {
                warte::docs::print_topic_list();
            }
        }
    }

    Ok(())
}
