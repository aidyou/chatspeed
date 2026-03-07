# Planning Mode Implementation Design (Final)

## 1. Objective
Introduce a "Planning Mode" to ChatSpeed. In this mode, the AI first conducts research and exploration to create a detailed implementation plan. The actual execution (making changes) only begins after the user reviews and approves the plan.

## 2. Architecture: Dual Executor Model
To keep the logic clean and maintainable, we will split the ReAct engine into two specialized executors:

### A. PlanningExecutor (`src-tauri/src/workflow/react/planners.rs`)
- **Focus**: Research, exploration, and strategy.
- **Tool Access**: Restricted to READ-ONLY tools for the main workspace, but ALLOWS WRITE access to designated sandbox directories (`planning/`, `tmp/`, `skills/`).
- **Goal**: Understand the user request, explore the environment, and produce a structured plan.
- **Output**: A Markdown plan and a structural `todo_list`.

### B. ExecutionExecutor (`src-tauri/src/workflow/react/runners.rs`)
- **Focus**: Implementation and task completion.
- **Tool Access**: Full access to all authorized directories.
- **Input**: Original user prompt + Approved Plan + Initial Todo List.
- **Goal**: Execute the steps defined in the plan accurately.

## 3. Workflow & Prompting
1. **Selection**: User toggles "Planning Mode" in the UI.
2. **Phase 1: Exploration (PlanningExecutor)**: AI performs research. It can write draft notes to `planning/`.
3. **Phase 2: Approval**: Session enters `AwaitingApproval` state.
4. **Phase 3: Execution Transition (Context Pruning)**:
   - Upon approval, detailed research messages are pruned.
   - The context starts fresh with: Original Query + Final Approved Plan + Current Todo List.

## 4. Tool Management & Safety Rules

### A. Recursive Task Creation (STRICTLY PROHIBITED)
- **Sub-agent Isolation**: Sub-agents **MUST NOT** have access to the `task` (sub-agent creation) tool. This is a critical security measure to prevent infinite recursion and resource exhaustion.

### B. Path Guard & Directory Security
- **Prefix Matching**: Path validation MUST use absolute, canonicalized path prefixes. Matching by directory name is strictly forbidden as it creates a security vulnerability.
- **Phase-based Restriction**:
    - **Planning Phase**: Write access is strictly limited to canonicalized sandbox roots.
    - **Execution Phase**: Access is granted to all user-authorized canonicalized roots.

## 6. Advanced Capabilities & Future Integration

### A. Long-term Memory Integration (Placeholder)
- Implement a persistent memory store (vector or KV) for cross-session knowledge.

### B. Workspace Documentation: `AGENTS.md` Support (Placeholder)
- If an `AGENTS.md` file exists in the root of the authorized workspace, the Agent MUST read it during initialization.
