//! `cs mcp` — MCP server commands.
//!
//! The CLI is a thin HTTP client of the app's control plane: it never opens the
//! database, the config cache or a server process, so the desktop and the CLI
//! cannot diverge about what is installed or running (AC-1). Mutations always
//! carry an idempotency key, generated when the caller did not supply one, so a
//! retried request cannot start a second process (AC-2).

use crate::args::{Cli, McpCommand};
use crate::capability::{fetch_and_render, name_of, text};
use crate::client::ControlPlaneClient;
use crate::error::CliError;
use serde_json::{json, Value};

/// Runs one `cs mcp` subcommand.
pub async fn run(
    cli: &Cli,
    client: &ControlPlaneClient,
    command: &McpCommand,
) -> Result<(), CliError> {
    match command {
        McpCommand::List => {
            fetch_and_render(cli, client, "/control/v1/mcp-servers", human_servers).await
        }
        McpCommand::Status { name } => status(cli, client, name).await,
        McpCommand::Install {
            descriptor_json,
            descriptor_file,
            enable,
            idempotency_key,
        } => install(cli, client, descriptor_json, descriptor_file, *enable, idempotency_key).await,
        McpCommand::Uninstall {
            name,
            idempotency_key,
        } => mutation(cli, client, "mcp-uninstall", name, "uninstall", idempotency_key).await,
        McpCommand::Enable {
            name,
            idempotency_key,
        } => mutation(cli, client, "mcp-enable", name, "enable", idempotency_key).await,
        McpCommand::Disable {
            name,
            idempotency_key,
        } => mutation(cli, client, "mcp-disable", name, "disable", idempotency_key).await,
        McpCommand::Restart {
            name,
            idempotency_key,
        } => mutation(cli, client, "mcp-restart", name, "restart", idempotency_key).await,
        McpCommand::Refresh {
            name,
            idempotency_key,
        } => mutation(cli, client, "mcp-refresh", name, "refresh", idempotency_key).await,
        McpCommand::Tools { name } => tools(cli, client, name).await,
    }
}

/// Implements `cs mcp status <name>` with a fresh bounded observation.
async fn status(cli: &Cli, client: &ControlPlaneClient, name: &str) -> Result<(), CliError> {
    let id = resolve_id(client, name).await?;
    let value = client
        .get(&format!("/control/v1/mcp-status?id={id}"))
        .await?;
    render_one(cli, &value);
    Ok(())
}

/// Implements `cs mcp install`.
///
/// The descriptor is passed through untouched: the service is the validator, so
/// the CLI cannot grow a second, laxer notion of what a valid server is.
/// `--enable` is deliberately a *second* operation, because starting a process
/// is a different effect from storing a record (AC-9/D-7).
async fn install(
    cli: &Cli,
    client: &ControlPlaneClient,
    descriptor_json: &Option<String>,
    descriptor_file: &Option<std::path::PathBuf>,
    enable: bool,
    idempotency_key: &Option<String>,
) -> Result<(), CliError> {
    let descriptor = read_document(descriptor_json, descriptor_file)?;
    let key = generated_idempotency_key(idempotency_key, "install");
    let value = client
        .post("/control/v1/mcp-install", descriptor.clone(), Some(&key))
        .await?;
    render_one(cli, &value);

    if !enable {
        return Ok(());
    }
    let id = value
        .get("result")
        .and_then(|result| result.get("id"))
        .and_then(Value::as_i64)
        .ok_or_else(|| CliError::io("the install result did not report a record id".to_string()))?;
    let key = generated_idempotency_key(idempotency_key, "enable");
    let enabled = client
        .post("/control/v1/mcp-enable", json!({ "id": id }), Some(&key))
        .await?;
    // The install line is already printed for the human reader; the enable
    // result is the fact that matters at the end of the command.
    render_one(cli, &enabled);
    Ok(())
}

/// Implements one name-addressed MCP mutation.
async fn mutation(
    cli: &Cli,
    client: &ControlPlaneClient,
    route: &str,
    name: &str,
    action: &str,
    idempotency_key: &Option<String>,
) -> Result<(), CliError> {
    let id = resolve_id(client, name).await?;
    let key = generated_idempotency_key(idempotency_key, action);
    let value = client
        .post(
            &format!("/control/v1/{route}"),
            json!({ "id": id }),
            Some(&key),
        )
        .await?;
    render_one(cli, &value);
    Ok(())
}

/// Implements `cs mcp tools <name>`: a read that never invokes a tool.
async fn tools(cli: &Cli, client: &ControlPlaneClient, name: &str) -> Result<(), CliError> {
    let id = resolve_id(client, name).await?;
    let value = client
        .get(&format!("/control/v1/mcp-tools?id={id}"))
        .await?;
    match cli.output {
        crate::args::OutputFormat::Json => crate::output::print_json(&value),
        crate::args::OutputFormat::Jsonl => crate::output::print_jsonl(&value),
        crate::args::OutputFormat::Human => {
            use std::io::Write;
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            for row in human_tools(&value) {
                let _ = writeln!(handle, "{row}");
            }
        }
    }
    Ok(())
}

