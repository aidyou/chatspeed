use serde::Deserialize;
use serde_json::{json, Value};

use crate::ccproxy::{adapter::unified::UnifiedContentBlock, helper::get_tool_id};

#[derive(Deserialize, Debug)]
pub struct ToolUse {
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "params")]
    pub params: Params,
}

impl TryFrom<&str> for ToolUse {
    type Error = anyhow::Error;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match quick_xml::de::from_str::<ToolUse>(s) {
            Ok(tool_use) => Ok(tool_use),
            Err(_) => parse_tool_use(s),
        }
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
        for param in tool_use.params.param {
            arguments.insert(param.name.clone(), param.get_value());
        }

        UnifiedContentBlock::ToolUse {
            id: get_tool_id(),
            name: tool_use.name,
            input: serde_json::Value::Object(arguments),
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct Params {
    #[serde(rename = "param", default)]
    pub param: Vec<Param>,
}

#[derive(Deserialize, Debug)]
pub struct Param {
    #[serde(rename = "@name")]
    pub name: String,

    #[serde(rename = "@type")]
    pub data_type: Option<String>,

    // parse the type attribute: <param name="location" type="string" value="beijing" />
    #[serde(rename = "@value")]
    pub value: Option<String>,

    // parse the text content: <param name="location">beijing</param>
    #[serde(rename = "$text", default)]
    pub text_content: Option<String>,
}

impl Param {
    pub fn get_value(&self) -> Value {
        let raw_value = self
            .value
            .as_ref()
            .map(|s| s.as_str())
            .or_else(|| self.text_content.as_ref().map(|s| s.as_str()))
            .unwrap_or_default();

        // Explicitly unescape the raw value to handle XML entities like &lt;, &amp;, etc.
        let value = quick_xml::escape::unescape(raw_value)
            .map(|cow| cow.into_owned())
            .unwrap_or_else(|_| raw_value.to_string()); // Fallback to raw value on error

        if let Some(data_type) = self.data_type.as_ref() {
            match data_type.as_str() {
                "float" | "double" | "f32" | "f64" | "number" | "float32" | "float64" => {
                    // Trim whitespace as intended for numeric types.
                    // If parsing fails, fallback to the original (unescaped) string value.
                    if let Ok(f) = value.trim().parse::<f64>() {
                        json!(f)
                    } else {
                        json!(value)
                    }
                }
                "int" | "integer" => {
                    if let Ok(i) = value.trim().parse::<i64>() {
                        json!(i)
                    } else {
                        json!(value)
                    }
                }
                "bool" | "boolean" => {
                    if let Ok(b) = value.trim().parse::<bool>() {
                        json!(b)
                    } else {
                        json!(value)
                    }
                }
                _ => json!(value), // For string and other types, preserve original value.
            }
        } else {
            Self::try_parse_value(&value)
        }
    }

    pub fn try_parse_value(s: &str) -> Value {
        if let Ok(b) = s.parse::<bool>() {
            return Value::Bool(b);
        }
        if let Ok(i) = s.parse::<i64>() {
            return json!(i);
        }
        if let Ok(f) = s.parse::<f64>() {
            return json!(f);
        }
        if (s.starts_with('[') && s.ends_with(']')) || (s.starts_with('{') && s.ends_with('}')) {
            if let Ok(json_value) = serde_json::from_str(s) {
                return json_value;
            }
        }
        serde_json::Value::String(s.to_string())
    }
}

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
            let required_params: std::collections::HashSet<String> = schema
                .get("required")
                .and_then(|r| r.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();

            if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
                tools_xml.push_str("<params>\n");

                let mut required_keys: Vec<&String> = Vec::new();
                let mut optional_keys: Vec<&String> = Vec::new();

                for name in properties.keys() {
                    if required_params.contains(name) {
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

                    if required_params.contains(name) {
                        description.push_str(" (required)");
                    } else {
                        description.push_str(" (optional)");
                    }

                    tools_xml.push_str(&format!(
                        "<param name=\"{}\" type=\"{}\">{}</param>\n",
                        name, param_type, description
                    ));
                };

                for name in required_keys {
                    append_param(name);
                }
                for name in optional_keys {
                    append_param(name);
                }

                tools_xml.push_str("</params>\n");
            }
        }
        tools_xml.push_str("</ccp:tool_define>\n");
    }
    tools_xml.push_str("</ccp:tools>");
    tools_xml
}

pub fn format_tool_use_xml(id: &str, name: &str, input: &serde_json::Value) -> String {
    let mut params_xml = String::new();
    if let Some(obj) = input.as_object() {
        for (key, value) in obj {
            let value_str = match value {
                serde_json::Value::String(s) => s.clone(),
                _ => value.to_string(),
            };
            // Per user request, the type attribute is omitted for simplicity.
            params_xml.push_str(&format!(
                "<param name=\"{}\">{}</param>\n",
                key,
                quick_xml::escape::escape(&value_str)
            ));
        }
    }

    format!(
        "<ccp:tool_use>\n<id>{}</id>\n<name>{}</name>\n<params>\n{}\n</params>\n</ccp:tool_use>",
        id, name, params_xml
    )
}

