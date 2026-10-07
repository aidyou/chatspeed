//! This module provides utilities for parsing and generating XML for tool use,
//! specifically designed to be robust against variations in XML-like formats provided by different models.
//!
//! ## XML Parsing Strategy
//!
//! The core of the parsing logic uses the `scraper` crate, which is an HTML parser.
//! This choice provides tolerance for malformed or incomplete XML, which is common in LLM outputs.
//! However, it introduces some important considerations:
//!
//! ### 1. Case-Insensitive Tag Matching
//!
//! `scraper` treats tag names case-insensitively, following HTML rules. This means `<Param>` and `<param>`
//! are considered the same. To avoid ambiguity and ensure reliable parameter extraction, a standardized
//! argument format is enforced.
//!
//! **Do not use parameter names as tags:**
//! ```xml
//! <!-- Not recommended: "param1" is the tag name, vulnerable to case-folding -->
//! <param1 type="string">abc</param1>
//! ```
//!
//! **Use the recommended `<arg>` tag with a `name` attribute:**
//! ```xml
//! <!-- Recommended: Robust and explicit -->
//! <arg name="param1" type="string">abc</arg>
//! ```
//!
//! The parser also supports custom tags (like `<param1>...`) as a fallback, but the `<arg>`
//! format is strongly preferred.
//!
//! ### 2. Handling of HTML Void Elements
//!
//! Tags like `<param>`, `<input>`, etc., are considered "void elements" in HTML and `scraper` may not
//! correctly extract their inner content. To work around this, the parser temporarily renames these
//! tags (e.g., to `<x-cs-param>`) before parsing and reverts the name afterward.
//!
//! ## Type Preservation
//!
//! This module carefully handles data type conversions to prevent loss of information, especially for numeric types.
//!
//! - **Parsing (`get_value`):** When parsing an argument from XML, the `type` attribute is used to cast the
//!   string content into the appropriate `serde_json::Value`. It supports various numeric types
//!   (e.g., "integer", "float", "u64") to ensure that numbers are correctly represented.
//!
//! - **Generating (`format_tool_use_xml`):** When converting a `serde_json::Value` back to XML, the code
//!   distinguishes between integers and floating-point numbers. A `serde_json::Number` that is an integer
//!   will be explicitly typed as `"integer"`, while a float will be typed as `"number"`. This ensures
//!   that the receiving end can correctly interpret the numeric type without ambiguity.
//!
//! ## Content Escaping
//!
//! The `escape_xml_content` function ensures that content written into XML tags is properly escaped
//! to prevent XML parsing errors, handling characters like `&`, `<`, and `>`.

use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::ccproxy::{adapter::unified::UnifiedContentBlock, helper::get_tool_id};

#[derive(Deserialize, Serialize, Debug, Clone, Copy, PartialEq)]
pub enum ParamType {
    String,
    Number, // Represents float/double
    Integer,
    Boolean,
    Array,
    Object,
    Null,
}

impl From<&str> for ParamType {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "string" => ParamType::String,
            "float" | "double" | "f32" | "f64" | "number" | "float32" | "float64" => {
                ParamType::Number
            }
            "int" | "i32" | "i64" | "integer" | "long" | "int32" | "int64" | "uint" | "u32"
            | "u64" | "uint32" | "uint64" | "usize" => ParamType::Integer,
            "bool" | "boolean" => ParamType::Boolean,
            "array" | "list" | "vec" => ParamType::Array,
            "object" | "map" | "dict" | "json" => ParamType::Object,
            "null" => ParamType::Null,
            _ => ParamType::String,
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct ToolUse {
    pub name: String,
    pub args: Vec<Arg>,
}

impl TryFrom<&str> for ToolUse {
    type Error = anyhow::Error;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        parse_tool_use(s)
    }
}

impl TryFrom<String> for ToolUse {
    type Error = anyhow::Error;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::try_from(s.as_str())
    }
}

