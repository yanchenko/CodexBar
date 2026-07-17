//! AgentBar CLI (`agentbar`) — usage, providers, version, config patch/set/migrate.
//!
//! Shares `ab-config` / `ab-engine` / `ab-provider` in-process (no C ABI).

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use ab_model::UsageSnapshot;
use serde_json::{Value, json};

fn main() -> ExitCode {
    let mut args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        print_help();
        return ExitCode::SUCCESS;
    }

    // Global flags interleaved before/after subcommand are uncommon; parse simply.
    let cmd = args.remove(0);
    match cmd.as_str() {
        "version" | "--version" | "-V" => {
            println!("agentbar {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        "help" | "--help" | "-h" => {
            print_help();
            ExitCode::SUCCESS
        }
        "usage" => cmd_usage(&args),
        "providers" => cmd_providers(&args),
        "config" => cmd_config(&args),
        "cost" => {
            eprintln!("agentbar cost: not implemented yet (local session scan deferred)");
            ExitCode::from(2)
        }
        other => {
            eprintln!("unknown command: {other}");
            print_help();
            ExitCode::from(2)
        }
    }
}

fn print_help() {
    eprintln!(
        "\
agentbar {ver} — AgentBar CLI

Usage:
  agentbar version
  agentbar usage [--format text|json] [--provider <id>] [--pretty]
  agentbar providers
  agentbar config path
  agentbar config patch --file <path>
  agentbar config set-provider <id> [--enabled true|false] [--source-mode <mode>]
                                    [--api-key-env <VAR>|--api-key -]
                                    [--cookie-header-env <VAR>|--cookie-header -]
  agentbar config migrate

Secrets:
  Never put real keys on argv (process list / shell history). Forms only:
    --api-key-env AGENTBAR_API_KEY     read secret from environment variable
    --api-key -                        read one line from stdin
    --cookie-header-env VAR / -        same for cookie header
  Bare --api-key <value> / --cookie-header <value> are rejected (exit 2).
  Fallback env (when flag omitted): AGENTBAR_API_KEY, AGENTBAR_COOKIE_HEADER.

Exit codes: 0 ok, 1 runtime/error, 2 usage.
",
        ver = env!("CARGO_PKG_VERSION")
    );
}

fn cmd_usage(args: &[String]) -> ExitCode {
    let mut format = "text".to_string();
    let mut provider: Option<String> = None;
    let mut pretty = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--format requires text|json");
                    return ExitCode::from(2);
                }
                format = args[i].clone();
            }
            "--provider" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("--provider requires an id");
                    return ExitCode::from(2);
                }
                provider = Some(args[i].clone());
            }
            "--pretty" => pretty = true,
            "-h" | "--help" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("unknown usage flag: {other}");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }

    // One-shot probe via engine (starts worker, snapshot, stop).
    if !ab_engine::start() {
        eprintln!("failed to start engine");
        return ExitCode::from(1);
    }
    let json_str = ab_engine::snapshot_json();
    let _ = ab_engine::stop();

    let mut snap: UsageSnapshot = match serde_json::from_str(&json_str) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("invalid snapshot json: {e}");
            return ExitCode::from(1);
        }
    };

    if let Some(id) = provider.as_deref() {
        snap.providers.retain(|p| p.id == id);
    }

    match format.as_str() {
        "json" => {
            let out = if pretty {
                match snap.to_json_string_pretty() {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("serialize: {e}");
                        return ExitCode::from(1);
                    }
                }
            } else {
                match snap.to_json_string() {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("serialize: {e}");
                        return ExitCode::from(1);
                    }
                }
            };
            // Schema contract: never leak secret keys.
            if out.contains("apiKey")
                || out.contains("cookieHeader")
                || out.contains("accessToken")
                || out.contains("refreshToken")
            {
                eprintln!("internal error: snapshot leaked secret field");
                return ExitCode::from(1);
            }
            println!("{out}");
            ExitCode::SUCCESS
        }
        "text" => {
            if snap.providers.is_empty() {
                println!("No providers in snapshot");
                return ExitCode::SUCCESS;
            }
            for p in &snap.providers {
                let name = &p.id;
                if let Some(err) = &p.error {
                    println!("{name}: {err}");
                    continue;
                }
                if let Some(w) = &p.primary {
                    let mut line = format!("{name}: {:.1}%", w.used_percent);
                    if let Some(desc) = &w.reset_description {
                        line.push_str(" · ");
                        line.push_str(desc);
                    }
                    if let Some(c) = p.credits_remaining {
                        line.push_str(&format!(" · ${c:.2} left"));
                    }
                    println!("{line}");
                } else if let Some(cr) = &p.cursor_requests {
                    let used = cr.used.unwrap_or(0.0);
                    let incl = cr.included.unwrap_or(0.0);
                    println!("{name}: {used:.0}/{incl:.0} req");
                } else {
                    println!("{name}: —");
                }
            }
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("unknown --format {other} (use text|json)");
            ExitCode::from(2)
        }
    }
}