/// Resolves a server name to its record id through the shared read model.
///
/// The mutation routes are id-addressed because a name can be renamed; resolving
/// here keeps one authoritative lookup instead of letting each route guess.
async fn resolve_id(client: &ControlPlaneClient, name: &str) -> Result<i64, CliError> {
    let value = client.get("/control/v1/mcp-servers").await?;
    value
        .as_array()
        .and_then(|servers| {
            servers.iter().find(|server| {
                server.get("name").and_then(Value::as_str) == Some(name)
            })
        })
        .and_then(|server| server.get("id").and_then(Value::as_i64))
        .ok_or_else(|| CliError::Server {
            status: 404,
            code: "not_found".to_string(),
            message: format!("MCP server '{name}' is not registered"),
        })
}

/// Uses the caller's key, or mints one so a mutation is always idempotent.
fn generated_idempotency_key(provided: &Option<String>, action: &str) -> String {
    match provided {
        Some(key) if !key.trim().is_empty() => key.clone(),
        _ => format!("cs-mcp-{action}-{}", uuid::Uuid::new_v4().simple()),
    }
}

/// Reads the descriptor document from one of the two accepted inputs.
fn read_document(
    inline: &Option<String>,
    file: &Option<std::path::PathBuf>,
) -> Result<Value, CliError> {
    use std::io::Read;
    let raw = match (inline, file) {
        (Some(text), None) => text.clone(),
        (None, Some(path)) if path.as_os_str() == "-" => {
            let mut buffer = String::new();
            std::io::stdin()
                .read_to_string(&mut buffer)
                .map_err(|e| CliError::io(format!("Failed to read the descriptor from stdin: {e}")))?;
            buffer
        }
        (None, Some(path)) => std::fs::read_to_string(path).map_err(|e| {
            CliError::io(format!("Failed to read the descriptor file {}: {e}", path.display()))
        })?,
        (None, None) => {
            return Err(CliError::usage(
                "provide the MCP descriptor with --descriptor-json or --descriptor-file",
            ))
        }
        _ => {
            return Err(CliError::usage(
                "--descriptor-json and --descriptor-file are mutually exclusive",
            ))
        }
    };
    serde_json::from_str(&raw).map_err(|e| {
        CliError::usage(format!("the MCP descriptor is not valid JSON: {e}"))
    })
}

/// `<status>  <name>  <operation>` for a mutation result.
fn human_mutation(value: &Value) -> Vec<String> {
    let result = value.get("result").cloned().unwrap_or(Value::Null);
    let name = text(&result, "name");
    let name = if name.is_empty() {
        "-".to_string()
    } else {
        name
    };
    vec![format!(
        "{}	{}	{}	{}",
        rust_i18n::t!("cs.mcp_mutation_header"),
        text(&result, "status"),
        name,
        text(value, "operation_id")
    )]
}

/// The tools header plus one line per published tool.
fn human_tools(value: &Value) -> Vec<String> {
    let mut rows = vec![format!(
        "{}  [{}]",
        rust_i18n::t!("cs.mcp_tools_header"),
        text(value, "freshness")
    )];
    let tools = value.get("tools").and_then(Value::as_array).cloned().unwrap_or_default();
    if tools.is_empty() {
        rows.push(rust_i18n::t!("cs.no_records").to_string());
        return rows;
    }
    for tool in tools {
        let mut row = name_of(&tool);
        let description = text(&tool, "description");
        if !description.is_empty() {
            row.push_str("  ");
            row.push_str(&description);
        }
        rows.push(row);
    }
    rows
}

/// Prints one record in the selected format.
///
/// A mutation result is rendered as its status line; a server view keeps the
/// list row layout, so a reader sees the same columns from `list` and `status`.
fn render_one(cli: &Cli, value: &Value) {
    match cli.output {
        crate::args::OutputFormat::Json => crate::output::print_json(value),
        crate::args::OutputFormat::Jsonl => crate::output::print_jsonl(value),
        crate::args::OutputFormat::Human => {
            use std::io::Write;
            let stdout = std::io::stdout();
            let mut handle = stdout.lock();
            let rows = if value.get("result").is_some() && value.get("operation_id").is_some() {
                human_mutation(value)
            } else if value.get("desired").is_some() {
                vec![server_row(value)]
            } else {
                vec![text(value, "status")]
            };
            for row in rows {
                let _ = writeln!(handle, "{row}");
            }
        }
    }
}

