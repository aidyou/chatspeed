# Workflow Automation Executor Development Plan

## Background

The automation executor is a workflow submodule. Scheduled jobs must create and run normal workflow sessions through the existing workflow runtime, not through an independent execution path.

Relevant constraints:

- Follow `src-tauri/src/workflow/react/CONSTITUTION.md`.
- Backend runtime authority remains `WorkflowManager`, structured workflow state, snapshots, and events.
- Frontend workflow UI may manage form state, but execution state and run lifecycle must come from backend structures.
- User-facing text must use i18n.
- Keep changes targeted and avoid unrelated workflow runtime refactors.

## Goals

1. Add an automation entry button in `src/views/Workflow.vue`, using `cs name="clock"`. During implementation, first locate the actual fold/collapse panel button in the current titlebar layout and insert the clock button immediately to its right. If the current layout has no matching right-side collapse control, place it at the start of the titlebar right control group so it remains in the requested upper-right area.
2. Add an `Automation` section in the workflow sidebar immediately after the `Sessions` section, and place all automation tasks under this section.
3. Add an automation management/create UI under the workflow module.
4. Persist automation definitions and execution history.
5. Add a backend scheduler that triggers due automation jobs and launches workflow sessions via `workflow_start`-equivalent shared logic.
6. Support these create form fields:
   - Task title.
   - Prompt text or selected prompt file.
   - Agent/model selection, matching the workflow input area behavior.
   - Authorized directories.
   - Frequency:
     - Daily: hour/minute, optional weekdays, optional effective start/end dates.
     - Interval: every X hours, optional weekdays, optional effective start/end dates.
     - Once: execution date and execution time.
   - Self-review: when enabled, run final audit after task completion.

## Non-Goals For First Implementation

- No cron expression editor.
- No cloud synchronization of automation tasks.
- No external OS scheduler integration.
- No multi-step visual workflow designer.
- No bypass of the existing workflow approval/waiting model.

## Architecture

### Authoritative State

- Automation definitions: new SQLite table owned by backend DB layer.
- Automation run lifecycle: backend scheduler state plus workflow session state.
- Actual workflow execution: existing `WorkflowManager`, `workflow_snapshots`, `workflow_events`, and `workflow_messages`.
- Frontend list/detail state: read-only projection from backend automation APIs.

The scheduler may remember in-memory timers for efficiency, but the database remains the durable authority. On app startup, the scheduler reloads enabled automation definitions and recomputes next runs.

### Backend Module Layout

Add a dedicated workflow submodule outside the React runtime kernel:

- `src-tauri/src/workflow/automation/mod.rs`
- `src-tauri/src/workflow/automation/types.rs`
- `src-tauri/src/workflow/automation/scheduler.rs`
- `src-tauri/src/workflow/automation/service.rs`

Reason: automation belongs to workflow as an orchestration submodule, but should not add scheduler concepts directly into `src-tauri/src/workflow/react/*`.

### Command Layer

Add Tauri commands in a focused command file or in `commands/workflow.rs` if the existing command registration style makes that simpler:

- `workflow_automation_list`
- `workflow_automation_get`
- `workflow_automation_create`
- `workflow_automation_update`
- `workflow_automation_delete`
- `workflow_automation_enable`
- `workflow_automation_disable`
- `workflow_automation_run_now`
- `workflow_automation_list_runs`

The commands should validate requests, call the automation service, and not implement scheduling logic inline.

### Execution Path

Refactor the current `workflow_start` internals just enough to expose a reusable backend helper that accepts:

- `session_id`
- `agent_id`
- `initial_prompt`
- `initial_metadata`
- `initial_attached_context`
- `planning_mode`

The automation scheduler will:

1. Create a normal workflow record using the selected agent, model overrides, allowed paths, and final audit setting.
2. Attach metadata identifying the automation job and run id.
3. Launch the session through the same runtime start helper used by `workflow_start`.

This preserves the constitution requirement that `WorkflowManager` remains the lifecycle registry and avoids creating a second runtime path.

## Database Plan

Add migration `src-tauri/src/db/sql/migrations/v6.rs` and register it in `src-tauri/src/db/sql/migrations/manager.rs`. Because a new release has already shipped, all database updates for this feature must be isolated in `v6.rs`; do not edit older migration files to add automation tables.

### `workflow_automations`

Suggested columns:

