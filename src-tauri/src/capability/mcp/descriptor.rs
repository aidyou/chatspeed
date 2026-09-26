//! Strict explicit MCP descriptor parsing.
//!
//! The first MCP install surface accepts exactly one shape: a strict descriptor
//! for a `stdio` or `streamable_http` server. There is no preset catalog yet, so
//! nothing here guesses a runner, resolves an "npx-ish" string, or accepts a
//! shell command line (D-6, AC-9). Anything ambiguous is refused with a stable
//! machine code rather than normalized into a best guess.
//!
//! Secrets are accepted as input (a bearer token and env values must be storable
//! to be useful) but never leave this module in an error, a log or a DTO: the
//! error messages below quote field *names* only (AC-13).

use std::collections::HashSet;

use serde_json::Value;

use crate::capability::error::{code, CapabilityError};
use crate::mcp::client::{McpProtocolType, McpServerConfig};

/// Longest accepted server name.
const MAX_NAME_CHARS: usize = 128;
/// Longest accepted single argument or env value.
const MAX_TOKEN_CHARS: usize = 4096;
/// Most arguments accepted for one server.
const MAX_ARGS: usize = 128;
/// Most environment variables accepted for one server.
const MAX_ENV: usize = 128;
/// Longest accepted runtime timeout.
const MAX_TIMEOUT_SECONDS: u64 = 3600;

/// Shell control characters that must never appear in a command or argument.
///
/// The command is spawned directly, never through a shell, so a value that only
/// makes sense to a shell is a sign of a copied shell line rather than an
/// argument (AC-13: no shell concatenation).
const SHELL_METACHARACTERS: &[char] = &[' ', '\t', '\n', ';', '&', '|', '<', '>', '$', '`', '\\'];

fn invalid(message: impl Into<String>) -> CapabilityError {
    CapabilityError::invalid_request(message)
}

/// A validated install request: the record description plus the server config.
#[derive(Debug, Clone)]
pub struct ParsedDescriptor {
    pub description: String,
    pub config: McpServerConfig,
}

