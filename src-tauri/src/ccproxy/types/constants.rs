pub const TOOL_TAG_START: &str = "<cs:tool_use>";
pub const TOOL_TAG_END: &str = "</cs:tool_use>";
pub const TODO_TAG_START: &str = "<cs:todo>";
pub const TODO_TAG_END: &str = "</cs:todo>";

pub const TOOL_CALL_EMPTY_REMAIN:&str = "<system-reminder>The tool call was cancelled or failed. The model can try again later if necessary.</system-reminder>";

pub const TOOL_PARSE_ERROR_REMINDER: &str = r###"<system-reminder>
Your last tool call had an invalid XML format and could not be parsed. Please check carefully and strictly follow the tool usage specifications.
Common reasons for failure:
1. Required arguments are missing.
2. XML special characters `&`, `<`, `>` must be escaped as `&amp;`, `&lt;`, `&gt;` respectively.
3. Do not escape other characters (e.g., `"`).
</system-reminder>"###;

pub const TOOL_ARG_ERROR_REMINDER: &str = r###"<system-reminder>
Your last tool call's argument contained malformed JSON and could not be parsed. The failed call is displayed above in a <cs:failed_tool_call> tag for your reference.
Please review JSON format carefully. Do not generate <cs:failed_tool_call> tags yourself.
Common reasons for failure:
1. Malformed JSON (e.g., trailing commas, mismatched brackets).
2. Incorrectly quoted JSON strings/keys (must use double quotes).
3. JSON structure does not match the tool's input schema.
</system-reminder>"###;

// pub const TOOL_RESULT_REMINDER: &str = r#"<system-reminder>
// This is the result of your last tool call. Use it to decide your next step. Do not output `<cs:tool_result>` tags yourself.
// </system-reminder>"#;

pub const TOOL_RESULT_SUFFIX_REMINDER: &str = r#"<system-reminder>This result is only for notification and display of the tool call result. It is not a question from the user. You do not need to reply to the user based on this result. Only consider whether further tasks need to be performed.</system-reminder>"#;

/// IMPORTANT: Do not attempt to remove any examples from this prompt.
/// Each example has been added after testing to regulate the AI's behavior.
/// They provide critical reference points to improve the success rate of AI tool execution.
pub const TOOL_COMPAT_MODE_PROMPT: &str = r###"<cs:tool-use-guide>## TOOL DEFINITIONS
You have access to the following tools to help accomplish the user's goals:

{TOOLS_LIST}

## HOW TO USE TOOLS
To call a tool, you must generate an XML block wrapped in `<cs:tool_use></cs:tool_use>` tags.

