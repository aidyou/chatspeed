# Planning Mode Implementation Design (Final)

## 1. Objective
Introduce a "Planning Mode" to ChatSpeed. In this mode, the AI first conducts research and exploration to create a detailed implementation plan. The actual execution (making changes) only begins after the user reviews and approves the plan.

## 2. Architecture: Dual Executor Model
To keep the logic clean and maintainable, we will split the ReAct engine into two specialized executors:

### A. PlanningExecutor (`src-tauri/src/workflow/react/planners.rs`)
- **Focus**: Research, exploration, and strategy.
- **Tool Access**: Restricted to READ-ONLY tools (`read_file`, `grep`, `glob`, `web_search`, `web_fetch`, `list_dir`).
- **Goal**: Understand the user request, explore the environment (codebase or web), and produce a structured plan.
- **Output**: A Markdown plan and a structural `todo_list`.

### B. ExecutionExecutor (`src-tauri/src/workflow/react/runners.rs`)
- **Focus**: Implementation and task completion.
- **Tool Access**: Full access (`write_file`, `edit_file`, `bash`, etc.).
- **Input**: Original user prompt + Approved Plan + Initial Todo List.
- **Goal**: Execute the steps defined in the plan accurately.

## 3. Workflow & Prompting
1. **Selection**: User toggles "Planning Mode" in the UI before starting a workflow.
2. **System Prompt**: Both modes use the same standard `CORE_SYSTEM_PROMPT` + `Agent.system_prompt`.
3. **User Message Construction**: In Planning Mode, the initial user message is constructed as a combined string: 
   `{PLANNING_MODE_PROMPT}\n\n{user_question}`.
4. **Phase 1: Exploration (PlanningExecutor)**:
   - AI performs research and calls `submit_plan` to present findings.
5. **Phase 2: Approval & Recovery**:
   - The session enters `AwaitingApproval` state (stored in DB).
   - Both phases share the same `session_id`.
   - If the app is restarted, the session remains in `AwaitingApproval` to ensure recovery.
6. **Phase 3: Execution Transition (Context Pruning)**:
   - Upon approval, the `ExecutionExecutor` is instantiated.
   - **Context Pruning**: The detailed research messages (thoughts/tool calls) from the planning phase are **pruned**.
   - The execution context starts fresh with:
     - The `CORE_SYSTEM_PROMPT`.
     - The original user query.
     - The **Final Approved Plan** (as high-priority context).
     - The current `todo_list`.

## 4. Tool Management & Safety Rules
### A. Built-in Mandatory Tools
The following tools are always allowed: `ask_user`, `finish_task`, and `todo_*`.
### B. Recursion Protection
Sub-agents **MUST NOT** have access to the `task` tool to prevent infinite recursion.

## 6. Advanced Capabilities & Future Integration

### A. Long-term Memory Integration
- **Mechanism**: Implement a persistent memory store (vector or KV) to allow Agents to remember facts, user preferences, and historical decisions across different sessions.
- **Context Injection**: Relevant memories are retrieved and injected into the system prompt or as a separate `<memory>` block.

### B. Workspace Documentation: `AGENTS.md` Support
- **Protocol**: If an `AGENTS.md` file exists in the root of the authorized workspace, the Agent MUST read it during the initialization/planning phase.
- **Content**: This file serves as the "team handbook," defining project-specific coding standards, architectural rules, and preferred libraries.

### C. Skills System Verification
- **Testing**: Implement a comprehensive test suite for the Skills system (`src-tauri/src/workflow/react/skills.rs`).
- **Dynamic Loading**: Ensure skills defined in YAML/Markdown are correctly scanned, parsed, and registered as available tools or sub-agents.
