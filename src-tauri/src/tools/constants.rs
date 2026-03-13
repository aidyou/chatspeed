/// Core Tool Name Constants (Single Source of Truth)

// These tools usually require review
pub const TOOL_BASH: &str = "bash";
pub const TOOL_READ_FILE: &str = "read_file";
pub const TOOL_WRITE_FILE: &str = "write_file";
pub const TOOL_EDIT_FILE: &str = "edit_file";
pub const TOOL_LIST_DIR: &str = "list_dir";
pub const TOOL_GLOB: &str = "glob";
pub const TOOL_GREP: &str = "grep";
pub const TOOL_WEB_SEARCH: &str = "web_search";
pub const TOOL_WEB_FETCH: &str = "web_fetch";

// These tools are internal tools for the agent, usually do not require review
pub const TOOL_TASK: &str = "task";
pub const TOOL_TASK_OUTPUT: &str = "task_output";
pub const TOOL_TASK_STOP: &str = "task_stop";

// todo tools
pub const TOOL_TODO_CREATE: &str = "todo_create";
pub const TOOL_TODO_LIST: &str = "todo_list";
pub const TOOL_TODO_UPDATE: &str = "todo_update";
pub const TOOL_TODO_GET: &str = "todo_get";

// skill tools
pub const TOOL_SKILL: &str = "skill";

pub const TOOL_ASK_USER: &str = "ask_user";
pub const TOOL_FINISH_TASK: &str = "finish_task";
pub const TOOL_SUBMIT_PLAN: &str = "submit_plan";

pub const MCP_TOOL_NAME_SPLIT: &str = "__MCP__";

use phf::{phf_set, Set};

/// Read-only bash commands that require exact match (no arguments expected)
/// Uses perfect hash function for O(1) lookup performance
pub static READ_ONLY_BASH_CMDS_EXACT: Set<&'static str> = phf_set! {
    // File system listing and navigation
    "ls",
    "pwd",
    "dir",
    "tree",
    "date",
    "whoami",
    "id",
    "groups",
    "hostname",
    "uptime",
    "uname",
    "arch",
    "env",
    "printenv",
    "ps",
    "lsattr",
    "df -h",
    "free -m",
    // Git commands
    "git status",
    "git log",
    "git diff",
    "git show",
    "git branch",
    "git tag",
    "git remote",
    "git remote -v",
    "git config --list",
    "git stash list",
    "git rev-parse",
    "git ls-files",
    "git ls-tree",
};

/// Read-only bash command prefixes that accept arguments
/// Commands with trailing space prevent false matches (e.g., "cat " won't match "catch")
pub const READ_ONLY_BASH_PREFIXES: &[&str] = &[
    // File content reading
    "cat ",
    "head ",
    "tail ",
    "less ",
    "more ",
    "hexdump ",
    "type ", // Windows equivalent of cat
    // File metadata and properties
    "stat ",
    "file ",
    "wc ", // word/line count
    "du ", // disk usage
    // Binary/Library analysis
    "nm ",
    "ldd ",
    "readelf ",
    "objdump ",
    // Search and locate
    "grep ",
    "egrep ",
    "fgrep ",
    "find ",
    "locate ",
    "which ",
    "whereis ",
    "where ", // Windows command location
    "history ",
    // Process information
    "pgrep ",
    // Git config
    "git config --get",
    "git remote ",
    "git branch ",
    "git show ",
    "git log ",
    "git diff ",
    // Package manager queries
    "npm list",
    "npm ls",
    "npm --version",
    "npx --version",
    "yarn --version",
    "yarn list",
    "yarn info",
    "pnpm --version",
    "pnpm list",
    "pnpm ls",
    "cnpm --version",
    "cnpm list",
    "cnpm ls",
    "bower list",
    "cargo --version",
    "cargo check",
    "cargo test --no-run",
    "cargo --list",
    "cargo tree",
    "rustc --version",
    "rustup --version",
    "node --version",
    "python --version",
    "python3 --version",
    "pip --version",
    "pip list",
    "pip show",
    "pip freeze",
    "pip3 --version",
    "pip3 list",
    "pip3 show",
    "pipenv --version",
    "poetry --version",
    "go version",
    "go env",
    "go list",
    "java -version",
    "javac -version",
    "mvn --version",
    "gradle --version",
    "ruby --version",
    "gem --version",
    "gem list",
    "bundler --version",
    "php --version",
    "composer --version",
    "composer show",
    "dotnet --version",
    "dotnet --list-sdks",
    "dotnet --list-runtimes",
    "swift --version",
    "xcodebuild -version",
    "flutter --version",
    "dart --version",
    "perl --version",
    "lua -v",
    "ghc --version",
    "stack --version",
    // Network diagnostics
    "ping ",
    "traceroute ",
    "nslookup ",
    "dig ",
    // Echo
    "echo ",
];
