use scraper::{ElementRef, Html, Selector};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::ccproxy::{adapter::unified::UnifiedContentBlock, helper::get_tool_id};

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
    pub data_type: Option<String>,

    // Holds the attribute value or the inner HTML content.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

impl Arg {
    pub fn get_value(&self) -> Value {
        let raw_value = self.value.as_deref().unwrap_or_default();

        // Use HTML decoding for entities since we're using HTML parser now
        let value = html_escape::decode_html_entities(raw_value).to_string();

        if let Some(data_type) = self.data_type.as_ref() {
            match data_type.as_str() {
                "float" | "double" | "f32" | "f64" | "number" | "float32" | "float64" => {
                    // Trim whitespace as intended for numeric types.
                    // If parsing fails, fallback to the original (unescaped) string value.
                    if let Ok(f) = value.trim().parse::<f64>() {
                        return json!(f);
                    }
                }
                "int" | "i32" | "i64" | "integer" | "long" | "int32" | "int64" => {
                    if let Ok(i) = value.trim().parse::<i64>() {
                        return json!(i);
                    }
                }
                "bool" | "boolean" => {
                    if let Ok(b) = value.trim().parse::<bool>() {
                        return json!(b);
                    }
                }
                "array" | "list" | "vec" => {
                    if let Ok(arr) = serde_json::from_str::<Vec<Value>>(value.trim()) {
                        return json!(arr);
                    }
                }
                "object" | "map" | "dict" => {
                    if let Ok(obj) = serde_json::from_str::<Map<String, Value>>(value.trim()) {
                        return json!(obj);
                    }
                }
                "json" => {
                    if let Ok(json_val) = serde_json::from_str::<Value>(value.trim()) {
                        return json_val;
                    }
                }
                _ => {} // Default to string for unknown types
            }
        }

        json!(value)
    }
}

