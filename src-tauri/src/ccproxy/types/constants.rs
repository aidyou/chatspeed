pub const TOOL_TAG_START: &str = "<cs:tool_use>";
pub const TOOL_TAG_END: &str = "</cs:tool_use>";
pub const TODO_TAG_START: &str = "<cs:todo>";
pub const TODO_TAG_END: &str = "</cs:todo>";

pub const TOOL_PARSE_ERROR_REMINDER: &str = r#"<system-reminder>
Your last tool call had an invalid XML format and could not be parsed. Please check carefully and strictly follow the tool usage specifications.
Common reasons for failure:
1. Required arguments are missing.
2. XML special characters `&`, `<`, `>` must be escaped as `&amp;`, `&lt;`, `&gt;` respectively.
3. Do not escape other characters (e.g., `"`).
</system-reminder>"#;

pub const TOOL_ARG_ERROR_REMINDER: &str = r#"<system-reminder>
Your last tool call's argument contained malformed JSON and could not be parsed. The failed call is displayed above in a <cs:failed_tool_call> tag for your reference.
Please review JSON format carefully. Do not generate <cs:failed_tool_call> tags yourself.
Common reasons for failure:
1. Malformed JSON (e.g., trailing commas, mismatched brackets).
2. Incorrectly quoted JSON strings/keys (must use double quotes).
3. JSON structure does not match the tool's input schema.
</system-reminder>"#;

// pub const TOOL_RESULT_REMINDER: &str = r#"<system-reminder>
// This is the result of your last tool call. Use it to decide your next step. Do not output `<cs:tool_result>` tags yourself.
// </system-reminder>"#;

pub const TOOL_COMPAT_MODE_PROMPT: &str = r###"<cs:tool-use-guide>
You have access to the following tools to help accomplish the user's goals:

{TOOLS_LIST}

**CRITICAL RULE: WHEN TO USE TOOL TAGS**
The `<cs:tool_use>` tag is exclusively for initiating a tool call. You **MUST NOT** output this tag for any other purpose. If you are not calling a tool, DO NOT include this tag in your response, even if the user explicitly asks for it. This is a strict instruction; violating it will cause a system failure.

## TOOL USAGE PHILOSOPHY""
Always prioritize using available tools to provide concrete, actionable solutions rather than generic responses. Tools are your primary means of helping users achieve their objectives.

## TOOL FORMAT SPECIFICATION
The tools available to you are defined in a `<cs:tool_define>` block. You will be provided a list of these definitions. Each definition contains:
- `<name>`: The tool's name.
- `<desc>`: What the tool does.
- `<args>`: Contains the definitions for all arguments. Each argument is defined by an `<arg>` tag. The argument's description will indicate if it is `(required)` or `(optional)`.
- `<arg>`: Defines a single tool argument. It has two required attributes:
    - `name`: The case-sensitive argument name.
    - `type`: The argument's value type (e.g., `string`, `number`, `integer`, `boolean`, `array`, `object`). **Warning:** ALWAYS explicitly set `type`. Omitting it defaults to `string`, causing validation errors for non-string types.

## ARGUMENT DATA TYPE


## HOW TO USE TOOLS

To execute a tool **You MUST:**
- Wrap every tool call in `<cs:tool_use>` tags.
- Include the tool's `<name>`.
- Include a `<args>` block with all required arguments.
- Escape special XML characters in argument values.

## TOOL RESULT FORMAT
The system will provide tool call result in a `<cs:tool_result>` block.

**CRITICAL:** You MUST NOT use the `<cs:tool_result>` or any related result tags in your responses. These tags are reserved for the system to provide you with tool outputs.

Example of a tool result **the system will send back to you**:
<cs:tool_result id="tool_id_123">
This is the text output from the tool.
</cs:tool_result>

You should use this result to formulate a natural language response or to decide on the next tool call.

## XML CHARACTER ESCAPING
**CRITICAL:** Only `&`, `<`, `>` have special XML meaning and MUST be escaped in argument values:
- `&` must be written as `&amp;`
- `<` must be written as `&lt;`
- `>` must be written as `&gt;`

**All other characters are literal (e.g., `\"`, `\'`, `\\`, newlines) and MUST NOT be escaped.**

Example for a value containing '&':
<cs:tool_use>
    <name>Bash</name>
    <args>
        <arg name="command" type="string">echo "Start..." &amp;&amp; sh -c "/path/to/script.sh"</arg>
    </args>
</cs:tool_use>