/// Convert a ToolUse into a UnifiedContentBlock::ToolUse.
impl From<ToolUse> for UnifiedContentBlock {
    fn from(tool_use: ToolUse) -> UnifiedContentBlock {
        let mut arguments = serde_json::Map::new();
        for arg in tool_use.args {
            arguments.insert(arg.name.clone(), arg.get_value());
        }

        UnifiedContentBlock::ToolUse {
            id: get_tool_id(),
            name: tool_use.name,
            input: serde_json::Value::Object(arguments),
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Arg {
    pub name: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_type: Option<ParamType>,

    // Holds the attribute value or the inner HTML content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

impl Arg {
    pub fn get_value(&self) -> Value {
        let raw_value = self.value.as_deref().unwrap_or_default();
        let value = html_escape::decode_html_entities(raw_value).to_string();

        if let Some(data_type) = self.data_type {
            match data_type {
                ParamType::Number => {
                    if let Ok(f) = value.parse::<f64>() {
                        // Handle AI output compatibility: sometimes AI outputs floating-point numbers
                        // for integer values (e.g., 5.0 instead of 5). For Number type both are valid,
                        // but for Integer type 5.0 is incorrect. To maximize compatibility and minimize
                        // errors, we treat floating-point numbers with zero fractional part as integers.
                        if f.fract().abs() < std::f64::EPSILON {
                            return json!(f as i64);
                        }
                        return json!(f);
                    }
                }
                ParamType::Integer => {
                    if let Ok(i) = value.parse::<i64>() {
                        return json!(i);
                    }
                    if let Ok(u) = value.parse::<u64>() {
                        return json!(u);
                    }
                }
                ParamType::Boolean => {
                    if let Ok(b) = value.parse::<bool>() {
                        return json!(b);
                    }
                }
                ParamType::Array => {
                    if let Ok(arr) = serde_json::from_str::<Vec<Value>>(&value) {
                        return json!(arr);
                    }
                }
                ParamType::Object => {
                    if let Ok(obj) = serde_json::from_str::<Map<String, Value>>(&value) {
                        return json!(obj);
                    }
                }
                ParamType::Null => return Value::Null,
                ParamType::String => {}
            }
        }

        json!(value)
    }
}

/// Generate XML for tool definitions
pub fn generate_tools_xml(tools: &Vec<crate::ccproxy::adapter::unified::UnifiedTool>) -> String {
    let mut tools_xml = String::new();
    tools_xml.push_str("<cs:tools desc=\"All available tools\">\n");

    for tool in tools {
        tools_xml.push_str(&format!(
            "<cs:tool_define>\n<name>{}</name>\n<desc>{}</desc>\n",
            tool.name,
            tool.description.as_deref().unwrap_or("")
        ));

        if let Some(schema) = tool.input_schema.as_object() {
            // Get the list of required parameters from the JSON schema
            let required_args: std::collections::HashSet<String> = schema
                .get("required")
                .and_then(|r| r.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
                tools_xml.push_str("<args>\n");

                let mut required_keys: Vec<&String> = Vec::new();
                let mut optional_keys: Vec<&String> = Vec::new();

                for name in properties.keys() {
                    if required_args.contains(name) {
                        required_keys.push(name);
                    } else {
                        optional_keys.push(name);
                    }
                }

                required_keys.sort();
                optional_keys.sort();

                let mut append_param = |name: &String| {
                    let details = &properties[name];
                    let param_type = details
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("any");
                    let mut description = details
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("")
                        .to_string();

                    if required_args.contains(name) {
                        description.push_str(" (required)");
                    } else {
                        description.push_str(" (optional)");
                    }

                    tools_xml.push_str(&format!(
                        "<arg name=\"{}\" type=\"{}\">{}</arg>\n",
                        name, param_type, description
                    ));
                };

                for name in required_keys {
                    append_param(name);
                }
                for name in optional_keys {
                    append_param(name);
                }

                tools_xml.push_str("</args>\n");
            }
        }
        tools_xml.push_str("</cs:tool_define>\n");
    }
    tools_xml.push_str("</cs:tools>");
    tools_xml
}

pub fn format_tool_use_xml(id: &str, name: &str, input: &serde_json::Value) -> String {
    let mut args_xml = String::new();
    if let Some(obj) = input.as_object() {
        for (key, value) in obj {
            let (type_str, value_str) = match value {
                serde_json::Value::String(s) => ("string", s.clone()),
                serde_json::Value::Number(n) => {
                    // Preserve integer vs float distinction
                    if n.is_i64() || n.is_u64() {
                        ("integer", value.to_string())
                    } else {
                        ("number", value.to_string())
                    }
                }
                serde_json::Value::Bool(_) => ("boolean", value.to_string()),
                serde_json::Value::Array(_) => ("array", value.to_string()),
                serde_json::Value::Object(_) => ("object", value.to_string()),
                serde_json::Value::Null => ("null", "null".to_string()),
            };
            // Escape the content to avoid XML parsing issues
            let escaped_value = escape_xml_content(&value_str);
            args_xml.push_str(&format!(
                "<arg name=\"{}\" type=\"{}\">{}</arg>\n",
                key, type_str, escaped_value
            ));
        }
    }

    format!(
        "<cs:tool_use>\n<id>{}</id>\n<name>{}</name>\n<args>\n{}\n</args>\n</cs:tool_use>",
        id, name, args_xml
    )
}

fn extract_tag_content(xml: &str, tag: &str) -> Option<String> {
    let start_tag = format!("<{}", tag);
    let end_tag = format!("</{}>", tag);

    if let Some(start_pos) = xml.find(&start_tag) {
        // Find the end of the start tag (handling attributes)
        if let Some(tag_open_end) = xml[start_pos..].find(">") {
            let content_start = start_pos + tag_open_end + 1;
            if let Some(end_pos) = xml[content_start..].find(&end_tag) {
                return Some(
                    xml[content_start..content_start + end_pos]
                        .trim()
                        .to_string(),
                );
            } else {
                // FALLBACK: If closing tag is missing, take until the end of </cs:tool_use>
                let end_limit = xml.find("</cs:tool_use>").unwrap_or(xml.len());
                if end_limit > content_start {
                    return Some(xml[content_start..end_limit].trim().to_string());
                }
            }
        }
    }
    None
}

fn extract_attr(tag_header: &str, attr_name: &str) -> Option<String> {
    // Look for attribute name followed by optional spaces and '='
    let key_pattern = format!("{}", attr_name);
    if let Some(key_pos) = tag_header.find(&key_pattern) {
        let after_key = &tag_header[key_pos + key_pattern.len()..];

        // Find the '=' sign after the key
        if let Some(equal_pos) = after_key.find('=') {
            // Only consider the '=' if it's not part of another attribute (i.e., no letters before it)
            let between = &after_key[..equal_pos].trim();
            if !between.is_empty() {
                // This might be a false match where the key was part of another string
                // but for simple XML tags this check is usually enough.
            }

            let after_equal = after_key[equal_pos + 1..].trim_start();
            if after_equal.is_empty() {
                return None;
            }

            let first_char = after_equal.chars().next().unwrap();
            if first_char == '"' || first_char == '\'' {
                let quote = first_char;
                let val_start = 1;
                if let Some(val_end) = after_equal[val_start..].find(quote) {
                    return Some(after_equal[val_start..val_start + val_end].to_string());
                }
            } else {
                // Unquoted value: take until space, '>', or '/'
                let val_end = after_equal
                    .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
                    .unwrap_or(after_equal.len());
                return Some(after_equal[..val_end].to_string());
            }
        }
    }
    None
}

const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

/// Parse tool use XML using scraper - SIMPLIFIED ONE-LEVEL ONLY
pub fn parse_tool_use(xml_str: &str) -> Result<ToolUse, anyhow::Error> {
    // 1. First attempt to manually extract name and ID using simple string scanning.
    // This is often more robust than HTML parsing when the content contains unescaped code.
    let name = extract_tag_content(xml_str, "name").unwrap_or_default();

    if !name.is_empty() {
        // Try to extract the args block.
        // Robustness: If </args> is missing, try to take content until the end of </cs:tool_use> or the string.
        let args_content = if let Some(content) = extract_tag_content(xml_str, "args") {
            content
        } else {
            // If <args> wrapper is missing entirely, try to parse from the root string
            xml_str.to_string()
        };

        let mut args = Vec::new();
        // Look for <arg name="...">content</arg> patterns
        // We use a simplified scanner here to handle nested HTML-like content safely.
        let mut current_pos = 0;
        while let Some(start_pos) = args_content[current_pos..].find("<") {
            let start_idx = current_pos + start_pos;
            if args_content[start_idx..].starts_with("</") {
                current_pos = start_idx + 1;
                continue;
            }

            // Find the end of the opening tag (could be ">" or "/>")
            if let Some(tag_header_end_rel) = args_content[start_idx..].find(">") {
                let tag_header_end = start_idx + tag_header_end_rel;
                let is_self_closing = args_content[..tag_header_end + 1].ends_with("/>");
                let tag_header = &args_content[start_idx..tag_header_end + 1];

                // Extract tag name
                let tag_name = tag_header[1..]
                    .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
                    .next()
                    .unwrap_or("");

                if tag_name.is_empty() {
                    current_pos = tag_header_end + 1;
                    continue;
                }

                if tag_name == "arg" {
                    // Standard <arg name="..." ...> format
                    let arg_name = extract_attr(tag_header, "name").unwrap_or_default();
                    if arg_name.is_empty() {
                        current_pos = tag_header_end + 1;
                        continue;
                    }

                    let data_type =
                        extract_attr(tag_header, "type").map(|t| ParamType::from(t.as_str()));

                    if is_self_closing {
                        let arg_value = extract_attr(tag_header, "value").unwrap_or_default();
                        args.push(Arg {
                            name: arg_name,
                            data_type,
                            value: Some(arg_value),
                        });
                        current_pos = tag_header_end + 1;
                        continue;
                    } else {
                        let content_start = tag_header_end + 1;
                        if let Some(tag_close_start_rel) =
                            args_content[content_start..].find("</arg>")
                        {
                            let tag_close_start = content_start + tag_close_start_rel;
                            let arg_value =
                                if let Some(val_attr) = extract_attr(tag_header, "value") {
                                    val_attr
                                } else {
                                    args_content[content_start..tag_close_start].to_string()
                                };

                            args.push(Arg {
                                name: arg_name,
                                data_type,
                                value: Some(arg_value),
                            });
                            current_pos = tag_close_start + 6;
                            continue;
                        }
                    }
                } else {
                    // Custom tag format <my_param>value</my_param>
                    let data_type =
                        extract_attr(tag_header, "type").map(|t| ParamType::from(t.as_str()));
                    let close_tag = format!("</{}>", tag_name);

                    if is_self_closing {
                        let arg_value = extract_attr(tag_header, "value").unwrap_or_default();
                        args.push(Arg {
                            name: tag_name.to_string(),
                            data_type,
                            value: Some(arg_value),
                        });
                        current_pos = tag_header_end + 1;
                        continue;
                    } else {
                        let content_start = tag_header_end + 1;
                        if let Some(tag_close_start_rel) =
                            args_content[content_start..].find(&close_tag)
                        {
                            let tag_close_start = content_start + tag_close_start_rel;
                            let arg_value =
                                args_content[content_start..tag_close_start].to_string();

                            args.push(Arg {
                                name: tag_name.to_string(),
                                data_type,
                                value: Some(arg_value),
                            });
                            current_pos = tag_close_start + close_tag.len();
                            continue;
                        }
                    }
                }
            }
            current_pos += 1;
            if current_pos >= args_content.len() {
                break;
            }
        }

        if !args.is_empty() {
            return Ok(ToolUse { name, args });
        }
    }

    // 2. Fallback to original HTML-based parser for compatibility with other variants
    let mut processed_xml = xml_str.to_string();
    // The `scraper` library, when parsing HTML, does not extract inner content from elements
    // that are considered "void elements" in HTML (e.g., `param`, `input`), even if they
    // contain content in the XML-like input we are processing.
    // For example, if a tool parameter is named `param`, directly parsing
    // `<param type="string">abc</param>` would not yield the value `abc`.
    // To overcome this, we temporarily transform these HTML void tags into custom tags
    // (e.g., `<x-cs-param type="string">abc</x-cs-param>`) before parsing.
    // After parsing, the parameter name is reverted from `x-cs-param` back to `param`.
    for tag in VOID_ELEMENTS {
        processed_xml = processed_xml.replace(&format!("<{}", tag), &format!("<x-cs-{}", tag));
        processed_xml = processed_xml.replace(&format!("</{}", tag), &format!("</x-cs-{}", tag));
    }

    // Parse as HTML fragment for better error tolerance
    let document = Html::parse_fragment(&processed_xml);

    // Find the tool name
    let name_selector =
        Selector::parse("name").map_err(|e| anyhow::anyhow!("Invalid name selector: {}", e))?;

    let name = document
        .select(&name_selector)
        .next()
        .ok_or_else(|| anyhow::anyhow!("No <name> element found"))?
        .text()
        .collect::<Vec<_>>()
        .join("")
        .trim()
        .to_string();

    if name.is_empty() {
        return Err(anyhow::anyhow!("Tool name is empty"));
    }

    // Parse parameters - ONLY ONE LEVEL
    let args = parse_simple_args(&document)?;

    #[cfg(debug_assertions)]
    log::debug!(
        "tool use parse success, name: {}, args: {}",
        name,
        serde_json::to_string_pretty(&args).unwrap_or_default()
    );

    Ok(ToolUse { name, args })
}

/// Parse parameters from the document - SIMPLIFIED: only direct children of <args>
fn parse_simple_args(document: &Html) -> Result<Vec<Arg>, anyhow::Error> {
    let mut param_vec = Vec::new();

    let args_selector =
        Selector::parse("args").map_err(|e| anyhow::anyhow!("Invalid args selector: {}", e))?;
    let args_element = match document.select(&args_selector).next() {
        Some(el) => el,
        None => return Ok(param_vec),
    };

    let mut elements_to_process: Vec<ElementRef> = args_element
        .children()
        .filter_map(ElementRef::wrap)
        .collect();
    let mut i = 0;

    while i < elements_to_process.len() {
        let element = elements_to_process[i];
        i += 1;

        let value = element.value();
        let tag_name = value.name();

        if tag_name == "arg" {
            // Only process arg elements that have a name attribute (are actual parameters)
            if value.attr("name").is_some() {
                // Heuristic: if an <arg> has a `value` attribute, its element children
                // are treated as swallowed siblings. Otherwise, they are content.
                if value.attr("value").is_some() {
                    if let Some(arg) = parse_arg_element(&element)? {
                        param_vec.push(arg);
                    }
                    let children: Vec<ElementRef> =
                        element.children().filter_map(ElementRef::wrap).collect();
                    // Insert children at the current position to be processed next.
                    elements_to_process.splice(i..i, children);
                } else {
                    // No `value` attribute, so children are content.
                    // `parse_arg_element` will correctly handle this by using inner_html.
                    if let Some(arg) = parse_arg_element(&element)? {
                        param_vec.push(arg);
                    }
                }
            }
        } else {
            // This is a custom tag parameter.
            if let Some(arg) = parse_custom_tag_as_simple_arg(&element, tag_name)? {
                param_vec.push(arg);
            }
        }
    }

    Ok(param_vec)
}

/// Parse custom tag as simple argument (all content treated as text)
fn parse_custom_tag_as_simple_arg(
    element: &ElementRef,
    tag_name: &str,
) -> Result<Option<Arg>, anyhow::Error> {
    let content = element.text().collect::<Vec<_>>().join("");
    let explicit_type = element.value().attr("type").map(ParamType::from);
    let processed_content = preprocess_xml_content(content, explicit_type);

    let final_name = tag_name.strip_prefix("x-cs-").unwrap_or(tag_name);

    Ok(Some(Arg {
        name: final_name.to_string(),
        data_type: explicit_type,
        value: processed_content, // Store preprocessed content
    }))
}

/// Parse a single <arg> element
fn parse_arg_element(element: &ElementRef) -> Result<Option<Arg>, anyhow::Error> {
    let name = element
        .value()
        .attr("name")
        .ok_or_else(|| anyhow::anyhow!("arg element missing name attribute"))?
        .to_string();

    let explicit_type = element.value().attr("type").map(ParamType::from);

    if let Some(value) = element.value().attr("value").map(|s| s.to_string()) {
        let processed_value = preprocess_xml_content(value, explicit_type);
        return Ok(Some(Arg {
            name,
            data_type: explicit_type,
            value: processed_value,
        }));
    }

    let content = element.text().collect::<Vec<_>>().join("");
    let processed_content = preprocess_xml_content(content, explicit_type);

    Ok(Some(Arg {
        name,
        data_type: explicit_type,
        value: processed_content, // Store preprocessed content
    }))
}

/// Preprocesses content based on its type for XML parsing.
/// This includes trimming whitespace and escaping XML special characters.
fn preprocess_xml_content(content: String, data_type: Option<ParamType>) -> Option<String> {
    if let Some(dt) = data_type {
        match dt {
            // For string-like types that can contain special characters.
            ParamType::String | ParamType::Array | ParamType::Object => {
                // For JSON, trim whitespace before escaping.
                let processed = if dt == ParamType::String {
                    content
                } else {
                    content.trim().to_string()
                };
                Some(escape_xml_content(&processed))
            }
            // For simple types, just trim. No escaping needed.
            ParamType::Number | ParamType::Boolean | ParamType::Integer | ParamType::Null => {
                Some(content.trim().to_string())
            }
        }
    } else {
        // If type is unknown, assume it might be a string that needs escaping.
        Some(escape_xml_content(&content))
    }
}

/// Escape XML content to prevent parsing issues
///
/// This function handles the 5 basic XML entities that are required for valid XML:
/// - & → &amp;   (must be escaped first to avoid double-escaping)
/// - < → &lt;    (starts XML tags)
/// - > → &gt;    (ends XML tags)
/// - " → &quot;  (for attribute values)
/// - ' → &apos;  (for attribute values)
///
/// Note: While HTML supports 252+ named entities (like &nbsp;, &copy;, etc.),
/// XML only requires these 5. The scraper library handles HTML entity decoding
/// automatically during parsing, so we don't need to protect other entities.
fn escape_xml_content(content: &str) -> String {
    // Multi-pass logic to handle existing entities properly

    // Pass 1: Protect common HTML entities to avoid double-escaping
    let protected_content = content
        // Basic XML entities
        .replace("&amp;", "__CCP_AMP__")
        .replace("&lt;", "__CCP_LT__")
        .replace("&gt;", "__CCP_GT__")
        .replace("&quot;", "__CCP_QUOT__")
        .replace("&apos;", "__CCP_APOS__")
        // Common HTML entities
        .replace("&nbsp;", "__CCP_NBSP__")
        .replace("&copy;", "__CCP_COPY__")
        .replace("&reg;", "__CCP_REG__")
        .replace("&trade;", "__CCP_TRADE__")
        .replace("&mdash;", "__CCP_MDASH__")
        .replace("&ndash;", "__CCP_NDASH__")
        .replace("&hellip;", "__CCP_HELLIP__")
        .replace("&laquo;", "__CCP_LAQUO__")
        .replace("&raquo;", "__CCP_RAQUO__")
        .replace("&ldquo;", "__CCP_LDQUO__")
        .replace("&rdquo;", "__CCP_RDQUO__")
        .replace("&lsquo;", "__CCP_LSQUO__")
        .replace("&rsquo;", "__CCP_RSQUO__")
        .replace("&euro;", "__CCP_EURO__")
        .replace("&pound;", "__CCP_POUND__")
        .replace("&yen;", "__CCP_YEN__")
        .replace("&cent;", "__CCP_CENT__")
        .replace("&plusmn;", "__CCP_PLUSMN__")
        .replace("&times;", "__CCP_TIMES__")
        .replace("&divide;", "__CCP_DIVIDE__")
        .replace("&ne;", "__CCP_NE__")
        .replace("&le;", "__CCP_LE__")
        .replace("&ge;", "__CCP_GE__");

    // Pass 2: Escape naked special characters that would break XML
    let escaped_content = protected_content
        .replace('&', "&amp;") // Must be first to avoid double-escaping
        .replace('<', "&lt;") // Prevents accidental tag creation
        .replace('>', "&gt;"); // Prevents malformed tags

    // Pass 3: Restore the protected entities
    escaped_content
        // Basic XML entities
        .replace("__CCP_AMP__", "&amp;")
        .replace("__CCP_LT__", "&lt;")
        .replace("__CCP_GT__", "&gt;")
        .replace("__CCP_QUOT__", "&quot;")
        .replace("__CCP_APOS__", "&apos;")
        // Common HTML entities
        .replace("__CCP_NBSP__", "&nbsp;")
        .replace("__CCP_COPY__", "&copy;")
        .replace("__CCP_REG__", "&reg;")
        .replace("__CCP_TRADE__", "&trade;")
        .replace("__CCP_MDASH__", "&mdash;")
        .replace("__CCP_NDASH__", "&ndash;")
        .replace("__CCP_HELLIP__", "&hellip;")
        .replace("__CCP_LAQUO__", "&laquo;")
        .replace("__CCP_RAQUO__", "&raquo;")
        .replace("__CCP_LDQUO__", "&ldquo;")
        .replace("__CCP_RDQUO__", "&rdquo;")
        .replace("__CCP_LSQUO__", "&lsquo;")
        .replace("__CCP_RSQUO__", "&rsquo;")
        .replace("__CCP_EURO__", "&euro;")
        .replace("__CCP_POUND__", "&pound;")
        .replace("__CCP_YEN__", "&yen;")
        .replace("__CCP_CENT__", "&cent;")
        .replace("__CCP_PLUSMN__", "&plusmn;")
        .replace("__CCP_TIMES__", "&times;")
        .replace("__CCP_DIVIDE__", "&divide;")
        .replace("__CCP_NE__", "&ne;")
        .replace("__CCP_LE__", "&le;")
        .replace("__CCP_GE__", "&ge;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_new_format_args() {
        let xml_input = r#"<cs:tool_use>
            <name>simple_tool</name>
            <args>
                <arg name="param1">value1</arg>
                <arg name="param2" value="value2" />
                <arg name="param3" type="int">123</arg>
            </args>
        </cs:tool_use>"#;

        let result = parse_tool_use(xml_input).unwrap();
        assert_eq!(result.name, "simple_tool");
        assert_eq!(result.args.len(), 3);

        assert_eq!(result.args[0].name, "param1");
        assert_eq!(result.args[0].get_value(), json!("value1"));

        assert_eq!(result.args[1].name, "param2");
        assert_eq!(result.args[1].get_value(), json!("value2"));

        assert_eq!(result.args[2].name, "param3");
        assert_eq!(result.args[2].get_value(), json!(123));
    }
    #[test]
    fn test_case_sensitive_args() {
        let xml_input = r#"<cs:tool_use>
            <name>simple_tool</name>
            <args>
                <arg name="paramTest">value1</arg>
                <arg name="Param2" value="value2" />
            </args>
        </cs:tool_use>"#;

        let result = parse_tool_use(xml_input).unwrap();
        assert_eq!(result.name, "simple_tool");
        assert_eq!(result.args.len(), 2);

        assert_eq!(result.args[0].name, "paramTest");
        assert_eq!(result.args[0].get_value(), json!("value1"));

        assert_eq!(result.args[1].name, "Param2");
        assert_eq!(result.args[1].get_value(), json!("value2"));
    }

    #[test]
    fn test_parse_mixed_and_custom_tags() {
        let xml_input = r#"<cs:tool_use>
            <name>custom_tool</name>
            <args>
                <arg name="traditional_arg">traditional_value</arg>
                <custom_param>custom_value</custom_param>
                <json_data type="array">[{"name": "test", "value": 42}]</json_data>
                <html_content>&lt;div&gt;This is HTML content&lt;/div&gt;</html_content>
            </args>
        </cs:tool_use>"#;

        let result = parse_tool_use(xml_input).unwrap();
        assert_eq!(result.name, "custom_tool");
        assert_eq!(result.args.len(), 4);

        // Traditional arg (still parsed for compatibility)
        let traditional_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "traditional_arg")
            .unwrap();
        assert_eq!(traditional_arg.get_value(), json!("traditional_value"));

        // Custom param as text
        let custom_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "custom_param")
            .unwrap();
        assert_eq!(custom_arg.get_value(), json!("custom_value"));

        // JSON data should be detected and parsed
        let json_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "json_data")
            .unwrap();
        assert_eq!(json_arg.data_type, Some(ParamType::Array));
        let json_value = json_arg.get_value();
        assert!(json_value.is_array());
        assert_eq!(json_value[0]["name"].as_str().unwrap(), "test");

        // HTML content should be treated as plain text
        let html_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "html_content")
            .unwrap();
        assert_eq!(
            html_arg.get_value(),
            json!("<div>This is HTML content</div>")
        );
    }