/// `<name>  <protocol>  <enabled|disabled>  <runtime>  <tools|->  <drift|->`
fn human_servers(value: &Value) -> Vec<String> {
    let servers = value.as_array().cloned().unwrap_or_default();
    let mut rows = vec![rust_i18n::t!("cs.mcp_list_header").to_string()];
    if servers.is_empty() {
        rows.push(rust_i18n::t!("cs.no_records").to_string());
        return rows;
    }
    for server in &servers {
        rows.push(server_row(server));
    }
    rows
}

fn server_row(server: &Value) -> String {
    let enabled = server["desired"]["enabled"] == Value::Bool(true);
    let observed = server["runtime"]["observed"] == Value::Bool(true);
    format!(
        "{}\t{}\t{}\t{}\t{}\t{}",
        name_of(server),
        text(server, "protocol_type"),
        if enabled { "enabled" } else { "disabled" },
        if observed {
            text(&server["runtime"], "state")
        } else {
            "unobserved".to_string()
        },
        text(&server["tools"], "count"),
        text(server, "drift")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn human_servers_separate_desired_from_observed() {
        let value = json!([
            {
                "name": "weather",
                "protocol_type": "stdio",
                "desired": { "enabled": true, "registered": true },
                "runtime": { "state": "connected", "observed": true },
                "tools": { "count": 3, "freshness": "observed" },
            },
            {
                "name": "notes",
                "protocol_type": "streamable_http",
                "desired": { "enabled": false, "registered": true },
                "runtime": { "state": "unknown", "observed": false },
                "tools": { "freshness": "unavailable" },
                "drift": "running_while_disabled",
            },
        ]);
        let rows = human_servers(&value);
        assert_eq!(rows.len(), 3);
        assert!(rows[1].contains("enabled"));
        assert!(rows[1].contains("connected"));
        assert!(rows[1].contains('3'));
        assert!(rows[2].contains("disabled"));
        // An unobserved runtime is never reported as a running state.
        assert!(rows[2].contains("unobserved"));
        assert!(rows[2].contains("running_while_disabled"));
    }

    /// A mutation prints its own status and the durable operation id, so an
    /// operator can follow the record afterwards (AC-2/AC-6).
    #[test]
    fn a_mutation_result_renders_status_and_operation() {
        let value = json!({
            "operation_id": "op-mcp-1",
            "replayed": false,
            "result": { "status": "registered", "id": 7, "name": "weather", "disabled": true },
        });
        let rows = human_mutation(&value);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].contains("registered"), "{rows:?}");
        assert!(rows[0].contains("weather"), "{rows:?}");
        assert!(rows[0].contains("op-mcp-1"), "{rows:?}");
    }

    /// Listing tools states the freshness of what it prints, because a stale
    /// snapshot must not read as a live answer (INV-7).
    #[test]
    fn the_tools_renderer_shows_freshness_and_never_a_tool_argument() {
        let value = json!({
            "source": "runtime",
            "freshness": "stale",
            "tools": [{ "name": "read_file", "description": "Read a file", "input_schema": {
                "properties": { "path": { "type": "string" } }
            } }],
        });
        let rows = human_tools(&value);
        assert!(rows[0].contains("stale"), "{rows:?}");
        assert!(rows[1].contains("read_file"), "{rows:?}");
        // The schema is machine output only; the human row is one line per tool.
        assert!(!rows[1].contains("input_schema"), "{rows:?}");
    }

    #[test]
    fn an_empty_tool_list_says_so_rather_than_looking_broken() {
        let rows = human_tools(&json!({ "freshness": "unavailable", "tools": [] }));
        assert_eq!(rows.len(), 2);
        assert!(rows[0].contains("unavailable"), "{rows:?}");
    }

    #[test]
    fn a_descriptor_can_be_inlined_or_named_but_not_both() {
        let inline = Some(json!({ "name": "weather" }).to_string());
        let parsed = read_document(&inline, &None).expect("inline descriptor");
        assert_eq!(parsed["name"], "weather");

        let both = Some(std::path::PathBuf::from("-"));
        assert!(read_document(&inline, &both).is_err());
        assert!(read_document(&None, &None).is_err());
    }

    /// A caller-supplied key is reused verbatim; an omitted one is generated, so
    /// a retry is always idempotent (AC-2).
    #[test]
    fn an_explicit_idempotency_key_is_never_replaced() {
        assert_eq!(
            generated_idempotency_key(&Some("keep-me".to_string()), "install"),
            "keep-me"
        );
        let generated = generated_idempotency_key(&None, "install");
        assert!(generated.starts_with("cs-mcp-install-"), "{generated}");
        assert_ne!(generated, generated_idempotency_key(&None, "install"));
    }

    #[test]
    fn an_empty_server_list_says_so() {
        let rows = human_servers(&json!([]));
        assert_eq!(rows.len(), 2);
    }
}
