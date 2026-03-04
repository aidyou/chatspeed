# Planning Mode Implementation Design

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

## 3. Workflow
1. **Selection**: User toggles "Planning Mode" in the UI before starting a workflow.
2. **Phase 1: Exploration (PlanningExecutor)**:
   - AI uses a specialized "Planning Prompt" (adapted from Claude Code).
   - AI performs research to understand the boundaries and requirements.
   - AI calls a final tool `submit_plan` (or similar) to present the findings.
3. **Phase 2: Approval**:
   - The workflow enters a `Paused` state.
   - UI displays the Markdown plan and the generated Todo List.
   - User can "Approve" (start Execution) or "Provide Feedback" (re-plan).
4. **Phase 3: Execution (ExecutionExecutor)**:
   - Upon approval, the `ExecutionExecutor` is instantiated.
   - It follows the plan until `finish_task` is called.

## 4. Generalized Planning Prompt
The prompt will be adapted from the Claude Code version to handle both technical and general tasks:
- **Phase 1: Initial Understanding**: Explore context and ask clarifying questions.
- **Phase 2: Design**: Create a step-by-step approach.
- **Phase 3: Review**: Ensure alignment with user intent.
- **Phase 4: Final Plan**: Write a concise, executable plan.
- **Phase 5: Request Approval**: Use the approval mechanism to pause and wait for the user.

## 5. UI Requirements (`Workflow.vue`)
- New toggle switch for Mode selection.
- Disabled toggle once a session starts.
- Specialized "Plan Review" component for the approval stage.
