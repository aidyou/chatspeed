use serde::Deserialize;
use std::sync::RwLockWriteGuard;

use crate::ccproxy::adapter::unified::SseStatus;

pub fn update_message_block(status: &mut RwLockWriteGuard<'_, SseStatus>, block: String) {
    if !status.current_content_block.is_empty() && status.current_content_block != block {
        status.message_index += 1;
    }
    status.current_content_block = block;
}

#[derive(Deserialize, Debug)]
pub struct ToolUse {
    #[serde(rename = "name")]
    pub name: String,
    #[serde(rename = "params")]
    pub params: Params,
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

    // parse the type attribute: <param name="location" type="string" value="beijing" />
    #[serde(rename = "@value")]
    pub value: Option<String>,

    // parse the text content: <param name="location">beijing</param>
    #[serde(rename = "$text", default)]
    pub text_content: Option<String>,
}

impl Param {
    pub fn get_value(&self) -> String {
        // use the @value attribate first
        self.value
            .as_ref()
            .map(|s| s.trim())
            .or_else(|| self.text_content.as_deref().map(|s| s.trim()))
            .unwrap_or_default()
            .to_string()
    }
}

pub const TOOL_TAG_START: &str = "<ccp:tool_use>";
pub const TOOL_TAG_END: &str = "</ccp:tool_use>";

fn generate_tools_xml(tools: &Vec<crate::ccproxy::adapter::unified::UnifiedTool>) -> String {
    let mut tools_xml = String::new();
    tools_xml.push_str("<ccp:tools description=\"You available tools\">\n");

    for tool in tools {
        tools_xml.push_str(&format!(
            "<ccp:tool_use>\n<name>{}</name>\n<description>{}</description>\n",
            tool.name,
            tool.description.as_deref().unwrap_or("")
        ));

        if let Some(schema) = tool.input_schema.as_object() {
            if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
                tools_xml.push_str("<params>\n");
                for (name, details) in properties {
                    let param_type = details
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("any");
                    let description = details
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("");
                    tools_xml.push_str(&format!(
                        "<param name=\"{}\" type=\"{}\">{}</param>\n",
                        name, param_type, description
                    ));
                }
                tools_xml.push_str("</params>\n");
            }
        }
        tools_xml.push_str("</ccp:tool_use>\n");
    }
    tools_xml.push_str("</ccp:tools>");
    tools_xml
}

pub fn generate_tool_prompt(tools: &Vec<crate::ccproxy::adapter::unified::UnifiedTool>) -> String {
    let tools_xml = generate_tools_xml(tools);

    let template = r#"You have access to the following tools:

{TOOLS_LIST}

Each tool in the above list is defined within <ccp:tool_use>...</ccp:tool_use> tags, containing:
- <name>tool_name</name>: The name of the tool.
- <description>tool_description</description>: A brief description of what the tool does and when to use it.
- <params>: The parameters the tool accepts, along with their expected types.

To use a tool, respond with an XML block formatted as follows:
<ccp:tool_use>
    <name>TOOL_NAME</name>
    <params>
        <param name=\"PARAM_NAME\" type=\"PARAM_TYPE\">PARAM_VALUE</param>
    </params>
</ccp:tool_use>

CRITICAL FORMATTING RULES - READ CAREFULLY:
1. NEVER use markdown code blocks (```xml, ```, or any ``` delimiters)
2. NEVER wrap the XML in any formatting
3. Output the XML tags directly as plain text in your response
4. Do NOT treat the XML as code - treat it as regular response text
5. The <ccp:tool_use> tags should appear directly in your message, not in a code block

WRONG (DO NOT DO THIS):
```xml
<ccp:tool_use>
    <name>get_weather</name>
    <params>
        <param name="city" type="string">Tokyo</param>
    </params>
</ccp:tool_use>
```

CORRECT (DO THIS):
<ccp:tool_use>
    <name>get_weather</name>
    <params>
        <param name="city" type="string">Tokyo</param>
    </params>
</ccp:tool_use>

IMPORTANT: When performing the same operation multiple times with different parameters (e.g., checking weather for multiple cities, querying different dates, etc.), YOU MUST use separate tool calls for each instance. Provide one <ccp:tool_use> block per operation, even if the operations are similar.

Example: If asked to read two files, make two separate tool calls:
<ccp:tool_use>
    <name>Read</name>
    <params>
        <param name=\"file_path\" type=\"string\">/path/to/a.txt</param>
    </params>
</ccp:tool_use>
<ccp:tool_use>
    <name>Read</name>
    <params>
        <param name=\"file_path\" type=\"string\">/path/to/b.txt</param>
    </params>
</ccp:tool_use>

Remember: The XML tool calls are part of your normal response text, not code blocks. Output them directly without any markdown formatting.

---

User Question:
"#;

    template.replace("{TOOLS_LIST}", &tools_xml)
}