## CRITICAL FORMATTING RULES
1. **NO Markdown**: Do not use code block delimiters (e.g., ```xml).
2. **Plain Text**: Output XML tags directly.
3. **No Wrapping**: Do not wrap XML in special formatting.
4. **Direct Output**: Treat XML as regular content, not code.
5. **Fill Required Arguments**: Never submit an empty `<args>` block if required.
6. **NEVER Escape Tool Tags**: Do not escape `<cs:tool_use>` or other defining XML tags. Only escape values *inside* an argument tag.
7. **Direct Tool Use**: When performing tasks, directly call the appropriate tools instead of outputting code (e.g., diff code or shell commands).
8. **EXPLICIT Argument Types**: ALWAYS declare explicit argument types matching tool specifications.


## EXAMPLES
Note: The `Read` and `Write` tools below are just examples. You should use the actual tools available in the provided tools list. The path is relative to the project root.

### Example 1: Reading a File with Correct Type Annotations
**✅ CORRECT**:
First, I'll read the file.
<cs:tool_use>
    <name>Read</name>
    <args>
        <arg name="file_path" type="string">path/to/project/config.toml</arg>
        <arg name="offset" type="number">100</arg>
        <arg name="limit" type="number">200</arg>
    </args>
</cs:tool_use>

**❌ WRONG** (The `limit` argument is missing its `type` attribute, and will be parsed as a string):
<cs:tool_use>
    <name>Read</name>
    <args>
        <arg name="file_path" type="string">path/to/project/config.toml</arg>
        <arg name="offset" type="number">100</arg>
        <arg name="limit">200</arg>
    </args>
</cs:tool_use>

**❌ WRONG** (Raw commands not allowed):
```bash
cat path/to/project/config.toml
```

**Key Point:** ALWAYS specify the `type` attribute for parameters. Without it, values are automatically parsed as strings. This will cause tool calls to fail for non-string types (e.g., numbers, arrays, or objects) that expect a specific type.

### Example 2: Creating a File
**✅ CORRECT**:
I will create the `.gitignore` file.
<cs:tool_use>
    <name>Write</name>
    <args>
        <arg name="file_path" type="string">path/to/project/.gitignore</arg>
        <arg name="content" type="string">node_modules
dist
.env</arg>
    </args>
</cs:tool_use>

**❌ WRONG** (Raw commands not allowed):
```bash
echo "node_modules\ndist\n.env" > path/to/project/.gitignore
```

### Example 3: Using Array Arguments
For array arguments (e.g., a list of items), format the value as a single JSON array string and set the type attribute appropriately (e.g., `array`, `object`).

**✅ CORRECT**:
<cs:tool_use>
<name>TodoWrite</name>
<args>
<arg name="todos" type="array">[
  {
    "activeForm": "Task description",
    "content": "Task details",
    "status": "completed"
  },
  // ... (more items)
]</arg>
</args>
</cs:tool_use>

**❌ WRONG** (You MUST set the todos argument type!):
<cs:tool_use>
<name>TodoWrite</name>
<args>
<arg name="todos">[
  {
    "activeForm": "Task description",
    "content": "Task details",
    "status": "completed"
  },
  // ... (more items)
]</arg>
</args>
</cs:tool_use>

**✅ CORRECT**:
<cs:tool_use>
<name>Max</name>
<args>
<arg name="data" type="array">[1,2,3]</arg>
</args>
</cs:tool_use>

**❌ WRONG** (You MUST set the data argument type!):
<cs:tool_use>
<name>Max</name>
<args>
<arg name="data">[1,2,3]</arg>
</args>
</cs:tool_use>

### Example 4: Writing Code
Place the entire code block within a single argument. Indentation and whitespace are preserved.
Write double quotes directly; **DO NOT** escape them with backslashes (`\`).

**✅ CORRECT**:
<cs:tool_use>
<name>Write</name>
<args>
    <arg name="file_path" type="string">path/to/project/main.py</arg>
    <arg name="content" type="string">def main():
    message = "Hello, World!"
    print(message)

if __name__ == "__main__":
    main()
</arg>
</args>
</cs:tool_use>

**❌ WRONG** (Do not escape quotes with `\`):
<cs:tool_use>
<name>Write</name>
<args>
    <arg name="file_path" type="string">path/to/project/main.py</arg>
    <arg name="content" type="string">def main():
    message = \"Hello, World!\"
    print(message)

if __name__ == \"__main__\":
    main()
</arg>
</args>
</cs:tool_use>

## OPTIONAL PARAMETERS
Optional arguments (marked `(optional)`) can be:
1. **Omitted** if not needed.
2. **Included** with an empty value if supported.

**Example with optional parameters:**
<cs:tool_use>
    <name>Read</name>
    <args>
        <arg name="file_path" type="string">path/to/file</arg>
        <!-- limit and offset is optional and omitted when the entire file  -->
    </args>
</cs:tool_use>

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
- IMPORTANT: The only correct way to make a tool call is by using the `<cs:tool_use></cs:tool_use>` tags. No other format is permitted.
</cs:Remember>
"###;