/// A lenient parser for tool use XML that ignores unknown tags within the <params> block.
///
/// This function is designed to be more robust against malformed XML from AI models,
/// where unexpected tags might be present. It manually iterates through the XML events.
pub fn parse_tool_use(xml_str: &str) -> Result<ToolUse, anyhow::Error> {
    let mut reader = quick_xml::Reader::from_str(xml_str.trim());
    let mut buf = Vec::new();

    let mut tool_name = None;
    let mut params_vec: Vec<Param> = Vec::new();

    // Loop until we find the opening <ccp:tool_use> tag, ignoring whitespace/comments etc.
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(e)) => {
                if e.name().as_ref() == b"ccp:tool_use" {
                    // Found it, break the loop and proceed.
                    break;
                } else {
                    // Found a different start tag, which is an error.
                    return Err(anyhow::anyhow!(
                        "Expected <ccp:tool_use> as root, found <{}>",
                        String::from_utf8_lossy(e.name().as_ref())
                    ));
                }
            }
            Ok(quick_xml::events::Event::Eof) => {
                return Err(anyhow::anyhow!(
                    "Unexpected EOF while looking for <ccp:tool_use> start tag"
                ));
            }
            Err(e) => return Err(e.into()),
            // Ignore other events like Text, Comment, etc.
            _ => (),
        }
    }
    buf.clear();

    // Inside <ccp:tool_use>, look for <name> and <params>
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(quick_xml::events::Event::Start(e)) => {
                log::info!(
                    "=== Start event: {}",
                    String::from_utf8_lossy(e.name().as_ref())
                );

                match e.name().as_ref() {
                    b"name" => {
                        if tool_name.is_none() {
                            let txt = reader.read_text(e.name())?;
                            tool_name = Some(txt.into_owned());
                        }
                    }
                    b"params" => {
                        // Enter the <params> block and parse its children leniently
                        let mut params_depth = 1;
                        while params_depth > 0 {
                            buf.clear();
                            match reader.read_event_into(&mut buf) {
                                Ok(quick_xml::events::Event::Start(param_e)) => {
                                    log::info!(
                                        "=== Params start event: {}",
                                        String::from_utf8_lossy(param_e.name().as_ref())
                                    );

                                    if param_e.name().as_ref() == b"param" {
                                        let mut param_name = None;
                                        let mut param_type = None;
                                        let mut param_value = None;
                                        for attr in param_e.attributes() {
                                            let attr = attr?;
                                            if attr.key.as_ref() == b"name" {
                                                param_name = Some(String::from_utf8(
                                                    attr.value.into_owned(),
                                                )?);
                                            } else if attr.key.as_ref() == b"type" {
                                                param_type = Some(String::from_utf8(
                                                    attr.value.into_owned(),
                                                )?);
                                            } else if attr.key.as_ref() == b"value" {
                                                param_value = Some(String::from_utf8(
                                                    attr.value.into_owned(),
                                                )?);
                                            }
                                        }
                                        let param_text = reader.read_text(param_e.name())?;
                                        if let Some(name) = param_name {
                                            params_vec.push(Param {
                                                name,
                                                data_type: param_type,
                                                value: param_value,
                                                text_content: Some(param_text.into_owned()),
                                            });
                                        }
                                    } else {
                                        // This is the "ignore" logic for unknown tags like <description>
                                        reader.read_to_end_into(param_e.name(), &mut Vec::new())?;
                                    }
                                }
                                Ok(quick_xml::events::Event::End(_)) => {
                                    params_depth -= 1;
                                }
                                Ok(quick_xml::events::Event::Eof) => {
                                    return Err(anyhow::anyhow!("Unexpected EOF inside <params>"));
                                }
                                Err(e) => return Err(e.into()),
                                _ => (),
                            }
                        }
                    }
                    // Ignore any other tags directly under <ccp:tool_use>
                    _ => {
                        reader.read_to_end_into(e.name(), &mut Vec::new())?;
                    }
                }
            }
            Ok(quick_xml::events::Event::End(e)) if e.name().as_ref() == b"ccp:tool_use" => {
                break; // End of the main tag
            }
            Ok(quick_xml::events::Event::Eof) => {
                return Err(anyhow::anyhow!("Unexpected EOF inside <ccp:tool_use>"));
            }
            Err(e) => return Err(e.into()),
            _ => (),
        }
        buf.clear();
    }

    let name = tool_name.ok_or_else(|| anyhow::anyhow!("<name> tag not found"))?;
    Ok(ToolUse {
        name,
        params: Params { param: params_vec },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tool_use_with_malformed_xml() {
        let malformed_xml = r#"
        <ccp:tool_use>
            <id>tool_2c2c8f78-2ad3-42f1-9dd2-bb1a2a7a5d29</id>
            <name>Bash</name>
            <params>
            <param name="command">cd /Volumes/dev/personal/dev/python/csv_to_api &amp;&amp; python3 -c "from utils import read_csv_files; import json; data = read_csv_files(); print('price records:', len(data.get('price', []))); print('store records:', len(data.get('store', [])))"</param>
            <param name="description">测试Python环境并验证CSV读取功能</param>

            </params>
        </ccp:tool_use>
        "#;

        let result = parse_tool_use(malformed_xml);
        assert!(
            result.is_ok(),
            "Lenient parsing should succeed, but failed with: {:?}",
            result.err()
        );
        let tool_use = result.unwrap();

        assert_eq!(tool_use.name, "Bash");
        // It should ignore <description>, <subagent_type> and the nested <params>
        // and only find the valid <param> tag.
        assert_eq!(
            tool_use.params.param.len(),
            2,
            "Should have found exactly one valid param"
        );

        let param = &tool_use.params.param[0];
        log::debug!("value: {}", param.get_value().as_str().unwrap_or_default());
        assert_eq!(param.name, "command");
        assert!(param
            .get_value()
            .as_str()
            .unwrap()
            .starts_with("cd /Volumes/dev/personal/dev/python/csv_to_api &&"),);
    }
}
