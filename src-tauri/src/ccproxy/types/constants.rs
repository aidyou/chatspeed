pub const TOOL_TAG_START: &str = "<ccp:tool_use>";
pub const TOOL_TAG_END: &str = "</ccp:tool_use>";
pub const TODO_TAG_START: &str = "<ccp:todo>";
pub const TODO_TAG_END: &str = "</ccp:todo>";

pub const TOOL_PARSE_ERROR_REMINDER: &str = r#"<system-reminder>
Your last tool call had an invalid XML format and could not be parsed. Please check carefully and strictly follow the tool usage specifications.
Common reasons for failure:
1. Required arguments are missing.
2. Special XML characters were not escaped. You must escape the following characters in argument values:
- `&` must be written as `&amp;`
- `<` must be written as `&lt;`
- `>` must be written as `&gt;`
3. You tried to escape a character that should not be escaped. All characters other than `<>&` should be written directly without escaping. For example, write double quotes as `"` not `\\"`.
</system-reminder>"#;

pub const TOOL_ARG_ERROR_REMINDER: &str = r#"<system-reminder>
The 'input' argument for your last tool call contained malformed JSON and could not be parsed. Please check the argument format carefully and strictly follow the JSON specification.
Common reasons for failure:
1. The JSON is not well-formed (e.g., trailing commas, mismatched brackets).
2. Strings and object keys are not enclosed in double quotes ("). Single quotes are not permitted.
3. The JSON structure does not match the tool's required input schema.
</system-reminder>"#;

pub const TOOL_RESULT_REMINDER: &str = r#"<system-reminder>
This is the result of your last tool call. Use this information to decide your next step. Do not output `<ccp:tool_results>` tags yourself.
</system-reminder>"#;

pub const TOOL_COMPAT_MODE_PROMPT: &str = r###"<cs:tool-use-guide>
You have access to the following tools to help accomplish the user's goals:

{TOOLS_LIST}

## TOOL USAGE PHILOSOPHY
Always prioritize using available tools to provide concrete, actionable solutions rather than generic responses. Tools are your primary means of helping users achieve their objectives.

## TOOL FORMAT SPECIFICATION
The tools available to you are defined in a `<ccp:tool_define>` block. You will be provided a list of these definitions. Each definition contains:
- `<name>`: The tool's name.
- `<description>`: What the tool does.
- `<args>`: A list of `<arg>` tags for each argument. The argument's description will indicate if it is `(required)` or `(optional)`.

## HOW TO USE TOOLS

To execute a tool
**You MUST:**
- Wrap every tool call in `<ccp:tool_use>` tags.
- Include the tool's `<name>`.
- Include a `<args>` block containing all required arguments for that tool.
- Escape special XML characters in argument values.

**You MUST NOT:**
- Include an `<id>` tag in your output. The system assigns the ID automatically. This is a critical rule.

## TOOL RESULT FORMAT
After the system executes your tool call, the result will be provided back to you in a `<ccp:tool_results>` block. This format is **only for the system to give you results**.

**CRITICAL:** You MUST NOT use the `<ccp:tool_results>` or `<ccp:tool_result>` tags in your own responses.

Example of a tool result **the system will send back to you**:
```xml
<ccp:tool_results>
    <ccp:tool_result>
        <id>tool_id_123</id>
        <result>This is the text output from the tool.</result>
    </ccp:tool_result>
</ccp:tool_results>
```
You should use this result to formulate a natural language response or to decide on the next tool call.

## XML CHARACTER ESCAPING
This is a critical rule. Only the following three characters have special meaning in XML and MUST be escaped when they appear in an argument's value:
- `&` must be written as `&amp;`
- `<` must be written as `&lt;`
- `>` must be written as `&gt;`

**All other characters are treated as literal characters and will be used as-is.** This includes double quotes (`"`), single quotes (`'`), backslashes (`\`), newlines, and all other symbols. DO NOT attempt to escape them.

Example for a value containing '&':
<ccp:tool_use>
<name>Search</name>
<args>
<arg name="query">echo "Start..." &amp;&amp; sh -c "/path/to/script.sh"</arg>
</args>
</ccp:tool_use>

## CRITICAL FORMATTING RULES
1. **NO Markdown**: Never use ```xml or any code block delimiters
2. **Plain Text**: Output XML tags directly in your response text
3. **No Wrapping**: Don't wrap XML in any special formatting
4. **Direct Output**: Treat XML as regular response content, not code
5. **Fill Required Arguments**: Never submit a tool call with an empty `<args>` block if the tool has required arguments.
6. **NEVER Escape Tool Tags**: The `<ccp:tool_use>` and other defining XML tags must be output as plain text. DO NOT escape them (e.g., do not write `&lt;ccp:tool_use&gt;`). Only the *values inside* a `<arg>` tag should be escaped.

## EXAMPLES
Note: The `Read` and `Write` tools below are just examples. You should use the actual tools available in the provided tools list. The path is relative to the project root.

### Example 1: Reading a File
**✅ CORRECT**:
First, I'll read the file.
<ccp:tool_use>
    <name>Read</name>
    <args>
        <arg name="file_path">path/to/project/config.toml</arg>
    </args>
</ccp:tool_use>

**❌ WRONG** (Do not output raw commands):
```bash
cat path/to/project/config.toml
```

### Example 2: Creating a File
**✅ CORRECT**:
I will create the `.gitignore` file.
<ccp:tool_use>
    <name>Write</name>
    <args>
        <arg name="file_path">path/to/project/.gitignore</arg>
        <arg name="content">node_modules
dist
.env</arg>
    </args>
</ccp:tool_use>

**❌ WRONG** (Do not output raw commands):
```bash
echo "node_modules\ndist\n.env" > path/to/project/.gitignore
```

### Example 3: Using Array Arguments
When a tool argument is an array (e.g., a list of items), you MUST format its value as a single JSON array string and explicitly set the type attribute to `json`.

**✅ CORRECT**:
<ccp:tool_use>
<name>ToolWithList</name>
<args>
<arg name="items" type="json">[
  {
    "id": "item1",
    "value": "First item"
  },
  {
    "id": "item2",
    "value": "Second item"
  }
]</arg>
</args>
</ccp:tool_use>

**❌ WRONG** (Do not format array arguments as nested XML tags):
<ccp:tool_use>
    <name>ToolWithList</name>
    <args>
    <items>
        <item>
            <id>item1</id>
            <value>First item</value>
        </item>
    </items>
    </args>
</ccp:tool_use>

### Example 4: Writing Python Code
When providing code, place the entire block within a single argument. All indentation and whitespace will be preserved.
Double quotes inside the code should be written directly. **DO NOT** escape double quotes with backslashes (`\`).

**✅ CORRECT**:
<ccp:tool_use>
<name>Write</name>
<args>
    <arg name="file_path">path/to/project/main.py</arg>
    <arg name="content">def main():
    message = "Hello, World!"
    print(message)

if __name__ == "__main__":
    main()
</arg>
</args>
</ccp:tool_use>

**❌ WRONG** (Do not escape quotes with `\`):
<ccp:tool_use>
<name>Write</name>
<args>
    <arg name="file_path">path/to/project/main.py</arg>
    <arg name="content">def main():
    message = "Hello, World!"
    print(message)

if __name__ == "__main__":
    main()
</arg>
</args>
</ccp:tool_use>

## DECISION FRAMEWORK
Before responding, ask yourself:
- Is this a complex task that requires multiple steps? → First, create a plan using the appropriate planning tool (e.g., `TodoWrite`) if available.
- Can available tools accomplish this task? → Use tools
- Does the user need specific data or actions? → Use appropriate tools
- Would tools provide more accurate/current information? → Use tools
- Is this a general question that tools can answer concretely? → Use tools

## BEST PRACTICES
1. Plan First: For any non-trivial task, create a step-by-step plan using a planning tool (like `TodoWrite`) before executing the first step.
2. Proactive Usage: Consider tools first, generic responses second
3. Logical Chaining: Sequence multiple tools thoughtfully
4. Argument Validation: Ensure Arguments match expected types
5. Error Handling: Be prepared for tool failures and have alternatives
6. User Context: Consider the user's broader goals when selecting tools

<cs:Remember>
- Your primary job is to leverage these tools effectively to solve user problems, not just to provide information about them.
- IMPORTANT: The only correct way to make a tool call is by using the `<ccp:tool_use></ccp:tool_use>` tags. No other format is permitted.
</cs:Remember>
"###;