- `id TEXT PRIMARY KEY`
- `title TEXT NOT NULL`
- `prompt TEXT`
- `prompt_file_path TEXT`
- `agent_id TEXT NOT NULL REFERENCES agents(id)`
- `agent_config TEXT`
- `allowed_paths TEXT NOT NULL DEFAULT '[]'`
- `schedule_kind TEXT NOT NULL`
- `schedule_config TEXT NOT NULL`
- `self_review INTEGER NOT NULL DEFAULT 0`
- `enabled INTEGER NOT NULL DEFAULT 1`
- `next_run_at DATETIME`
- `last_run_at DATETIME`
- `created_at DATETIME DEFAULT CURRENT_TIMESTAMP`
- `updated_at DATETIME DEFAULT CURRENT_TIMESTAMP`

`schedule_kind`: `daily`, `interval`, `once`.

`schedule_config` uses backend snake_case internally and converts to frontend camelCase at the command boundary:

- Daily: `time`, `weekdays`, `start_date`, `end_date`.
- Interval: `interval_hours`, `weekdays`, `start_date`, `end_date`, `anchor_time`.
- Once: `run_at`.

### `workflow_automation_runs`

Suggested columns:

- `id TEXT PRIMARY KEY`
- `automation_id TEXT NOT NULL REFERENCES workflow_automations(id)`
- `workflow_session_id TEXT REFERENCES workflows(id)`
- `status TEXT NOT NULL`
- `scheduled_for DATETIME NOT NULL`
- `started_at DATETIME`
- `finished_at DATETIME`
- `error TEXT`
- `created_at DATETIME DEFAULT CURRENT_TIMESTAMP`
- `updated_at DATETIME DEFAULT CURRENT_TIMESTAMP`

Run status values: `pending`, `running`, `completed`, `failed`, `cancelled`, `skipped`.

## Scheduling Semantics

Use local time for all user-facing schedule inputs, because the existing app UI is desktop/local-first. Store timestamps in a consistent backend format and document conversion behavior.

Rules:

- Disabled automations have no active timer.
- Once automations disable themselves after a successful trigger.
- Effective date range is inclusive.
- Empty weekdays means every day in the effective range.
- Daily schedules trigger once per matching local date at the selected hour/minute.
- Interval schedules trigger every `interval_hours` from the last successful scheduled fire or `anchor_time`; when outside weekday/date constraints, skip forward to the next valid window.
- If the app was closed during a due time, on startup run the most recent missed due occurrence unless the schedule is outside its effective range. Do not replay every missed interval by default.
- Prevent overlapping runs for the same automation in the first version. If a previous run is still active, mark the new due occurrence as `skipped` with a structured reason.

## Frontend Plan

### Entry Point

In `src/views/Workflow.vue` titlebar right slot:

- Add an icon button with `cs name="clock"`.
- Place it immediately to the right of the actual fold/collapse panel button when that button exists in the upper-right control group; otherwise place it before the existing approval/sound/action controls.
- Tooltip via i18n, e.g. `workflow.automation.title`.
- Opens the automation panel/dialog.

### Sidebar Placement

In `src/components/workflow/WorkflowSidebar.vue`:

- Add an `Automation` section immediately after the existing `Sessions` section.
- List all automation tasks under this section, not mixed into normal workflow sessions.
- Selecting an automation item opens the automation detail/edit view instead of switching to a workflow session.
- Automation-triggered workflow runs may still create normal workflow sessions, but the automation task definitions remain managed from the sidebar automation section.

### Components

Add focused components under `src/components/workflow/automation/`:

- `WorkflowAutomationDrawer.vue`
- `WorkflowAutomationForm.vue`
- `WorkflowAutomationList.vue`
- `WorkflowAutomationScheduleEditor.vue`
- `WorkflowAutomationRunHistory.vue`

Prefer a drawer for management plus create/edit dialog or side panel. The create form should be dense and operational, consistent with the workflow UI.

### Form Controls

- Task title: `el-input`.
- Prompt: `el-input type="textarea"` plus file picker using Tauri dialog/read APIs.
- Agent: reuse `AgentSelector`.
- Model: reuse `WorkflowModelSelector` or extract a smaller selector wrapper if necessary.
- Authorized directories: reuse existing workflow path selection patterns from `useWorkflowPaths`/sidebar tree where feasible; otherwise use Tauri folder picker first.
- Frequency: `el-segmented` or Element Plus radio button group for daily/interval/once.
- Time/date: Element Plus date/time pickers.
- Weekdays: seven toggle buttons, using locale labels.
- Self-review: checkbox mapped to final audit behavior.

