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

    let template = r#"You have access to the following tools to help accomplish the user's goals:

{TOOLS_LIST}

## TOOL USAGE PHILOSOPHY
Always prioritize using available tools to provide concrete, actionable solutions rather than generic responses. Tools are your primary means of helping users achieve their objectives.

## TOOL SELECTION GUIDELINES
1. **Analyze First**: Carefully examine the user's request to identify which tools can help
2. **Choose Appropriately**: Select the most suitable tool(s) based on task requirements
3. **Think Sequentially**: If multiple tools are needed, plan their logical sequence
4. **Consider Limitations**: Understand each tool's capabilities and constraints
5. **Be Proactive**: Use tools without waiting for explicit instructions when they can solve the problem

## WHEN TO USE TOOLS
- **File Operations**: Reading, writing, or manipulating files
- **Data Processing**: Transformation, analysis, or computation tasks
- **External Services**: API calls or service interactions when external data is needed
- **System Operations**: Environment queries or system-level tasks
- **Information Retrieval**: Search or query operations for specific information

## TOOL FORMAT SPECIFICATION
Each tool is defined within <ccp:tool_use>...</ccp:tool_use> tags containing:
- <name>tool_name</name>: The tool identifier
- <description>tool_description</description>: What the tool does and when to use it
- <params>: Required parameters with their types and descriptions

## HOW TO USE TOOLS
Format your tool calls as XML blocks like this:
<ccp:tool_use>
    <name>TOOL_NAME</name>
    <params>
        <param name=\"PARAM_NAME\" type=\"PARAM_TYPE\">PARAM_VALUE</param>
    </params>
</ccp:tool_use>

## CRITICAL FORMATTING RULES
1. **NO Markdown**: Never use ```xml or any code block delimiters
2. **Plain Text**: Output XML tags directly in your response text
3. **No Wrapping**: Don't wrap XML in any special formatting
4. **Direct Output**: Treat XML as regular response content, not code

## EXAMPLES

**WRONG** (Never do this):
```xml
<ccp:tool_use>
    <name>get_weather</name>
    <params>
        <param name=\"city\" type=\"string\">Tokyo</param>
    </params>
</ccp:tool_use>
```

**CORRECT** (Always do this):
<ccp:tool_use>
    <name>get_weather</name>
    <params>
        <param name=\"city\" type=\"string\">Tokyo</param>
    </params>
</ccp:tool_use>

## MULTIPLE OPERATIONS
When performing similar operations with different parameters, use separate tool calls:

<ccp:tool_use>
    <name>Read</name>
    <params>
        <param name=\"file_path\" type=\"string\">/path/to/first.txt</param>
    </params>
</ccp:tool_use>
<ccp:tool_use>
    <name>Read</name>
    <params>
        <param name=\"file_path\" type=\"string\">/path/to/second.txt</param>
    </params>
</ccp:tool_use>

## DECISION FRAMEWORK
Before responding, ask yourself:
- Can available tools accomplish this task? → **Use tools**
- Does the user need specific data or actions? → **Use appropriate tools**
- Would tools provide more accurate/current information? → **Use tools**
- Is this a general question that tools can answer concretely? → **Use tools**

## BEST PRACTICES
1. **Proactive Usage**: Consider tools first, generic responses second
2. **Logical Chaining**: Sequence multiple tools thoughtfully
3. **Parameter Validation**: Ensure parameters match expected types
4. **Error Handling**: Be prepared for tool failures and have alternatives
5. **User Context**: Consider the user's broader goals when selecting tools

Remember: Your primary job is to leverage these tools effectively to solve user problems, not just to provide information about them.

---

User Question:
"#;

    template.replace("{TOOLS_LIST}", &tools_xml)
}