/// Parses the strict install descriptor into a validated server configuration.
///
/// `deny_unknown_fields` behavior is explicit rather than a serde attribute so
/// every refusal names the offending key.
pub fn parse_descriptor(value: &Value) -> Result<ParsedDescriptor, CapabilityError> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("the MCP descriptor must be a JSON object"))?;

    let known = [
        "name",
        "description",
        "type",
        "url",
        "bearer_token",
        "proxy",
        "command",
        "args",
        "env",
        "disabled_tools",
        "timeout",
    ];
    for key in object.keys() {
        if !known.contains(&key.as_str()) {
            return Err(invalid(format!(
                "unknown MCP descriptor field '{key}'; accepted fields are name, description, \
                 type, url, bearer_token, proxy, command, args, env, disabled_tools, timeout"
            )));
        }
    }

    let name = required_string(object.get("name"), "name")?;
    validate_name(&name)?;
    let description = match object.get("description") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(text)) => text.clone(),
        Some(_) => return Err(invalid("'description' must be a string")),
    };

    let protocol = match object.get("type") {
        Some(Value::String(text)) => match text.as_str() {
            "stdio" => McpProtocolType::Stdio,
            "streamable_http" => McpProtocolType::StreamableHttp,
            // SSE was removed from the client stack; accepting it here would
            // create a record that can never run.
            "sse" => {
                return Err(CapabilityError::new(
                    code::UNSUPPORTED_ADAPTER,
                    "'sse' has no supported MCP adapter; use 'streamable_http'",
                ));
            }
            other => {
                return Err(CapabilityError::new(
                    code::UNSUPPORTED_ADAPTER,
                    format!("unsupported MCP transport type '{other}'"),
                ));
            }
        },
        _ => return Err(invalid("'type' is required and must be 'stdio' or 'streamable_http'")),
    };

    let url = optional_string(object.get("url"), "url")?;
    let bearer_token = optional_string(object.get("bearer_token"), "bearer_token")?;
    let proxy = optional_string(object.get("proxy"), "proxy")?;
    let command = optional_string(object.get("command"), "command")?;
    let args = optional_string_list(object.get("args"), "args")?;
    let env = parse_env(object.get("env"))?;
    let disabled_tools = optional_string_list(object.get("disabled_tools"), "disabled_tools")?;
    let timeout = match object.get("timeout") {
        None | Some(Value::Null) => None,
        Some(Value::Number(number)) => {
            let seconds = number
                .as_u64()
                .ok_or_else(|| invalid("'timeout' must be a positive integer number of seconds"))?;
            if seconds > MAX_TIMEOUT_SECONDS {
                return Err(invalid(format!(
                    "'timeout' must be at most {MAX_TIMEOUT_SECONDS} seconds"
                )));
            }
            Some(seconds)
        }
        Some(_) => return Err(invalid("'timeout' must be a number of seconds")),
    };

    match protocol {
        McpProtocolType::Stdio => {
            let command = command
                .clone()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| invalid("a stdio server requires a non-empty 'command'"))?;
            validate_executable(&command)?;
            if let Some(values) = args.as_ref() {
                if values.len() > MAX_ARGS {
                    return Err(invalid(format!("'args' accepts at most {MAX_ARGS} entries")));
                }
                for arg in values {
                    validate_argument(arg)?;
                }
            }
            if url.is_some() {
                return Err(invalid("'url' is not valid for a stdio server"));
            }
        }
        McpProtocolType::StreamableHttp => {
            if command.is_some() || args.is_some() || env.is_some() {
                return Err(invalid(
                    "'command', 'args' and 'env' are only valid for a stdio server",
                ));
            }
            let url = url
                .clone()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| invalid("a streamable_http server requires a non-empty 'url'"))?;
            validate_url(&url)?;
            if let Some(proxy) = proxy.as_deref() {
                validate_url(proxy)?;
            }
        }
        // Refused above, kept for exhaustiveness.
        McpProtocolType::Sse => unreachable!("sse is refused before this point"),
    }

    // A stdio record always stores explicit (possibly empty) collections, so the
    // persisted JSON shape does not depend on which fields the caller omitted.
    let stdio = matches!(protocol, McpProtocolType::Stdio);

    Ok(ParsedDescriptor {
        description,
        config: McpServerConfig {
            name: name.clone(),
            protocol_type: protocol,
            url,
            bearer_token,
            proxy,
            command,
            args: args.or(if stdio { Some(Vec::new()) } else { None }),
            env: env.or(if stdio { Some(Vec::new()) } else { None }),
            disabled_tools: Some(
                disabled_tools
                    .map(|values| values.into_iter().collect::<HashSet<_>>())
                    .unwrap_or_default(),
            ),
            timeout,
        },
    })
}

fn required_string(field: Option<&Value>, name: &str) -> Result<String, CapabilityError> {
    match field {
        Some(Value::String(text)) if !text.trim().is_empty() => Ok(text.clone()),
        Some(Value::String(_)) => Err(invalid(format!("'{name}' must not be empty"))),
        Some(_) => Err(invalid(format!("'{name}' must be a string"))),
        None => Err(invalid(format!("'{name}' is required"))),
    }
}

fn optional_string(field: Option<&Value>, name: &str) -> Result<Option<String>, CapabilityError> {
    match field {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => {
            if text.len() > MAX_TOKEN_CHARS {
                return Err(invalid(format!(
                    "'{name}' must be at most {MAX_TOKEN_CHARS} characters"
                )));
            }
            Ok(Some(text.clone()))
        }
        Some(_) => Err(invalid(format!("'{name}' must be a string"))),
    }
}

fn optional_string_list(
    field: Option<&Value>,
    name: &str,
) -> Result<Option<Vec<String>>, CapabilityError> {
    match field {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                let Some(text) = item.as_str() else {
                    return Err(invalid(format!("'{name}' must be an array of strings")));
                };
                values.push(text.to_string());
            }
            Ok(Some(values))
        }
        Some(_) => Err(invalid(format!("'{name}' must be an array of strings"))),
    }
}