### Frontend Store/Composable

Add `src/stores/workflowAutomation.js` or `src/composables/workflow/useWorkflowAutomation.ts`:

- Load list.
- Create/update/delete.
- Enable/disable.
- Run now.
- Load run history.

Keep scheduler state backend-authoritative. The store should not calculate due jobs except for display.

## i18n Plan

Add sorted locale keys in every frontend locale file under `src/i18n/locales/`.

Initial key group:

- `workflow.automation.title`
- `workflow.automation.create`
- `workflow.automation.edit`
- `workflow.automation.taskTitle`
- `workflow.automation.prompt`
- `workflow.automation.promptFile`
- `workflow.automation.agent`
- `workflow.automation.model`
- `workflow.automation.allowedDirectories`
- `workflow.automation.frequency`
- `workflow.automation.daily`
- `workflow.automation.interval`
- `workflow.automation.once`
- `workflow.automation.executionTime`
- `workflow.automation.executionDate`
- `workflow.automation.weekdays`
- `workflow.automation.effectiveRange`
- `workflow.automation.startDate`
- `workflow.automation.endDate`
- `workflow.automation.intervalHours`
- `workflow.automation.selfReview`
- `workflow.automation.runNow`
- `workflow.automation.enable`
- `workflow.automation.disable`
- `workflow.automation.history`

## Implementation Phases

### Phase 1: Data Model And Service Skeleton

1. Add Rust automation types.
2. Add DB migration and store methods.
3. Add command DTOs with explicit snake_case/camelCase boundary handling.
4. Register commands.
5. Add unit tests for schedule parsing and next-run calculation.

### Phase 2: Scheduler

1. Add app-managed scheduler state in Tauri setup.
2. Load enabled automations at startup.
3. Compute `next_run_at` after create/update/run.
4. Trigger due automations.
5. Record run rows and structured errors.
6. Add overlap prevention.

### Phase 3: Runtime Integration

1. Extract shared workflow start helper from `workflow_start`.
2. Add automation run metadata:
   - `automation_id`
   - `automation_run_id`
   - `scheduled_for`
3. Ensure final audit maps to existing workflow `finalAudit` config.
4. Ensure allowed paths and model overrides are stored in workflow `agent_config`.
5. Verify all execution still appears as normal workflow sessions in the workflow list.

### Phase 4: UI

1. Add clock titlebar button.
2. Add automation drawer/list.
3. Add create/edit form.
4. Add schedule editor for daily/interval/once.
5. Add run history and enable/disable/run-now actions.
6. Add i18n keys for all user-facing text.

### Phase 5: Validation And Polish

1. Run frontend build/type checks if available.
2. Run Rust checks/tests for workflow and DB migration.
3. Manually validate:
   - Create daily schedule.
   - Create interval schedule.
   - Create once schedule.
   - Prompt from text.
   - Prompt from file.
   - Agent/model override.
   - Allowed directory propagation.
   - Self-review enabled.
   - App restart reloads scheduler.
   - Overlap skip behavior.
4. Verify no hardcoded user-facing strings remain.

## Constitution Review Questions

1. Single authoritative state source:
   - Automation definitions and run records: backend DB.
   - Workflow execution lifecycle: `WorkflowManager` and structured workflow runtime state.
2. Parallel path risk:
   - The scheduler must call a shared workflow start helper. It must not execute tools or mutate runtime state directly.
3. Recovery:
   - Workflow recovery remains snapshot-first and event replay fallback. Automation recovery reloads definitions/runs from DB and recomputes future due times.
4. Frontend structure:
   - Frontend consumes command DTOs and workflow state. It does not infer automation state from transcript text.
5. Compatibility debt:
   - No legacy payload parsing should be added. Use one new structured automation schema.
6. Protected invariant:
   - Backend authority and single canonical runtime path are preserved.

## Open Questions Before Coding

1. Should prompt file content be copied into each run at trigger time, or should the file path remain authoritative and be read when the schedule fires?
2. Should interval schedules count from last scheduled time or last completion time?
3. Should missed runs after app restart execute immediately or only update `next_run_at`?
4. Should automation-created sessions be grouped/filtered separately in the workflow sidebar?

Recommended defaults for first implementation:

- Read prompt file at trigger time and store the resolved prompt in run metadata.
- Count intervals from last scheduled fire time.
- Execute only the most recent missed due occurrence on startup.
- Show automation sessions in the normal workflow list, with metadata for future filtering.