fn cmd_providers(_args: &[String]) -> ExitCode {
    // Static catalog (no secrets); same as ab_providers_catalog_json.
    println!("{}", ab_provider::mvp_catalog_json());
    ExitCode::SUCCESS
}

fn cmd_config(args: &[String]) -> ExitCode {
    if args.is_empty() {
        eprintln!("config requires a subcommand: path | patch | set-provider | migrate");
        return ExitCode::from(2);
    }
    match args[0].as_str() {
        "path" => {
            match ab_config::sticky_path() {
                Some(p) => {
                    println!("{}", p.display());
                    ExitCode::SUCCESS
                }
                None => {
                    eprintln!("could not resolve config path");
                    ExitCode::from(1)
                }
            }
        }
        "patch" => {
            let mut file: Option<PathBuf> = None;
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--file" | "-f" => {
                        i += 1;
                        if i >= args.len() {
                            eprintln!("--file requires a path");
                            return ExitCode::from(2);
                        }
                        file = Some(PathBuf::from(&args[i]));
                    }
                    other => {
                        eprintln!("unknown config patch flag: {other}");
                        return ExitCode::from(2);
                    }
                }
                i += 1;
            }
            let Some(path) = file else {
                eprintln!("config patch requires --file <path>");
                return ExitCode::from(2);
            };
            match ab_config::apply_patch_file(&path) {
                Ok(_) => {
                    println!("ok");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("config patch failed: {e}");
                    ExitCode::from(1)
                }
            }
        }
        "set-provider" => {
            if args.len() < 2 {
                eprintln!("config set-provider requires <id>");
                return ExitCode::from(2);
            }
            let id = args[1].clone();
            let mut enabled: Option<bool> = None;
            let mut source_mode: Option<String> = None;
            let mut api_key: Option<String> = None;
            let mut cookie_header: Option<String> = None;
            let mut i = 2;
            while i < args.len() {
                match args[i].as_str() {
                    "--enabled" => {
                        i += 1;
                        if i >= args.len() {
                            eprintln!("--enabled requires true|false");
                            return ExitCode::from(2);
                        }
                        enabled = match args[i].as_str() {
                            "true" | "1" | "yes" => Some(true),
                            "false" | "0" | "no" => Some(false),
                            other => {
                                eprintln!("invalid --enabled {other}");
                                return ExitCode::from(2);
                            }
                        };
                    }
                    "--source-mode" => {
                        i += 1;
                        if i >= args.len() {
                            eprintln!("--source-mode requires a value");
                            return ExitCode::from(2);
                        }
                        source_mode = Some(args[i].clone());
                    }
                    "--api-key-env" => {
                        i += 1;
                        if i >= args.len() {
                            eprintln!("--api-key-env requires a variable name");
                            return ExitCode::from(2);
                        }
                        match read_secret_from_env(&args[i]) {
                            Ok(v) => api_key = Some(v),
                            Err(e) => {
                                eprintln!("{e}");
                                return ExitCode::from(1);
                            }
                        }
                    }
                    "--api-key" => {
                        i += 1;
                        if i >= args.len() {
                            eprintln!("--api-key requires '-' (stdin) — secrets on argv are rejected");
                            return ExitCode::from(2);
                        }
                        if args[i] == "-" {
                            match read_secret_from_stdin("api key") {
                                Ok(v) => api_key = Some(v),
                                Err(e) => {
                                    eprintln!("{e}");
                                    return ExitCode::from(1);
                                }
                            }
                        } else {
                            // AB-001: refuse plain-argv secrets (process list / shell history).
                            eprintln!(
                                "error: refusing secret on argv (visible in process lists / shell history).\n\
Use --api-key - (stdin), --api-key-env VAR, AGENTBAR_API_KEY, or `config patch --file`."
                            );
                            return ExitCode::from(2);
                        }
                    }
                    "--cookie-header-env" => {
                        i += 1;
                        if i >= args.len() {
                            eprintln!("--cookie-header-env requires a variable name");
                            return ExitCode::from(2);
                        }
                        match read_secret_from_env(&args[i]) {
                            Ok(v) => cookie_header = Some(v),
                            Err(e) => {
                                eprintln!("{e}");
                                return ExitCode::from(1);
                            }
                        }
                    }
                    "--cookie-header" => {
                        i += 1;
                        if i >= args.len() {
                            eprintln!(
                                "--cookie-header requires '-' (stdin) — secrets on argv are rejected"
                            );
                            return ExitCode::from(2);
                        }
                        if args[i] == "-" {
                            match read_secret_from_stdin("cookie header") {
                                Ok(v) => cookie_header = Some(v),
                                Err(e) => {
                                    eprintln!("{e}");
                                    return ExitCode::from(1);
                                }
                            }
                        } else {
                            eprintln!(
                                "error: refusing secret on argv (visible in process lists / shell history).\n\
Use --cookie-header - (stdin), --cookie-header-env VAR, AGENTBAR_COOKIE_HEADER, or `config patch --file`."
                            );
                            return ExitCode::from(2);
                        }
                    }
                    other => {
                        eprintln!("unknown set-provider flag: {other}");
                        return ExitCode::from(2);
                    }
                }
                i += 1;
            }

            // Prefer env fallbacks when flags omitted (still no secret on argv).
            if api_key.is_none() {
                if let Ok(v) = env::var("AGENTBAR_API_KEY") {
                    if !v.is_empty() {
                        api_key = Some(v);
                    }
                }
            }
            if cookie_header.is_none() {
                if let Ok(v) = env::var("AGENTBAR_COOKIE_HEADER") {
                    if !v.is_empty() {
                        cookie_header = Some(v);
                    }
                }
            }

            let mut entry = json!({ "id": id });
            if let Some(e) = enabled {
                entry["enabled"] = Value::Bool(e);
            }
            if let Some(m) = source_mode {
                entry["sourceMode"] = Value::String(m);
            }
            if let Some(k) = api_key {
                entry["apiKey"] = Value::String(k);
            }
            if let Some(c) = cookie_header {
                entry["cookieHeader"] = Value::String(c);
            }
            let patch = json!({ "providers": [entry] });
            match ab_config::apply_patch(&patch) {
                Ok(_) => {
                    println!("ok");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("set-provider failed: {e}");
                    ExitCode::from(1)
                }
            }
        }
        "migrate" => match ab_config::migrate_to_agentbar() {
            Ok(dest) => {
                println!("{}", dest.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("migrate failed: {e}");
                ExitCode::from(1)
            }
        },
        other => {
            eprintln!("unknown config subcommand: {other}");
            ExitCode::from(2)
        }
    }
}

fn read_secret_from_env(var: &str) -> Result<String, String> {
    match env::var(var) {
        Ok(v) if !v.is_empty() => Ok(v),
        Ok(_) => Err(format!("environment variable {var} is empty")),
        Err(_) => Err(format!("environment variable {var} is not set")),
    }
}

fn read_secret_from_stdin(label: &str) -> Result<String, String> {
    use std::io::{self, BufRead};
    let mut line = String::new();
    let stdin = io::stdin();
    let n = stdin
        .lock()
        .read_line(&mut line)
        .map_err(|e| format!("failed to read {label} from stdin: {e}"))?;
    if n == 0 {
        return Err(format!("stdin closed before {label} was provided"));
    }
    let trimmed = line.trim_end_matches(['\r', '\n']).to_string();
    if trimmed.is_empty() {
        return Err(format!("{label} from stdin is empty"));
    }
    Ok(trimmed)
}