/// Accepts `[[NAME, VALUE], ...]`, the shape the existing record stores.
fn parse_env(field: Option<&Value>) -> Result<Option<Vec<(String, String)>>, CapabilityError> {
    match field {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) => {
            if items.len() > MAX_ENV {
                return Err(invalid(format!("'env' accepts at most {MAX_ENV} entries")));
            }
            let mut pairs = Vec::with_capacity(items.len());
            for item in items {
                let entry = item
                    .as_array()
                    .filter(|pair| pair.len() == 2)
                    .ok_or_else(|| {
                        invalid("'env' entries must be [NAME, VALUE] pairs; a bare object is not \
                                accepted because its values would leak into logs")
                    })?;
                let name = entry[0]
                    .as_str()
                    .ok_or_else(|| invalid("'env' entry names must be strings"))?;
                let value = entry[1]
                    .as_str()
                    .ok_or_else(|| invalid("'env' entry values must be strings"))?;
                validate_env_name(name)?;
                if value.len() > MAX_TOKEN_CHARS {
                    return Err(invalid(format!(
                        "'env' values must be at most {MAX_TOKEN_CHARS} characters"
                    )));
                }
                pairs.push((name.to_string(), value.to_string()));
            }
            Ok(Some(pairs))
        }
        Some(_) => Err(invalid("'env' must be an array of [NAME, VALUE] pairs")),
    }
}

fn validate_name(name: &str) -> Result<(), CapabilityError> {
    if name.len() > MAX_NAME_CHARS {
        return Err(invalid(format!(
            "'name' must be at most {MAX_NAME_CHARS} characters"
        )));
    }
    if !name
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
    {
        return Err(invalid(
            "'name' may only contain ASCII letters, digits, '.', '_' and '-'",
        ));
    }
    Ok(())
}

fn validate_env_name(name: &str) -> Result<(), CapabilityError> {
    let valid = !name.is_empty()
        && name.len() <= MAX_NAME_CHARS
        && name
            .chars()
            .next()
            .map_or(false, |first| first.is_ascii_alphabetic() || first == '_')
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_');
    if valid {
        Ok(())
    } else {
        Err(invalid(
            "'env' names must be shell-safe identifiers (letters, digits and '_')",
        ))
    }
}

fn validate_executable(command: &str) -> Result<(), CapabilityError> {
    if command.contains(SHELL_METACHARACTERS) {
        return Err(invalid(
            "'command' must be one executable path or program name, not a shell line",
        ));
    }
    Ok(())
}

fn validate_argument(argument: &str) -> Result<(), CapabilityError> {
    if argument.len() > MAX_TOKEN_CHARS {
        return Err(invalid(format!(
            "'args' entries must be at most {MAX_TOKEN_CHARS} characters"
        )));
    }
    // A leading `$(`, backtick or unquoted newline is the shape of a shell
    // fragment. Argument values are passed verbatim to the process.
    if argument.contains(['\n', '\r', '\0']) || argument.starts_with("$(") || argument.contains('`')
    {
        return Err(invalid(
            "'args' entries must be literal values, not shell substitutions",
        ));
    }
    Ok(())
}