/// Generate XML for tool definitions
pub fn generate_tools_xml(tools: &Vec<crate::ccproxy::adapter::unified::UnifiedTool>) -> String {
    let mut tools_xml = String::new();
    tools_xml.push_str("<ccp:tools description=\"You available tools\">\n");

    for tool in tools {
        tools_xml.push_str(&format!(
            "<ccp:tool_define>\n<name>{}</name>\n<description>{}</description>\n",
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
        tools_xml.push_str("</ccp:tool_define>\n");
    }
    tools_xml.push_str("</ccp:tools>");
    tools_xml
}

pub fn format_tool_use_xml(id: &str, name: &str, input: &serde_json::Value) -> String {
    let mut args_xml = String::new();
    if let Some(obj) = input.as_object() {
        for (key, value) in obj {
            let value_str = match value {
                serde_json::Value::String(s) => s.clone(),
                _ => value.to_string(),
            };
            // Escape the content to avoid XML parsing issues
            let escaped_value = escape_xml_content(&value_str);
            args_xml.push_str(&format!("<arg name=\"{}\">{}</arg>\n", key, escaped_value));
        }
    }

    format!(
        "<ccp:tool_use>\n<id>{}</id>\n<name>{}</name>\n<args>\n{}\n</args>\n</ccp:tool_use>",
        id, name, args_xml
    )
}

/// Parse tool use XML using scraper - SIMPLIFIED ONE-LEVEL ONLY
pub fn parse_tool_use(xml_str: &str) -> Result<ToolUse, anyhow::Error> {
    // Parse as HTML fragment for better error tolerance
    let document = Html::parse_fragment(xml_str);

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
    // Per user request, get the full inner content, including HTML tags and all whitespace.
    let content = element.inner_html();

    // The data_type is ONLY determined by the explicit `type` attribute.
    let explicit_type = element.value().attr("type").map(|s| s.to_string());

    Ok(Some(Arg {
        name: tag_name.to_string(),
        data_type: explicit_type,
        value: Some(content),
    }))
}

/// Parse a single <arg> element
fn parse_arg_element(element: &ElementRef) -> Result<Option<Arg>, anyhow::Error> {
    let name = element
        .value()
        .attr("name")
        .ok_or_else(|| anyhow::anyhow!("arg element missing name attribute"))?
        .to_string();

    let explicit_type = element.value().attr("type").map(|s| s.to_string());

    if let Some(value) = element.value().attr("value").map(|s| s.to_string()) {
        return Ok(Some(Arg {
            name,
            data_type: explicit_type,
            value: Some(value),
        }));
    }

    // No 'value' attribute, so use inner_html to preserve nested tags and all whitespace.
    let content = element.inner_html();

    Ok(Some(Arg {
        name,
        data_type: explicit_type,
        value: Some(content),
    }))
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
    fn test_parse_simple_traditional_args() {
        let xml_input = r#"<ccp:tool_use>
            <name>simple_tool</name>
            <args>
                <arg name="param1">value1</arg>
                <arg name="param2" value="value2" />
                <arg name="param3" type="int">123</arg>
            </args>
        </ccp:tool_use>"#;

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
    fn test_parse_custom_tags() {
        let xml_input = r#"<ccp:tool_use>
            <name>custom_tool</name>
            <args>
                <arg name="traditional_arg">traditional_value</arg>
                <custom_param>custom_value</custom_param>
                <json_data type="json">[{"name": "test", "value": 42}]</json_data>
                <html_content><div>This is HTML content</div></html_content>
            </args>
        </ccp:tool_use>"#;

        let result = parse_tool_use(xml_input).unwrap();
        assert_eq!(result.name, "custom_tool");
        assert_eq!(result.args.len(), 4);

        // Traditional arg
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
        assert_eq!(json_arg.data_type, Some("json".to_string()));
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
        let xml_input = r#"<ccp:tool_use>
            <name>TodoWrite</name>
            <args>
                <arg name="description">Description of the todo item</arg>
                <arg name="todo" type="json">[{"item1":"xxx", "item2":"yyy", "item3":"zzz"}]</arg>
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
        </ccp:tool_use>"#;

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
        assert_eq!(todo_arg.data_type, Some("json".to_string()));
        let todo_value = todo_arg.get_value();
        assert!(todo_value.is_array());

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
        let xml_input = r#"<ccp:tool_use>
            <name>HTMLWithArgTags</name>
            <args>
                <arg name="html_content">
                    <html>
                        <head><title>Test Page</title></head>
                        <body>
                            <p>Some content</p>
                            <arg>This should be treated as HTML content, not a parameter</arg>
                            <div>More content</div>
                        </body>
                    </html>
                </arg>
                <arg name="regular_param">regular_value</arg>
            </args>
        </ccp:tool_use>"#;

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
    fn test_todo_write_example() {
        // Test the exact example from the user's description
        let xml_input = r#"<ccp:tool_use>
            <name>TodoWrite</name>
            <args>
                <arg name="description">Description of the todo item</arg>
                <arg name="todo" type="json">[{"item1":"xxx", "item2":"yyy", "item3":"zzz"}]</arg>
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
        </ccp:tool_use>"#;

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

        // Check todo JSON data
        let todo_arg = result.args.iter().find(|arg| arg.name == "todo").unwrap();
        assert_eq!(todo_arg.data_type, Some("json".to_string()));
        let todo_value = todo_arg.get_value();
        assert!(todo_value.is_array());
        let todo_array = todo_value.as_array().unwrap();
        assert_eq!(todo_array.len(), 1);
        let todo_object = &todo_array[0];
        assert_eq!(todo_object["item1"].as_str().unwrap(), "xxx");
        assert_eq!(todo_object["item2"].as_str().unwrap(), "yyy");
        assert_eq!(todo_object["item3"].as_str().unwrap(), "zzz");

        // Check HTML content - should be plain text with newlines
        let html_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "html_content")
            .unwrap();
        let html_value = html_arg.get_value();
        assert!(html_value.is_string());

        let html_text = html_value.as_str().unwrap();
        assert!(html_text.contains("Todo List"));
        assert!(html_text.contains("<li>Item 1</li>"));
        assert!(html_text.contains("<li>Item 2</li>"));
        assert!(html_text.contains("Item 3"));
        // Should contain newlines and whitespace from original formatting
        assert!(html_text.contains("\n"));
    }

    #[test]
    fn test_complex_nested_as_plain_text() {
        let xml_input = r#"<ccp:tool_use>
            <name>complex_tool</name>
            <args>
                <complex_data>
                    <level1>
                        <level2>
                            <level3>Deep nested content</level3>
                        </level2>
                    </level1>
                    <another_section>
                        <item>Item 1</item>
                        <item>Item 2</item>
                    </another_section>
                </complex_data>
                <arg name="test-arg">
                    <arg name="test-arg1">Item 1</arg>
                    <arg name="test-arg2">Item 2</arg>
                </arg>
            </args>
        </ccp:tool_use>"#;

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
        // All content should be flattened to plain text
        //
        let test_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "test-arg")
            .unwrap();

        let val = test_arg.get_value();
        let complex_text = val.as_str().unwrap();

        assert!(complex_text.contains("<arg name=\"test-arg1\">Item 1</arg>"));
    }

    #[test]
    fn test_json_detection_and_parsing() {
        let xml_input = r#"<ccp:tool_use>
            <name>json_tool</name>
            <args>
                <valid_json type="json">{"name": "test", "values": [1, 2, 3]}</valid_json>
                <invalid_json>{"name": "test", "incomplete":</invalid_json>
                <array_json type="json">[{"a": 1}, {"b": 2}]</array_json>
                <explicit_json type="json">{"explicit": true}</explicit_json>
                <not_json>This is just plain text</not_json>
            </args>
        </ccp:tool_use>"#;

        let result = parse_tool_use(xml_input).unwrap();
        assert_eq!(result.name, "json_tool");
        assert_eq!(result.args.len(), 5);

        // Valid JSON should be parsed
        let valid_json_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "valid_json")
            .unwrap();
        assert_eq!(valid_json_arg.data_type, Some("json".to_string()));
        let valid_json_value = valid_json_arg.get_value();
        assert!(valid_json_value.is_object());
        assert_eq!(valid_json_value["name"].as_str().unwrap(), "test");

        // Invalid JSON should be treated as plain text
        let invalid_json_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "invalid_json")
            .unwrap();
        assert!(invalid_json_arg.data_type.is_none());
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
        assert_eq!(array_json_arg.data_type, Some("json".to_string()));
        let array_json_value = array_json_arg.get_value();
        assert!(array_json_value.is_array());

        // Explicit type should be respected
        let explicit_json_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "explicit_json")
            .unwrap();
        assert_eq!(explicit_json_arg.data_type, Some("json".to_string()));
        let explicit_json_value = explicit_json_arg.get_value();
        assert!(explicit_json_value.is_object());

        // Plain text should remain as string
        let plain_text_arg = result
            .args
            .iter()
            .find(|arg| arg.name == "not_json")
            .unwrap();
        assert!(plain_text_arg.data_type.is_none());
        assert_eq!(plain_text_arg.get_value(), json!("This is just plain text"));
    }

    #[test]
    fn test_mixed_traditional_and_custom() {
        let xml_input = r#"<ccp:tool_use>
            <name>mixed_tool</name>
            <args>
                <arg name="traditional1" value="trad_value" />
                <custom1>"custom_value"</custom1>
                <arg name="traditional2" type="int">42</arg>
                <custom2 type="json">{"json": "data"}</custom2>
                <arg name="traditional3">text content</arg>
                <html_like><p>HTML content</p></html_like>
            </args>
        </ccp:tool_use>"#;

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
        assert_eq!(custom2_value["json"].as_str().unwrap(), "data");

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
        assert!(xml.contains("<name>test_tool</name>"));
        assert!(xml.contains("A test tool"));
        assert!(xml.contains("param1"));
        assert!(xml.contains("param2"));
    }
}