### 1. Critical Rules
- The `<cs:tool_use>` tag is ONLY for calling a tool. NEVER output this tag for any other reason.
- Set the `type` attribute for EVERY `<arg>` (e.g., `string`, `number`, `array`). This is the most common error.
- For tools that modify files (like `Edit` or `MultiEdit`), the `old_string` argument MUST be a perfect, character-for-character copy of the text in the file. Do not simplify or reformat code syntax.
- DO NOT use Markdown (like ```xml) or any other special formatting around the tool XML block.

### 2. Tool Call Structure
- `<name>`: The tool's name.
- `<args>`: Contains all arguments for the tool.
- `<arg>`: A single argument, which MUST have a `name` and `type` attribute.

### 3. Argument Formatting: The Most Common Source of Errors

#### 3.1. XML Character Escaping (Applies to ALL argument values)
Because your tool calls are XML, you MUST escape the following special characters within any argument value:
- `&` must be written as `&amp;`
- `<` must be written as `&lt;`
- `>` must be written as `&gt;`
IMPORTANT: Do not escape any other characters at the XML level (e.g., write `"` as is, not `&quot;`).

#### 3.2. Value Formatting based on Type
The content inside an `<arg>` tag depends on its `type` attribute.

- For `type="string"` (Literal Content)
  The value is treated as a raw, literal string, AS IS.
  - Newlines: Use literal newlines, not `\n`.
  - Quotes: Write quotes like `"` and `'` directly, do not escape them like `\"`.
  - XML Escaping: You MUST still escape `&`, `<`, `>` as described in 3.1.
  - For file paths, do NOT add any newlines or leading/trailing whitespace. The path must be exact.

- For `type="array"` or `type="object"` (JSON Content)
  The value MUST be a single, valid JSON string.
  - Inside the JSON string, you MUST follow standard JSON escaping rules (e.g., use `"` for quotes, `\n` for newlines).
  - After creating the JSON string, you MUST also apply XML escaping (see 3.1) to the entire string.

## EXAMPLES

> Note: The tools and paths below are examples. Use the actual tools and paths provided.

### Example 1: Reading a File
This example is critical because it shows that `limit` requires `type="number"`. Forgetting this will cause the tool to fail.

**✅ CORRECT** (All args have explicit `type` attributes):
<cs:tool_use>
    <name>Read</name>
    <args>
        <arg name="file_path" type="string">/path/to/config.rs</arg>
        <arg name="limit" type="number">200</arg>
    </args>
</cs:tool_use>

### Example 2: Writing Multi-line Code (`type="string"`)
**✅ CORRECT** (Content is literal, with real newlines and no escaped quotes):
<cs:tool_use>
    <name>Write</name>
    <args>
        <arg name="file_path" type="string">/path/to/main.py</arg>
        <arg name="content" type="string">if __name__ == "__main__":
    print("Hello, World!")
</arg>
    </args>
</cs:tool_use>

### Example 3: Writing an HTML File (`type="string"`)
For `type="string"`, the content is a literal block of text. However, because the entire tool call is XML, you **MUST** still escape the special XML characters (`<`, `>`, `&`) within the string value.

**✅ CORRECT** (HTML tags are escaped):
<cs:tool_use>
    <name>Write</name>
    <args>
        <arg name="file_path" type="string">/path/to/index.html</arg>
        <arg name="content" type="string">&lt;!DOCTYPE html&gt;
&lt;html&gt;
&lt;body&gt;
    &lt;h1&gt;My First Heading&lt;/h1&gt;
&lt;/body&gt;
&lt;/html&gt;
</arg>
    </args>
</cs:tool_use>

**❌ WRONG** (Raw HTML tags will break the XML structure):
<cs:tool_use>
    <name>Write</name>
    <args>
        <arg name="file_path" type="string">/path/to/index.html</arg>
        <arg name="content" type="string"><!DOCTYPE html>
<html>...</html>
</arg>
    </args>
</cs:tool_use>

### Example 4: Complex Edit on HTML with `MultiEdit` (`type="array"`)
This is the most complex scenario. It requires a "double escaping" process.

Scenario: Replace `<p id="hi">Say "Hello" to Q&A</p>` with `<div class="hello">Say "Hello" to Questions & Answers</div>`.

The Process (Double Escaping):
For `type="array"` or `type="object"`, the value MUST be a single, valid JSON string.
1. JSON Escaping: Within this JSON string, you MUST follow standard JSON escaping rules (e.g., use `\"` for quotes, `\n` for newlines).
2. XML Escaping: After creating the JSON string, you MUST ALSO apply XML escaping (see 3.1) to the entire string. This means `&` → `&amp;`, `<` → `&lt;`, and `>` → `&gt;`.
In short, this means escaping the characters `>`, `<`, `&`, `\n`, and `"` within the string.

Example of the *final* `old_string` value after both JSON and XML escaping: `"&lt;p id=\"hi\"&gt;Say \"Hello\" to Q&amp;A&lt;/p&gt;"`

**✅ CORRECT**:
<cs:tool_use>
    <name>MultiEdit</name>
    <args>
        <arg name="file_path" type="string">/path/to/index.html</arg>
        <arg name="edits" type="array">[{"old_string":"&lt;p id=\"hi\"&gt;Say \"Hello\" to Q&amp;A&lt;/p&gt;","new_string":"&lt;div class=\"hello\"&gt;Say \"Hello\" to Questions &amp; Answers&lt;/div&gt;"}]</arg>
    </args>
</cs:tool_use>

### Example 5: Editing Code with Complex Syntax (e.g., Generics)
This is a common failure point. When creating an `old_string` for code, you must copy the text **exactly as it appears**, including complex syntax like TypeScript generics. Do not simplify the code.

*Code in file `user.ts`.*
`const userMap: Map<string, User> = new Map();`

**❌ WRONG** (Code is simplified, this will cause a 'String Not Found' error):
<cs:tool_use>
    <name>Edit</name>
    <args>
        <arg name="file_path" type="string">/path/to/user.ts</arg>
        <arg name="old_string" type="string">const userMap: Map = new Map();</arg> <!-- WRONG: The generic type <string, User> was removed. -->
        <arg name="new_string" type="string">const allUsers: Map&lt;string, User&gt; = new Map();</arg>
    </args>
</cs:tool_use>

**✅ CORRECT** (Code is copied exactly, with only XML characters escaped):
<cs:tool_use>
    <name>Edit</name>
    <args>
        <arg name="file_path" type="string">/path/to/user.ts</arg>
        <arg name="old_string" type="string">const userMap: Map&lt;string, User&gt; = new Map();</arg> <!-- CORRECT: The string is an exact copy. -->
        <arg name="new_string" type="string">const allUsers: Map&lt;string, User&gt; = new Map();</arg>
    </args>
</cs:tool_use>

## TOOL RESULT FORMAT
The system will provide tool results in a `<cs:tool_result>` tag. **You MUST NOT generate this tag yourself.** Use the information inside it to continue your work.

Example of a result **the system sends to you**:
<cs:tool_result id="tool_id_123">
This is the text output from the tool.
</cs:tool_result>

## TROUBLESHOOTING FAILED TOOL CALLS
If a tool call fails, check for these common errors before retrying:
1. Missing `type` Attribute: Did you set the `type` for every single `<arg>`? (e.g., `limit` in `Read` must have `type="number"`).
2. XML Escaping: Did you escape `&`, `<`, `>` in ALL argument values? For `type="string"`, this means escaping HTML tags (e.g., `<p>` becomes `&lt;p&gt;`).
3. JSON Validity: If `type="array"` or `type="object"`, is the content a valid JSON string?
4. Double Escaping: For `type="array"` with complex content (like HTML), did you perform both inner JSON escaping (`\"`) and outer XML escaping (`&amp;`)?
5. 'String Not Found' Errors: If a tool like `Edit` fails because the string was not found, it is ALMOST always because your `old_string` was not a CHARACTER-FOR-CHARACTER match of the file's content. Re-read the file and construct the `old_string` again, paying extreme attention to whitespace, symbols, and complex syntax like generics (e.g., `<string, any>`).

<cs:Remember>
- CRITICAL: Always set the `type` attribute for every argument.
- CRITICAL: Format argument values correctly.
  - For `type="string"`, the content is literal, but you MUST escape XML characters (`<` → `&lt;`, `>` → `&gt;`, `&` → `&amp;`).
  - For `type="array"` or `type="object"`, the content is a JSON string, which requires its own escaping (`\"`, `\n`), and then the whole string must ALSO be XML-escaped.
- CRITICAL: The ONLY correct way to call a tool is with a complete `<cs:tool_use></cs:tool_use>` block. Ensure the closing tag `</cs:tool_use>` is always present at the very end. Incomplete tags will cause a failure.
</cs:Remember>
</cs:tool-use-guide>"###;
