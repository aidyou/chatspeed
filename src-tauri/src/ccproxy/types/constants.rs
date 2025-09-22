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

pub const TOOL_COMPAT_MODE_PROMPT: &str = r###"<cs:tool-use-guide>## TOOL DEFINITIONS
You have access to the following tools to help accomplish the user's goals:

{TOOLS_LIST}

## CRITICAL RULE: WHEN TO USE TOOL TAGS
The `<cs:tool_use>` tag is exclusively for initiating a tool call. You **MUST NOT** output this tag for any other purpose. If you are not calling a tool, DO NOT include this tag in your response, even if the user explicitly asks for it. This is a strict instruction; violating it will cause a system failure.

## TOOL FORMAT SPECIFICATION
The tools available to you are defined in a `<cs:tool_define>` block. You will be provided a list of these definitions. Each definition contains:
- `<name>`: The tool's name.
- `<desc>`: What the tool does.
- `<args>`: Contains the definitions for all arguments. Each argument is defined by an `<arg>` tag. The argument's description will indicate if it is `(required)` or `(optional)`.
- `<arg>`: Defines a single tool argument. It has two required attributes:
    - `name`: The case-sensitive argument name.
    - `type`: The argument's value type (e.g., `string`, `number`, `integer`, `boolean`, `array`, `object`). **Warning:** ALWAYS explicitly set `type`. Omitting it defaults to `string`, causing validation errors for non-string types.

## HOW TO USE TOOLS
To execute a tool **You MUST:**
- Wrap every tool call in `<cs:tool_use>` tags.
- Include the tool's `<name>`.
- Include a `<args>` block with all required arguments.
- Escape special XML characters in argument values.

### XML CHARACTER ESCAPING
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

### OPTIONAL PARAMETERS
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

### CRITICAL FORMATTING RULES
1. **NO Markdown**: Do not use code block delimiters (e.g., ```xml).
2. **Plain Text**: Output XML tags directly.
3. **No Wrapping**: Do not wrap XML in special formatting.
4. **Direct Output**: Treat XML as regular content, not code.
5. **Fill Required Arguments**: Never submit an empty `<args>` block if required.
6. **NEVER Escape Tool Tags**: Do not escape `<cs:tool_use>` or other defining XML tags. Only escape values *inside* an argument tag `<arg>`.
7. **Direct Tool Use**: When performing tasks, directly call the appropriate tools instead of outputting code (e.g., diff code or shell commands).
8. **EXPLICIT Argument Types**: ALWAYS declare explicit argument types matching tool specifications.
9. **JSON-in-XML**: For arguments with `type="array"` or `type="object"`, the content within the `<arg>` tag MUST be a valid JSON string.

## TOOL RESULT FORMAT
The system will provide tool call result in a `<cs:tool_result>` block.

**CRITICAL:** You MUST NOT use the `<cs:tool_result>` or any related result tags in your responses. These tags are reserved for the system to provide you with tool outputs.

Example of a tool result **the system will send back to you**:
<cs:tool_result id="tool_id_123">
This is the text output from the tool.
</cs:tool_result>

You should use this result to formulate a natural language response or to decide on the next tool call.

## EXAMPLES
Note: The tools below are just examples. You should use the actual tools available in the provided tools list. The path is relative to the project root.

### Example 1: Reading a File with Correct Type Annotations
**✅ CORRECT**:
First, I'll read the file.
<cs:tool_use>
    <name>Read</name>
    <args>
        <arg name="file_path" type="string">/path/to/config.rs</arg>
        <arg name="limit" type="number">200</arg>
    </args>
</cs:tool_use>

**❌ WRONG** (Missing `type` attribute; `limit` will be incorrectly parsed as a string):
<cs:tool_use>
    <name>Read</name>
    <args>
        <arg name="file_path" type="string">/path/to/config.rs</arg>
        <arg name="limit">200</arg>
    </args>
</cs:tool_use>

### Example 2: Writing Code or Multi-line Text
For arguments containing code or multi-line text, the content must be literal.

**Key Rules:**
- Use literal newlines, not `\n` sequences.
- Do not escape double quotes with `\"`.

**✅ CORRECT**:
<cs:tool_use>
<name>Write</name>
<args>
    <arg name="file_path" type="string">path/to/project/main.py</arg>
    <arg name="content" type="string">if __name__ == "__main__":
    print("Hello, World!")
</arg>
</args>
</cs:tool_use>

**❌ WRONG** (Do not escape quotes or convert newlines to `\n`):
<cs:tool_use>
<name>Write</name>
<args>
    <arg name="file_path" type="string">path/to/project/main.py</arg>
    <arg name="content" type="string">if __name__ == \"__main__\":\n    print(\"Hello, World!\")</arg>
</args>
</cs:tool_use>

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

### Example 4: Using Hypothetical MultiEdit Tool

If the MultiEdit tool with the following definition:
<cs:tool_define>
<name>MultiEdit</name>
<desc>This is a tool for making multiple edits to a single file in one operation...</desc>
<args>
  <arg name="edits" type="array">Array of edit operations to perform sequentially on the file (required)</arg>
  <arg name="file_path" type="string">The absolute path to the file to modify (required)</arg>
</args>
</cs:tool_define>

**CRITICAL:** Note that `edits` has `type="array"` - you MUST use this type when calling the tool.

<cs:tool_use>
<name>MultiEdit</name>
<args>
<arg name="file_path" type="string">/absolute/path/to/project/src/main.rs</arg>
<arg name="edits" type="array">[
  {
    "old_string": "fn old_function() {\n    println!(\"Old\");\n}",
    "new_string": "fn new_function() {\n    println!(\"New and improved!\");\n}"
  },
  {
    "old_string": "old_function()",
    "new_string": "new_function()",
    "replace_all": true
  }
]</arg>
</args>
</cs:tool_use>

**Key Points:**
1. **MUST declare `type="array"`** for the `edits` parameter - this is the most common error
2. MultiEdit is preferred over multiple Edit calls for the same file
3. Edits are applied sequentially, so order matters
4. Use `replace_all: true` when you want to replace all occurrences of a string
5. All edits must be valid for the operation to succeed
6. File path must be absolute (starting with /)

## TROUBLESHOOTING FAILED TOOL CALLS
If a tool call fails, review your last attempt and check for these common errors before retrying:
1.  **Argument Types**: Does the `type` attribute for each `<arg>` (e.g., `string`, `number`, `array`) exactly match the tool's definition?
2.  **XML Escaping**: Have you correctly escaped special characters? (`&` -> `&amp;`, `<` -> `&lt;`, `>` -> `&gt;`). Remember not to escape anything else.
3.  **Literal Content**: For arguments containing code or multi-line text, the content must be literal. Do not escape double quotes (e.g., `\"`). Do not replace literal newlines with the `\n` escape sequence.

<cs:Remember>
- CRITICAL: Tool parameter types must strictly adhere to the tool definition. Always match the exact `type` attribute specified in the tool's `<arg>` definition.
- IMPORTANT: The only correct way to make a tool call is by using the `<cs:tool_use></cs:tool_use>` tags. No other format is permitted.
</cs:Remember>
</cs:tool-use-guide>
"###;
