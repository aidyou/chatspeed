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

## 5. UI Requirements (`Workflow.vue`)
- **Mode Selection**: New toggle switch for Mode selection (Autonomous vs. Planning).
- **Final Audit Switch**: A new toggle to enable/disable the "Self-Reflection Audit" before the task concludes. This allows users to skip the audit for trivial tasks.
- **Dynamic Controls**: Both switches are editable before the workflow starts and disabled once a session is active.
- **Plan Review UI**: Specialized "Plan Review" component for the approval stage.