    #[test]
    fn test_html_content_as_plain_text() {
        let xml_input = r#"<cs:tool_use>
            <name>TodoWrite</name>
            <args>
                <arg name="description">Description of the todo item</arg>
                <arg name="todo" type="array">[{"item1":"xxx", "item2":"yyy", "item3":"zzz"}]</arg>
                <html_content>
                    <div>
                        <h1>Todo List</h1>
                        <ul>
                            <li>Item 1</li>
                            <li>Item 2</li>
                            <li>Item 3</li>
                        </ul>
                    </div>
                </html_content>
            </args>
        </cs:tool_use>"#;

        let result = parse_tool_use(xml_input).unwrap();
        assert_eq!(result.name, "TodoWrite");
        assert_eq!(result.args.len(), 3);

        // Check description
        let desc_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "description")
            .unwrap();
        assert_eq!(desc_arg.get_value(), json!("Description of the todo item"));

        // Check JSON todo data
        let todo_arg = result.args.iter().find(|arg| arg.name == "todo").unwrap();
        assert_eq!(todo_arg.data_type, Some(ParamType::Array));
        let todo_value = todo_arg.get_value();
        assert!(
            todo_value.is_array(),
            "Expected array, current type: {:?}",
            todo_value
        );

        // Check HTML content - should be plain text
        let html_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "html_content")
            .unwrap();
        let html_value = html_arg.get_value();
        assert!(html_value.is_string());

        let html_text = html_value.as_str().unwrap();
        assert!(html_text.contains("Todo List"));
        assert!(html_text.contains("Item 1"));
        assert!(html_text.contains("Item 2"));
        assert!(html_text.contains("Item 3"));
        // Should NOT have nested JSON structure
    }

    #[test]
    fn test_nested_arg_tags_treated_as_content() {
        // This is the exact scenario you were worried about
        let xml_input = r#"<cs:tool_use>
            <name>HTMLWithArgTags</name>
            <args>
                <arg name="html_content">
                    &lt;html&gt;
                        &lt;head&gt;&lt;title&gt;Test Page&lt;/title&gt;&lt;/head&gt;
                        &lt;body&gt;
                            &lt;p&gt;Some content&lt;/p&gt;
                            &lt;arg&gt;This should be treated as HTML content, not a parameter&lt;/arg&gt;
                            &lt;div&gt;More content&lt;/div&gt;
                        &lt;/body&gt;
                    &lt;/html&gt;
                </arg>
                <arg name="regular_param">regular_value</arg>
            </args>
        </cs:tool_use>"#;

        let result = parse_tool_use(xml_input).unwrap();
        assert_eq!(result.name, "HTMLWithArgTags");
        assert_eq!(result.args.len(), 2); // Only 2 top-level parameters

        let arg_names: Vec<&String> = result.args.iter().map(|p| &p.name).collect();
        assert!(arg_names.contains(&&"html_content".to_string()));
        assert!(arg_names.contains(&&"regular_param".to_string()));

        // Should NOT contain any nested "arg" parameters
        assert!(!arg_names.iter().any(|name| name == &"arg"));

        // Check the html_content parameter contains the nested arg tag as plain text
        let html_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "html_content")
            .unwrap();
        let html_value = html_arg.get_value();
        let html_text = html_value.as_str().unwrap();

        assert!(html_text.contains("This should be treated as HTML content"));
        assert!(html_text.contains("Test Page"));
        assert!(html_text.contains("<p>Some content</p>"));
        assert!(html_text
            .contains("<arg>This should be treated as HTML content, not a parameter</arg>"));

        // Check regular param works normally
        let regular_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "regular_param")
            .unwrap();
        assert_eq!(regular_arg.get_value(), json!("regular_value"));
    }

    #[test]
    fn test_invalid_tags() {
        // This is the exact scenario you were worried about
        let xml_input = r#"<ccp:tool_use>
            <tool_2a71d8e1>
            <id>tool_thinking</id>
            <name>thinking</name>
            <args>
            <arg name="content">用户的问题是：macOS 重启后自启动失败，但手动启动正常。从日志看程序能启动，SSH连接也能建立，但耗时很长。
            主要问题分析：
            1. SSH密钥路径在系统启动时可能无法访问
            2. NetworkStateUp可能只是网络接口up，但DNS/routing还没完全就绪
            3. SSH连接在启动初期网络不稳定时可能失败
            解决方案思路：
            1. 将SSH密钥复制到系统可访问的位置，比如/opt/tunfox/
            2. 添加更多启动条件，如MountPoints（确保文件系统就绪）
            3. 增加启动延迟或重试机制
            4. 优化SSH连接超时设置
            需要检查当前plist是否已经是最优配置，以及是否可以添加更多launchd配置选项。</arg>
            </args>
            </tool_2a71d8e1> "#;

        let result = parse_tool_use(xml_input).unwrap();
        assert_eq!(result.name, "thinking");
        assert_eq!(result.args.len(), 1); // Only 2 top-level parameters

        let arg_names: Vec<&String> = result.args.iter().map(|p| &p.name).collect();
        assert!(arg_names.contains(&&"content".to_string()));
    }

    #[test]
    fn test_complex_nested_as_plain_text() {
        let xml_input = r#"<cs:tool_use>
            <name>complex_tool</name>
            <args>
                <complex_data>
                    &lt;level1&gt;
                        &lt;level2&gt;
                            &lt;level3&gt;Deep nested content&lt;/level3&gt;
                        &lt;/level2&gt;
                    &lt;/level1&gt;
                    &lt;another_section&gt;
                        &lt;item&gt;Item 1&lt;/item&gt;
                        &lt;item&gt;Item 2&lt;/item&gt;
                    &lt;/another_section&gt;
                </complex_data>
                <arg name="test-arg">
                    &lt;arg name="test-arg1"&gt;Item 1&lt;/arg&gt;
                    &lt;arg name="test-arg2"&gt;Item 2&lt;/arg&gt;
                </arg>
            </args>
        </cs:tool_use>"#;

        let result = parse_tool_use(xml_input).unwrap();
        assert_eq!(result.name, "complex_tool");
        assert_eq!(result.args.len(), 2);

        let complex_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "complex_data")
            .unwrap();
        let complex_value = complex_arg.get_value();
        assert!(complex_value.is_string());

        let complex_text = complex_value.as_str().unwrap();
        assert!(complex_text.contains("Deep nested content"));
        assert!(complex_text.contains("<item>Item 1</item>"));
        assert!(complex_text.contains("Item 2"));

        let test_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "test-arg")
            .unwrap();

        let val = test_arg.get_value();
        let test_text = val.as_str().unwrap();

        assert!(test_text.contains("<arg name=\"test-arg1\">Item 1</arg>"));
    }

    #[test]
    fn test_self_closing_and_redundant_tags() {
        // Test self-closing <arg /> and redundant </cs:tool_use>
        let xml_input = r#"<cs:tool_use>
            <name>test_tool</name>
            <args>
                <arg name="param1" value="val1" />
                <arg name="param2" type="int">123</arg>
                <param3 value="val3" />
            </args>
        </cs:tool_use></cs:tool_use></cs:tool_use>"#;

        let result = parse_tool_use(xml_input).unwrap();
        assert_eq!(result.name, "test_tool");
        assert_eq!(result.args.len(), 3);

        assert_eq!(result.args[0].name, "param1");
        assert_eq!(result.args[0].get_value(), json!("val1"));

        assert_eq!(result.args[1].name, "param2");
        assert_eq!(result.args[1].get_value(), json!(123));

        assert_eq!(result.args[2].name, "param3");
        assert_eq!(result.args[2].get_value(), json!("val3"));
    }

    #[test]
    fn test_json_detection_and_parsing() {
        let xml_input = r#"<cs:tool_use>
            <name>json_tool</name>
            <args>
                <valid_json type="object">{"name": "test", "values": [1, 2, 3]}</valid_json>
                <invalid_json>{"name": "test", "incomplete":</invalid_json>
                <array_json type="array">[{"a": 1}, {"b": 2}]</array_json>
                <explicit_json type="object">{"explicit": true}</explicit_json>
                <not_json>This is just plain text</not_json>
            </args>
        </cs:tool_use>"#;

        let result = parse_tool_use(xml_input).unwrap();
        assert_eq!(result.name, "json_tool");
        assert_eq!(result.args.len(), 5);

        // Valid JSON should be parsed
        let valid_json_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "valid_json")
            .unwrap();
        assert_eq!(valid_json_arg.data_type, Some(ParamType::Object));
        let valid_json_value = valid_json_arg.get_value();
        assert!(valid_json_value.is_object());
        assert_eq!(valid_json_value["name"].as_str().unwrap(), "test");

        // Invalid JSON should be treated as plain text
        let invalid_json_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "invalid_json")
            .unwrap();
        // Manual scanner uses type if provided, but get_value will handle parsing failure
        assert_eq!(
            invalid_json_arg.get_value(),
            json!("{\"name\": \"test\", \"incomplete\":")
        );

        // Array JSON should be parsed
        let array_json_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "array_json")
            .unwrap();
        assert_eq!(array_json_arg.data_type, Some(ParamType::Array));
        let array_json_value = array_json_arg.get_value();
        assert!(array_json_value.is_array());

        // Explicit type should be respected
        let explicit_json_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "explicit_json")
            .unwrap();
        assert_eq!(explicit_json_arg.data_type, Some(ParamType::Object));
        let explicit_json_value = explicit_json_arg.get_value();
        assert!(explicit_json_value.is_object());

        // Plain text should remain as string
        let plain_text_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "not_json")
            .unwrap();
        assert_eq!(plain_text_arg.get_value(), json!("This is just plain text"));
    }

    #[test]
    fn test_mixed_traditional_and_custom() {
        let xml_input = r#"<cs:tool_use>
            <name>mixed_tool</name>
            <args>
                <arg name="traditional1" value="trad_value" />
                <custom1>"custom_value"</custom1>
                <arg name="traditional2" type="int">42</arg>
                <custom2 type="object">{"test_key": "data"}</custom2>
                <arg name="traditional3">text content</arg>
                <html_like>&lt;p&gt;HTML content&lt;/p&gt;</html_like>
            </args>
        </cs:tool_use>"#;

        let result = parse_tool_use(xml_input).unwrap();
        assert_eq!(result.name, "mixed_tool");
        assert_eq!(result.args.len(), 6);

        let arg_names: Vec<String> = result.args.iter().map(|arg| arg.name.clone()).collect();
        assert!(arg_names.contains(&"traditional1".to_string()));
        assert!(arg_names.contains(&"custom1".to_string()));
        assert!(arg_names.contains(&"traditional2".to_string()));
        assert!(arg_names.contains(&"custom2".to_string()));
        assert!(arg_names.contains(&"traditional3".to_string()));
        assert!(arg_names.contains(&"html_like".to_string()));

        // Verify specific values
        let trad1 = result
            .args
            .iter()
            .find(|arg| arg.name == "traditional1")
            .unwrap();
        assert_eq!(trad1.get_value(), json!("trad_value"));

        let custom1 = result
            .args
            .iter()
            .find(|arg| arg.name == "custom1")
            .unwrap();
        assert_eq!(custom1.get_value(), json!("\"custom_value\""));

        let trad2 = result
            .args
            .iter()
            .find(|arg| arg.name == "traditional2")
            .unwrap();
        assert_eq!(trad2.get_value(), json!(42));

        let custom2 = result
            .args
            .iter()
            .find(|arg| arg.name == "custom2")
            .unwrap();
        let custom2_value = custom2.get_value();
        assert!(custom2_value.is_object());
        assert_eq!(custom2_value["test_key"].as_str().unwrap(), "data");

        let html_like = result
            .args
            .iter()
            .find(|arg| arg.name == "html_like")
            .unwrap();
        assert_eq!(html_like.get_value(), json!("<p>HTML content</p>"));
    }

    #[test]
    fn test_generate_tools_xml() {
        let tools = vec![crate::ccproxy::adapter::unified::UnifiedTool {
            name: "test_tool".to_string(),
            description: Some("A test tool".to_string()),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "param1": {
                        "type": "string",
                        "description": "First parameter"
                    },
                    "param2": {
                        "type": "number",
                        "description": "Second parameter"
                    }
                },
                "required": ["param1"]
            }),
        }];

        let xml = generate_tools_xml(&tools);
        assert!(xml.contains("<cs:tools"));
        assert!(xml.contains("<cs:tool_define>"));
        assert!(xml.contains("<name>test_tool</name>"));
        assert!(xml.contains("<desc>A test tool</desc>"));
        assert!(xml.contains("<arg name=\"param1\""));
        assert!(xml.contains("First parameter (required)"));
        assert!(xml.contains("<arg name=\"param2\""));
        assert!(xml.contains("Second parameter (optional)"));
    }

    #[test]
    fn test_void_element_tag_as_argument() {
        let xml_input = r###"<cs:tool_use>
            <name>test_void_elements</name>
            <args>
                <param type="string">param value</param>
                <input type="number">123.5</input>
                <arg name="new_line" type="string">New
line</arg>
            </args>
        </cs:tool_use>"###;

        let result = parse_tool_use(xml_input).unwrap();
        assert_eq!(result.name, "test_void_elements");
        assert_eq!(result.args.len(), 3);

        let param_arg = result.args.iter().find(|arg| arg.name == "param").unwrap();
        assert_eq!(
            param_arg.get_value(),
            json!("param value"),
            "Parser should correctly extract content from <param> tag"
        );

        let input_arg = result.args.iter().find(|arg| arg.name == "input").unwrap();
        assert_eq!(
            input_arg.get_value(),
            json!(123.5),
            "Parser should correctly extract content from <input> tag"
        );
        let new_line_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "new_line")
            .unwrap();
        assert_eq!(
            new_line_arg.get_value(),
            json!("New\nline"),
            "Parser should correctly extract content from <arg> tag"
        );
    }

    #[test]
    fn test_attribute_flexibility() {
        // Test 1: Standard double quotes
        let h1 = r#"<arg name="test" type="string">"#;
        assert_eq!(extract_attr(h1, "name"), Some("test".to_string()));
        assert_eq!(extract_attr(h1, "type"), Some("string".to_string()));

        // Test 2: Single quotes and spaces
        let h2 = r#"<arg name = 'test'  type='int' >"#;
        assert_eq!(extract_attr(h2, "name"), Some("test".to_string()));
        assert_eq!(extract_attr(h2, "type"), Some("int".to_string()));

        // Test 3: Unquoted value
        let h3 = r#"<arg name=test type=bool>"#;
        assert_eq!(extract_attr(h3, "name"), Some("test".to_string()));
        assert_eq!(extract_attr(h3, "type"), Some("bool".to_string()));
    }

    #[test]
    fn test_read_tool_regression() {
        let xml_input = r#"<cs:tool_use>
<name>Read</name>
<args>
<arg name="file_path" type="string">/path/to/file.rs</arg>
<arg name="offset" type="number">755</arg>
<arg name="limit" type="number">200</arg>
</args>
</cs:tool_use>"#;

        let result = parse_tool_use(xml_input).unwrap();
        assert_eq!(result.name, "Read");
        assert_eq!(result.args.len(), 3);
        assert_eq!(result.args[0].name, "file_path");
        assert_eq!(result.args[1].get_value(), json!(755));
        assert_eq!(result.args[2].get_value(), json!(200));
    }

    #[test]
    fn test_xml_entity_unescaping() {
        // Test 1: Standard escaped entities
        let xml_input = r#"<cs:tool_use>
            <name>test_tool</name>
            <args>
                <arg name="escaped">if (a &lt; b &amp;&amp; b &gt; c) return &quot;found&quot;;</arg>
                <arg name="raw">if (a < b && b > c) return "found";</arg>
                <arg name="mixed">Escaped &lt; and Raw < inside same tag</arg>
                <arg name="html_entity">Price: &yen;100 &nbsp; Done</arg>
            </args>
        </cs:tool_use>"#;

        let result = parse_tool_use(xml_input).unwrap();

        // Both standard escaped and raw should result in the same unescaped string
        let expected = "if (a < b && b > c) return \"found\";";
        assert_eq!(result.args[0].get_value(), json!(expected));
        assert_eq!(result.args[1].get_value(), json!(expected));

        // Mixed should work correctly
        assert_eq!(
            result.args[2].get_value(),
            json!("Escaped < and Raw < inside same tag")
        );

        // HTML entities should be decoded (thanks to html_escape)
        let html_val = result.args[3].get_value();
        let html_str = html_val.as_str().unwrap();
        assert!(html_str.contains("¥100"));
        assert!(html_str.contains("\u{a0}")); // non-breaking space
    }

    #[test]
    fn test_integer_vs_float_type_preservation() {
        let xml_input = r#"<cs:tool_use>
            <name>type_test</name>
            <args>
                <int_param type="integer">42</int_param>
                <float_param type="number">42.5</float_param>
                <uint_param type="uint64">18446744073709551615</uint_param>
                <large_int type="integer">9223372036854775807</large_int>
            </args>
        </cs:tool_use>"#;

        let result = parse_tool_use(xml_input).unwrap();
        assert_eq!(result.name, "type_test");
        assert_eq!(result.args.len(), 4);

        // Integer should remain as integer
        let int_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "int_param")
            .unwrap();
        let int_value = int_arg.get_value();
        assert!(int_value.is_i64(), "Integer should be parsed as i64");
        assert_eq!(int_value.as_i64().unwrap(), 42);

        // Float should be parsed as float
        let float_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "float_param")
            .unwrap();
        let float_value = float_arg.get_value();
        assert!(float_value.is_f64(), "Float should be parsed as f64");
        assert_eq!(float_value.as_f64().unwrap(), 42.5);

        // Large unsigned integer
        let uint_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "uint_param")
            .unwrap();
        let uint_value = uint_arg.get_value();
        assert!(uint_value.is_u64(), "Large uint should be parsed as u64");
        assert_eq!(uint_value.as_u64().unwrap(), 18446744073709551615u64);

        // Large signed integer
        let large_int_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "large_int")
            .unwrap();
        let large_int_value = large_int_arg.get_value();
        assert!(
            large_int_value.is_i64(),
            "Large int should be parsed as i64"
        );
        assert_eq!(large_int_value.as_i64().unwrap(), 9223372036854775807i64);
    }

    #[test]
    fn test_format_tool_use_xml_type_detection() {
        let input = json!({
            "string_param": "hello",
            "int_param": 42,
            "float_param": 42.5,
            "bool_param": true,
            "array_param": [1, 2, 3],
            "object_param": {"key": "value"}
        });

        let xml = format_tool_use_xml("test_id", "test_tool", &input);

        // Check that integers are marked as "integer" not "number"
        assert!(xml.contains(r#"<arg name="int_param" type="integer">42</arg>"#));
        // Check that floats are marked as "number"
        assert!(xml.contains(r#"<arg name="float_param" type="number">42.5</arg>"#));
        // Check other types
        assert!(xml.contains(r#"<arg name="string_param" type="string">hello</arg>"#));
        assert!(xml.contains(r#"<arg name="bool_param" type="boolean">true</arg>"#));
        assert!(xml.contains(r#"<arg name="array_param" type="array">[1,2,3]</arg>"#));
    }

    #[test]
    fn test_complex_double_escaping_for_multiedit() {
        // 测试复杂的双重转义逻辑，用于 MultiEdit 工具的 HTML 内容
        let xml_input = r#"<cs:tool_use>
            <name>MultiEdit</name>
            <args>
                <arg name="file_path" type="string">/path/to/index.html</arg>
                <arg name="edits" type="array">[{"old_string":"&lt;p id=\"greeting\"&gt;Say \"Hello\" to Q&amp;A&lt;/p&gt;","new_string":"&lt;div class=\"message\"&gt;Say \"Hello\" to Questions &amp; Answers&lt;/div&gt;"}]</arg>
            </args>
        </cs:tool_use>"#;

        let result = parse_tool_use(xml_input).unwrap();
        assert_eq!(result.name, "MultiEdit");
        assert_eq!(result.args.len(), 2);

        // 检查 edits 数组参数
        let edits_arg = result.args.iter().find(|arg| arg.name == "edits").unwrap();
        assert_eq!(edits_arg.data_type, Some(ParamType::Array));

        let edits_value = edits_arg.get_value();
        assert!(edits_value.is_array());
        assert_eq!(edits_value.as_array().unwrap().len(), 1);

        let edit_obj = &edits_value[0];
        assert!(edit_obj.is_object());

        let edit_map = edit_obj.as_object().unwrap();
        assert!(edit_map.contains_key("old_string"));
        assert!(edit_map.contains_key("new_string"));

        // 验证双重转义后的内容
        let old_string = edit_map["old_string"].as_str().unwrap();
        let new_string = edit_map["new_string"].as_str().unwrap();

        // 正确的结果应该是 XML 转义被解析器还原后的内容
        assert_eq!(old_string, "<p id=\"greeting\">Say \"Hello\" to Q&A</p>");
        assert_eq!(
            new_string,
            "<div class=\"message\">Say \"Hello\" to Questions & Answers</div>"
        );

        // 验证 XML 解析器正确还原了转义字符
        assert!(old_string.contains("Q&A"), "&amp; should be unescaped to &");
        assert!(old_string.contains("<p"), "&lt; should be unescaped to <");
        assert!(old_string.contains("</p>"), "&gt; should be unescaped to >");
        assert!(
            old_string.contains("\"greeting\""),
            "JSON quotes should be preserved"
        );

        assert!(
            new_string.contains("Questions & Answers"),
            "&amp; should be unescaped to &"
        );
        assert!(new_string.contains("<div"), "&lt; should be unescaped to <");
        assert!(
            new_string.contains("</div>"),
            "&gt; should be unescaped to >"
        );
        assert!(
            new_string.contains("\"message\""),
            "JSON quotes should be preserved"
        );
    }
}