fn validate_url(url: &str) -> Result<(), CapabilityError> {
    let parsed = url::Url::parse(url)
        .map_err(|_| invalid("a server URL must be an absolute http(s) URL"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(invalid("a server URL must use the http or https scheme"));
    }
    // Credentials in a URL end up in request logs and connection errors, so the
    // dedicated bearer token field is the only accepted place for a token.
    if parsed.username() != "" || parsed.password().is_some() {
        return Err(invalid(
            "a server URL must not embed credentials; use 'bearer_token' instead",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn stdio() -> Value {
        json!({
            "name": "weather",
            "description": "Weather data",
            "type": "stdio",
            "command": "node",
            "args": ["server.js", "--profile", "safe"]
        })
    }

    #[test]
    fn a_valid_stdio_descriptor_is_accepted_verbatim() {
        let parsed = parse_descriptor(&stdio()).expect("valid descriptor");
        let config = parsed.config;
        assert_eq!(parsed.description, "Weather data");
        assert_eq!(config.name, "weather");
        assert_eq!(config.protocol_type, McpProtocolType::Stdio);
        assert_eq!(config.command.as_deref(), Some("node"));
        assert_eq!(
            config.args,
            Some(vec![
                "server.js".to_string(),
                "--profile".to_string(),
                "safe".to_string()
            ])
        );
        // An empty tool set is stored explicitly so the shape stays stable.
        assert_eq!(config.disabled_tools, Some(HashSet::new()));
    }

    #[test]
    fn a_streamable_http_descriptor_requires_only_a_url() {
        let config = parse_descriptor(&json!({
            "name": "remote",
            "type": "streamable_http",
            "url": "https://example.test/mcp",
            "bearer_token": "tok",
            "timeout": 30
        }))
        .expect("valid http descriptor").config;
        assert_eq!(config.protocol_type, McpProtocolType::StreamableHttp);
        assert_eq!(config.bearer_token.as_deref(), Some("tok"));
        assert_eq!(config.timeout, Some(30));
        assert!(config.command.is_none());
    }

    #[test]
    fn sse_is_refused_as_an_unsupported_adapter() {
        let error = parse_descriptor(&json!({
            "name": "legacy", "type": "sse", "url": "https://example.test/sse"
        }))
        .expect_err("sse must be refused");
        assert_eq!(error.code(), code::UNSUPPORTED_ADAPTER);
    }

    #[test]
    fn an_unknown_field_is_refused_rather_than_ignored() {
        let error = parse_descriptor(&json!({
            "name": "weather", "type": "stdio", "command": "node", "shell": true
        }))
        .expect_err("unknown field");
        assert_eq!(error.code(), code::INVALID_REQUEST);
        assert!(error.redacted_message().contains("shell"));
    }

    #[test]
    fn a_shell_command_line_is_refused() {
        for command in ["node server.js", "bash -c 'rm -rf /'", "$(whoami)", "a;b", "a|b"] {
            let error = parse_descriptor(&json!({
                "name": "weather", "type": "stdio", "command": command
            }))
            .expect_err("shell-shaped command");
            assert_eq!(error.code(), code::INVALID_REQUEST, "for {command}");
        }
    }

    #[test]
    fn an_argument_substitution_is_refused() {
        let error = parse_descriptor(&json!({
            "name": "weather", "type": "stdio", "command": "node",
            "args": ["$(id)"]
        }))
        .expect_err("substitution argument");
        assert_eq!(error.code(), code::INVALID_REQUEST);
    }

    #[test]
    fn a_url_with_embedded_credentials_is_refused() {
        let error = parse_descriptor(&json!({
            "name": "remote", "type": "streamable_http",
            "url": "https://user:canary-secret@example.test/mcp"
        }))
        .expect_err("embedded credentials");
        assert_eq!(error.code(), code::INVALID_REQUEST);
        // The refusal never repeats the credential it just rejected (AC-13).
        assert!(!error.redacted_message().contains("canary-secret"));
    }

    #[test]
    fn env_must_be_pair_arrays_not_a_bare_object() {
        let pairs = parse_descriptor(&json!({
            "name": "weather", "type": "stdio", "command": "node",
            "env": [["API_TOKEN", "value"]]
        }))
        .expect("pair env")
        .config
        .env;
        assert_eq!(pairs, Some(vec![("API_TOKEN".to_string(), "value".to_string())]));

        let error = parse_descriptor(&json!({
            "name": "weather", "type": "stdio", "command": "node",
            "env": { "API_TOKEN": "value" }
        }))
        .expect_err("object env");
        assert_eq!(error.code(), code::INVALID_REQUEST);

        let error = parse_descriptor(&json!({
            "name": "weather", "type": "stdio", "command": "node",
            "env": [["9BAD NAME", "value"]]
        }))
        .expect_err("invalid env name");
        assert_eq!(error.code(), code::INVALID_REQUEST);
    }

    #[test]
    fn transport_specific_fields_are_enforced_in_both_directions() {
        let error = parse_descriptor(&json!({
            "name": "weather", "type": "stdio", "command": "node",
            "url": "https://example.test/mcp"
        }))
        .expect_err("url on stdio");
        assert_eq!(error.code(), code::INVALID_REQUEST);

        let error = parse_descriptor(&json!({
            "name": "remote", "type": "streamable_http",
            "url": "https://example.test/mcp", "command": "node"
        }))
        .expect_err("command on http");
        assert_eq!(error.code(), code::INVALID_REQUEST);
    }

    #[test]
    fn an_empty_or_odd_name_is_refused() {
        for name in ["", "  ", "has space", "slash/es", "emoji-\u{1f600}"] {
            let error = parse_descriptor(&json!({
                "name": name, "type": "stdio", "command": "node"
            }))
            .expect_err("bad name");
            assert_eq!(error.code(), code::INVALID_REQUEST, "for {name:?}");
        }
    }

    #[test]
    fn a_secret_never_appears_in_a_refusal_message() {
        let error = parse_descriptor(&json!({
            "name": "weather", "type": "stdio", "command": "node",
            "env": [["TOKEN", "canary-env-value"]],
            "unexpected": 1
        }))
        .expect_err("unknown field");
        assert!(!error.redacted_message().contains("canary-env-value"));
    }

}
