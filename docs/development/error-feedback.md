# Error feedback coverage and verification ledger

## Scope and status

Implementation and parent review are recorded in **Current reconciled coverage
after implementation** below. Production bundling and live desktop acceptance
are still unverified and handed off to the user for local completion; this
document does not sign off the entire plan. The user accepted this partial
verification boundary. The initial rows retain the U-1 characterization baseline
and must not be read as the current implementation status.
No production behavior, dependency, schema, or DAG changes are made by U-1.
Paths below are relative to the repository root. `B` abbreviates
`src-tauri/crates/backend/src`, `D` abbreviates `src-tauri/src`, and `H`
abbreviates `B/workflow/react/client/http`.

Status means:

- **Pending**: an active first-party path whose source/carrier/presentation still
  needs explicit migration review. Inventory does not mean implementation complete.
- **Protected**: existing wire, content, visibility, redaction, or exit-code
  behavior that must survive later changes. Protected does not mean every
  construction site has been exhaustively audited or tested.
- **Excluded**: outside the active migration; do not count it as completed work.

Each row describes source → carrier → wire → UI/client → log. “Not established”
means this baseline does not prove a dedicated log or end-to-end test exists.
A `Display` string, serialized domain enum, HTTP envelope, and model-visible tool
result are different contracts; do not replace them with one global formatter.

## Active source → carrier → wire → UI → log ledger

| Group / status | Source and actual boundary anchors | Carrier and wire | UI/client and log boundary / remaining review |
| --- | --- | --- | --- |
| Desktop union — **Pending**, legacy shape **Protected** | `D/error.rs::AppError`, native commands under `D/commands`; desktop-only `Http`, `Updater`, Tauri/Wry conversions remain desktop-owned | Direct `Serialize` is `{module,details}`. `From<AppError> for String` separately serializes the enum and inserts a top-level `message`; it is not automatically invoked when Tauri serializes a command's error | `src/libs/tauri.js::invokeWrapper` accepts objects and JSON strings. Direct and String-returning command signatures must be audited independently; no universal logging is provided by the union |
| Backend union — **Pending**, legacy shape **Protected** | `B/error.rs::AppError` wraps AI/DB/tool/workflow/CCProxy/MCP/sensitive/general failures | Same module/details tagging and separate String conversion, without desktop-only variants. `General {message}` serializes details as an object. Nested AI errors can have an object-valued `message` | Do not assume nested `details.message` is readable text. Backend `AppError` is not the control-plane `ErrorEnvelope`; log ownership stays with construction/transport sites |
| AI HTTP — **Pending**, raw body behavior **Protected** | `B/ai/chat/openai.rs::api_request_error` constructs `RawApiRequestFailed` from HTTP status/provider/body; OpenAI Responses has separate terminal-failure construction in `openai_responses.rs` | `B/ai/error.rs`: `{kind,message}` with camelCase variants; `ApiRequestFailed` and `RawApiRequestFailed` retain status/provider/details. Raw Display returns details; localized Display is different | Chat callbacks and ReAct consume these errors through different paths. Preserve provider detail while reviewing safe human feedback; OpenAI/Responses have local error logging, not one union-level log |
| AI SSE / Responses terminal events — **Pending**, stream semantics **Protected** | `B/ai/chat/openai.rs` constructs `StreamProcessingFailed` for chunk/read/processing failures; `openai_responses.rs` handles failed/incomplete/error outcomes and emits error callbacks | Error `ChatResponse` content/callback and returned `AiError` are separate carriers; an established stream cannot be replaced with an ordinary HTTP failure | `src/libs/chat.js`, `src/stores/chat.js`, desktop `D/runtime_chat.rs::error_response` and `stream_error_message` are presentation/transport boundaries. Review duplicates and stream endings; preserve callback ordering and log context |
| AI model list — **Pending** | `B/ai/chat/list_models.rs` constructs `ApiRequestFailed` for OpenAI, Claude, Gemini network/status failures | `AppError`/`AiError` result, not a CCProxy protocol error | Model settings and runtime model catalog adapters consume results. Audit status fallback and local logging separately from chat |
| CCProxy direct chat — **Pending**, pass-through and headers **Protected** | `B/ccproxy/handler/direct_handler.rs`, selected by same protocol + compat off; `helper/retry.rs::send_with_retry` constructs `BackendRequestError` for send failures | Upstream response bytes/status pass through; transport failure uses `CCProxyError::IntoResponse`, not the adapted four-protocol formatter. `utils/http::filter_proxy_headers` is mandatory | External clients, not a dedicated app toast. Retry logs attempt/top-level error/source chain; handler stats/logs remain required. Do not normalize upstream body on the direct path merely for consistency |
| CCProxy adapted chat — **Pending**, four protocol shapes **Protected** | `handler/chat_handler.rs` catches `BackendRequestError` and constructs `UnifiedErrorResponse` with 502; non-2xx upstream bodies use `adapter/error.rs::normalize_backend_error` | `adapter/output/adapter_enum.rs::adapt_error_response` dispatches to `error_response.rs`: OpenAI, Claude, Gemini, Ollama. Upstream type is not blindly reused as client error type | External clients receive protocol-safe JSON; request id/code/message and status mappings differ by protocol. Filter upstream headers after adaptation; retry/helper/handler/stat logging must remain intact |
| CCProxy Responses — **Pending**, route/output distinction **Protected** | `handler/responses_handler.rs`; `OutputAdapterEnum::OpenAIResponses` shares OpenAI error formatting | Do not infer that every Responses branch uses the adapted chat error path; responses/direct forwarding and stream conversion require separate review | External Responses consumers; response headers are filtered. Route-specific stream/error stats and transport failures need focused coverage before migration |
| CCProxy embedding — **Pending**, unsupported capability **Protected** | `handler/embedding_handler.rs::get_proxy_alias_from_body` / `build_unified_request` construct parse/protocol errors, including explicit Claude/Decision unsupported errors | `CCProxyError` plus unified embedding adapters, not automatically `UnifiedErrorResponse` for every failure | External embedding clients; preserve authentication, resolver authority, retry, stats, and header safety. Four chat output tests do not prove embedding coverage |
| CCProxy model list — **Pending** | `handler/list_models_handler.rs` constructs `ModelAliasNotFound` for missing groups and returns locally projected catalog JSON | `ProxyResult<Json<Value>>` → `CCProxyError::IntoResponse`; not AI provider model-list HTTP parsing | External list clients; group-not-found warnings and CCProxy error logs. Do not conflate this with `B/ai/chat/list_models.rs` |
| CCProxy decision / extra endpoints — **Pending** | `handler/decision_handler.rs` maps `DecisionError::Catalog` to `InvalidProtocolError`, other failures to `BackendRequestError`; `ollama_extra_handler.rs` is another endpoint family | Decision response bytes/status use filtered headers; domain failures use CCProxy error conversion | External protocol clients; route-specific behavior remains pending. Preserve fixed/direct/compat/group routing order and existing decision execution authority |
| ReAct fatal / retry exhaustion — **Pending**, terminal content and metadata **Protected** | `B/workflow/react/error.rs::WorkflowEngineError::terminal_error`, `engine.rs::publish_terminal_error` and run-loop error handling | Runtime enum is `{type,details}`. Terminal projection has content/error_type/metadata. Publisher persists assistant/observe, `is_error=true`, and typed `TerminalError`; terminal fields are copied into both observation data and top-level metadata | Durable transcript/event → workflow UI. `TerminalError` is UI show / LLM hide. Run-loop and message-publish failure logs include session/phase/event. Raw upstream body and fallback generic reminder remain legacy content, not a new error schema |
| ReAct fatal/manual compression — **Pending**, fatal precondition **Protected** | `goal_tracker.rs` and `engine.rs` construct `CompressionFailed`; `engine.rs::run_terminal_manual_compression` is a distinct stopped-workflow operation | Fatal compression has dedicated content/type; no-safe-boundary manual compression may return unavailable without appending transcript or changing terminal lifecycle | UI compression progress/result and command errors are distinct from fatal terminal observations. Preserve tracked-goal precondition and stopped-state checks; log-only best-effort failures must not become fatal by accident |
| ReAct tool observations — **Pending**, LLM payload/pairing **Protected** | Tool errors and reinforced results in `engine.rs`, `tools` and `runtime_observation.rs::render_runtime_observation_for_llm` | Tool-call-id paired observations remain ordered/preserved even when generic visibility says defer/hide. `data.llm_content` may intentionally differ from UI message | UI tool ledger and AI function-call result are not interchangeable. Do not wrap tool content with user-observation tags; preserve original tool arguments/results and diagnostic context |
| ReAct subagent / history / recovery — **Pending**, projection **Protected** | `engine.rs` constructs `SubAgentCompletion` observations; `runtime_observation.rs` retains typed/legacy subagent and interruption compatibility; context/history projection lives under `context*` | Typed completion is preserve/card; interruption preserve/hide; explicit hide remains hide. Legacy user observe + `<SYSTEM_REMINDER>` is deferred, legacy `<tool_result>` completion preserves position | Durable `workflow_messages` is history authority; `workflow_context_messages` is rebuildable cache. UI history/projection must not reconstruct lifecycle from error text. Recovery/projection end-to-end remains pending |
| Control HTTP — **Pending**, old envelope/status **Protected** | `H/dto.rs::error_response` directly constructs `ErrorEnvelope {error: ErrorDetail {code,message}}`; application/auth/chat/data/workflow handlers call it | HTTP status plus `{error:{code,message}}`; application codes include invalid_input/not_found/conflict/invalid_state/gateway_unavailable/internal_error/runtime_stopping | `src-tauri/crates/client/src/lib.rs::error_from_status` parses envelope, else raw body or reason fallback. Auth 401/403 becomes `ClientError::Auth`; other failures retain status/code/message. Do not claim every rejection goes through one formatter without auditing extractors |
| Shared client and desktop runtime bridges — **Pending**, availability/no-fallback **Protected** | `src-tauri/crates/client/src/lib.rs::ClientError`; `D/runtime_client.rs::RuntimeUnavailable`; `D/runtime_workflow.rs`, `runtime_chat.rs`, `runtime_data.rs`, `runtime_agent.rs`, `runtime_config.rs` | ClientError Server retains code; Auth folds message/code into text. Desktop bridges often return `String` after mapping, so domain identity may be lost before Tauri. Canonical HTTP snake_case and historical Tauri camelCase remain separate | Frontend invoke/callback/store boundary; supervisor logs and stream logs differ from command errors. No local runtime fallback and no second DB/workflow owner |
| Capability — **Pending**, stable code/redaction **Protected** | `B/capability/error.rs`; `H/dto.rs::capability_error_response`; `D/runtime_capability.rs::map_client_error` | HTTP uses old envelope with `redacted_message`; desktop capability carrier has its own code/message contract and preserves server codes | `src/libs/capability.js::parseCapabilityError`, capability store / Agent Skills UI branch on code, not translated text. Keep redaction; diagnostic log coverage requires operation-level review |
| Plugin — **Pending**, domain contract **Protected** | `B/plugin_types.rs`, `B/plugin.rs`, `D/runtime_plugin.rs::map_client_error`, HTTP plugin handlers | PluginError and shared client mapping are distinct from AppError; server code/message and unavailable classifications must survive | Plugin store and settings consumers; audit redaction and local failures without importing plugin lifecycle into desktop |
| Terminal capability — **Pending**, session/transport **Protected** | `B/terminal.rs`; `H/terminal_commands.rs::terminal_error_response`; `D/runtime_terminal.rs::client_message` | Stable HTTP envelope/status for session/gate errors; terminal output/exit streams are not workflow fatal observations. Desktop may flatten client errors to String | `src/composables/workflow/useTerminal.ts`, `TerminalPanel.vue`; logs/PTY process ownership remain separate from UI display |
| Automation — **Pending**, code/privacy **Protected** | `B/workflow/automation/errors.rs`, `H/dto.rs::automation_error_response`, `D/runtime_automation.rs` | Stable facade codes → HTTP status/envelope; human messages must not contain raw shell output, secrets, or full prompts | Workflow automation editor/store and CLI automation use the same codes; operation log review remains pending |
| Data / store / sensitive — **Pending**, domain payload **Protected** | `B/data`, `B/data_commands.rs`, `B/db/error.rs`, `B/sensitive/error.rs`; `H/data_commands.rs` maps ApplicationError | Domain enum/AppError and application HTTP envelope are separate; DB errors can retain underlying diagnostic text | Desktop runtime_data / stores/settings consume responses. Audit each mapping and notification ownership; no database migration is authorized |
| MCP JSON-RPC / tool error — **Pending**, protocol **Protected** | `B/mcp/error.rs::From<McpError> for ErrorData` constructs INTERNAL_ERROR with Display message; `mcp/client/types.rs` encodes `CallToolResult.is_error`; server/handler preserves tool error content | JSON-RPC ErrorData `{code,message,data?}` is not control HTTP ErrorEnvelope. A successful RPC carrying `CallToolResult.is_error=true` is another distinct error channel | MCP clients, workflow tool results, MCP settings; retain rmcp protocol and model-facing tool semantics. Redaction and transport/source-chain logging need separate audit |
| CLI — **Pending**, exit categories **Protected** | `src-tauri/crates/cli/src/error.rs::CliError`, client mapping/rendering/entrypoint | Server status/code/message or local categories → Display/stderr and exit code; 1 server/I/O, 2 usage, 3 discovery/transport, 4 auth, 5 protocol, 9 budget | Terminal user rather than Vue UI; stderr is not an HTTP envelope. Preserve machine exit codes and structured command output |
| Legacy CLI — **Pending / compatibility-only**, build gate **Protected** | `D/bin/cs.rs` and `D/bin/cs/*`; Cargo `legacy-cli-binary` required feature | Retained source has its own CliError/client mappings; not the normal `crates/cli` binary | Do not count source duplication as another active normal CLI, or silently remove compatibility tests. Further migration/retirement requires explicit scope |
| Daemon startup / lifecycle — **Pending**, process contract **Protected** | `src-tauri/crates/daemon/src/main.rs`, `lib.rs::RuntimeError` and `start_runtime` | Startup errors → `chatspeed-runtime: {error}` stderr and failure ExitCode; active control errors still use HTTP handlers | Supervisor/CLI observes readiness and process failure. Daemon logger emits level/target/message to stderr; do not turn startup failure into a fake ready runtime |
| Native / UI helpers — **Pending** | `D/commands/{fs,clipboard,updater,...}`, `D/window.rs`, `tray.rs`, `shortcut.rs`, updater/scraper/search/native helpers | Mix of desktop AppError, String, callback/event and log-only failures; no universal carrier is established | Native UI and frontend notifications; per-command signatures, localization and log ownership must be inventoried before conversion |
| Frontend Tauri / capability adapters — **Pending**, input compatibility **Protected** | `src/libs/tauri.js`, `src/libs/capability.js` | `FrontendAppError(module,kind,message,originalError)` and capability Error+code are separate. Tauri parser accepts object/string; nested message is currently selected without a string check | Consumers should not stringify structured nested objects as readable diagnostics. Preserve originalError and stable codes; console logging is not a substitute for backend cause logs |
| Frontend settings / common — **Pending** | `src/components/setting/{General,Model,Mcp,Agent,Skill,Proxy,...}.vue`, `components/common/{FileSelector,ApiKeyUnlockDialog}.vue` | Catch sites mix translated fallback, `error.message`, interpolation and direct notification | Element Plus message/notification/dialog. Config-transfer and editor tests are protected baseline evidence only, not proof all settings/common catch sites are migrated |
| Frontend views / stores — **Pending** | `src/views/{Settings,Index,Workflow,AgentSkills,Note,Assistant,ProxySwitcher}.vue`; `src/stores/{workflow,chat,model,mcp,plugin,capability,setting,...}` | Re-throw, state error, toast and console-only paths coexist | Preserve state and notification ownership; audit duplicate toasts and fallback text per operation. No broad frontend refactor is included here |
| Frontend workflow — **Pending**, structured projection **Protected** | `src/composables/workflow/{useErrorBoundary,useWorkflowCore,useWorkflowApproval,useWorkflowCodeEditor,...}`, `src/components/workflow/*` | Structured metadata drives terminal/tool/subagent rendering; command exceptions are not transcript observations | Preserve UI show/hide/card, backend lifecycle/wait/approval authority and durable history paging. MessageProjectionRules/workflowUiContract protect only tested contracts |
| Frontend chat / directive / helpers — **Pending**, AI content **Protected** | `src/libs/{chat,directive,fs,clipboard,sync}.js`, `src/stores/chat.js`, chat components, `src/tool/ic.js` | Stream callback content, invoke rejection, local Error and direct UI helpers are independent sources | Chat error content can be user-visible conversation output. Do not rewrite LLM/tool/directive content through a generic toast formatter; audit per-call logs and notification ownership |

## U-2 foundation evidence

Implemented: optional `UserErrorPresentation`/finite `ErrorReason` in contracts;
legacy envelopes omit `presentation` when absent. `FailureMessage` keeps String
Display/serde and memory-only `ErrorOrigin`; explicit source capture, frames,
safe user projection, and bounded JSON diagnostics are available. The pure
frontend adapter accepts actual HTTP, Tauri, AI and Sensitive shapes, preserves
codes/status and never derives reason from text. New fallback keys exist in all
three frontend/backend locales. No active domain is marked migrated solely
because the shared helper now exists.

Focused verification after foundation changes:

- contracts `presentation`: 2 passed; `error_fixture`: 2 passed.
- backend `error_diagnostics::`: 7 passed (source capture, String serde,
  cycle/frame limits, Unicode/HTML, structured JSON budget and log facade).
- backend `capability::redaction::tests`: 10 passed (including credential
  containers, escaped/multiline JSON, scalar text and bounded deep input).
- Node `error.test.js` + `capability.test.js`: 22 passed.
- `cargo check -p chatspeed`: passed in the available desktop build environment.
- `cargo check -p chatspeed-runtime-client -p chatspeed-cli`: passed.
- client legacy-envelope filter: 1 passed; CLI error exit filters: 3 passed.

The log-facade test is not yet proof of daemon stderr or desktop file-sink
integration; those remain V-8/U-7/U-9 work. Domain source propagation, typed
business-state migration, full display-site migration and desktop rendering
remain pending U-3 through U-9. No full-workspace test has been run.

## U-3 domain and IPC evidence

The desktop/backend AppError serializers now share direct/String JSON output,
retaining legacy module/details/message and adding presentation. AI, DB,
Sensitive, Tool, MCP, desktop HTTP/updater and General typed sources have
memory-only origins and safe projections. Capability/plugin retain their
single code/message payloads and skip in-memory origins in legacy serde.
ApplicationError preserves typed StoreError provenance and its existing kind;
control HTTP attaches the optional projection without changing status/code.
MCP JSON-RPC retains INTERNAL_ERROR and uses the safe user message. Ordinary
model/tool content remains legacy text, not diagnostic JSON.

Verification: backend `error::tests` 31 passed; desktop `error::tests` 4 passed;
capability error tests 2 passed; plugin projection 1 passed; application HTTP
origin 1 passed; data store origin 1 passed; backup filters 17 passed; AI raw
body and network raw-body filters 1 each; typed git-read failure 1 passed;
provider detail bounds/redaction 2 passed and structured provider code 1 passed.
Native invocation/UI rendering and final log responsibility sinks are still
pending the later transport/UI/log units, not implied by serializer tests.

Extended DB checks reported two failures outside changed logic:
`db::config_transfer::tests::all_auto_increment_categories_restore_explicit_ids_and_sequences`
rejects an MCP stdio fixture with no command; a migration ahead-of-head test
expects capability_operations after a deliberately skipped migration. Neither
schema nor assertions were changed to mask these failures. These are recorded
for final verification reconciliation, not treated as passed.

## Explicit exclusions

- **DAG: Excluded.** `B/workflow/mod.rs` contains `// pub mod dag;`; the active
  engine is ReAct. Do not migrate the inactive DAG or use it as evidence of
  active-path coverage.
- **Old workflow event gateway: Excluded.** `D/workflow_event_gateway/*` is retained
  source, not declared by `D/lib.rs`. `B/workflow/react/client/mod.rs` documents
  that former desktop adapter. Active desktop workflow events use the runtime
  supervisor/control plane, not this old gateway.
- **Third-party, generated/build output, scripts, website and documentation
  submodules: Excluded.** External SDK/protocol payloads remain compatibility
  boundaries, not first-party error-carrier migration targets.

## Legacy invariants and known traps

1. Direct enum serialization and `From<AppError> for String` are observably
   different. For example General directly gives
   `{"module":"General","details":{"message":"bad"}}`; conversion adds
   `"message":"bad"`. For an AI struct variant, the inner serialized `message`
   may itself be `{status_code,provider,details}`. A future human-feedback field
   must not assume all legacy `message` values are strings.
2. The old control envelope is exactly
   `{"error":{"code":"invalid_input","message":"bad request"}}`.
   Existing clients must still read that two-field form. Additional optional
   metadata is future work, not part of U-1.
3. Adapted protocol errors are intentionally different: OpenAI uses
   error.message/type/param/code; Claude adds top-level type and optional
   request_id; Gemini uses numeric error.code plus status; Ollama uses a string
   error. OpenAI Responses shares the OpenAI output adapter. Representative
   overload mappings are 529→503 OpenAI, 503→529 Claude, 529→503 Gemini, and
   503→503 Ollama. Do not force them into the control envelope.
4. Retry send diagnostics currently start with exactly one
   `Request to backend failed:` prefix. Source-chain causes are separately
   logged by `log_backend_request_error`; do not silently append duplicate
   prefixes or leak a new cause chain into the client payload.
5. Generic engine terminal content still contains the legacy `Critical Error:`
   and `<SYSTEM_REMINDER>` text. Compression has its own fatal-precondition
   reminder. Raw AI errors retain upstream details, including JSON bodies;
   empty details get an explicit provider/status fallback. U-1 records this
   behavior without endorsing it as an ideal future human presentation.
6. Terminal UI visibility is metadata-driven: typed assistant terminal
   observations are UI-visible and LLM-hidden even while their stored content
   contains a reminder. Legacy user/observe reminders without typed metadata
   are still deferred model-visible compatibility input and receive a
   `<RUNTIME_OBSERVATION type="legacy_reminder">` wrapper. Text alone cannot
   decide terminal semantics.
7. Tool result pairing, `data.llm_content`, subagent results, structured
   completion/retry/approval state, and ordered history are model/runtime
   contracts. Do not change them while improving human-readable feedback.
8. Preserve proxy routing/header filtering/auth/retry/stat obligations and ReAct
   backend authority. U-1 changes tests/docs only; it creates no parallel error
   transport, context state machine, or runtime owner.

## Post-delegation verification and outstanding work

This checkpoint follows a user-requested, per-delegation verification after
multiple implementer failures. It is not system-wide acceptance. All launched
implementers in that batch have returned; no failed handoff is accepted as a
completed unit. Future concurrency is capped at three Code Implementers and five
Code Explorers. During this audit only two read-only explorers were launched;
the parent checked their claims against current source and executed tests.

### Handoff reconciliation

| Delegated scope | Actual state checked by the parent | Outstanding work |
| --- | --- | --- |
| U-4 workflow/editor source and frontend | Source reasons and frontend exact-reason branches exist; editor and UI contract tests pass | `D/runtime_workflow.rs::map_client_error` still returns `response.message`, dropping presentation/reason. U-4 remains incomplete despite isolated passing tests. Data/agent bridges have the same loss. |
| U-4 client/bridge handoff that exited without a report | Typed Auth/Server payloads, old-envelope fixture and positive presentation fixture exist and pass; supervisor tests pass | Inspect and close every String-returning IPC bridge; a successful client parser test is not end-to-end evidence. |
| U-5 CCProxy observation handoff | Only retry request-failure logging was converted. Its source-chain characterization test was initially weakened; parent restored a real chained-error assertion and all five retry tests pass | Direct/adapted endpoint ownership, remaining existing final logs, protocol/stream loopback and redaction coverage remain open. |
| U-5 AI and U-8 ordinary chat tools | Parent removed the failed child edits that inserted `is_error` into model history/result bodies; the only remaining `chat_completion.rs` change at this checkpoint preserves an init-error source. Three frontend renderers now consume `toolResultState.js` | Explicit tool success/failure metadata producer and persistence are missing, so new results can be unknown. Helper has no dedicated regression test. AI source projections are not proof that every SSE callback carries one. |
| U-6 ReAct backend | Child made no implementation edits after mistaking this task's existing foundation changes for an unresolvable conflict | `CompressionFailed(String)`, goal-tracker source preservation, terminal/tool/subagent `user_error`, and manual stopped-compression origin are still open. Preserve legacy content/reminders and LLM visibility exactly; do not remove them in response to a review suggestion. |
| U-6/U-8 workflow frontend | Concentrated projection adapter and component consumers exist; focused projection/hydration/UI tests pass | Backend producers are missing. Automation editor still uses raw error display at three catches. UI-side tests do not establish the live/persisted producer contract. |
| U-8 settings/common/views | Child changed only App setup-error, ScraperTest and three locale files | Other settings, common components and views remain unaudited or unmigrated. Confirmed raw displays include ProxyGroup, Proxy, General, Agent, Skill, Mcp, Sandbox, Decision, Index and Assistant. ProxyStats remains subject to the baseline's inactive-route classification. |
| U-8 stores/helpers handoff that exited without a report | No broad store/helper migration is established; parent confirmed raw user error output in update/window stores | Review each assigned display boundary, preserve silent/cancel paths, and verify directive error fallback safety. |
| U-7 peripheral/CLI handoff | Only CLI Server Display and daemon direct startup stderr changed | CLI mapping discards optional presentation; daemon direct stderr is sanitized text, not a structured diagnostic. Terminal/automation/owner/lifecycle/log sinks and legacy CLI remain open. CLI status/code fields now produce a dead-code warning. |
| U-3 DB telemetry/cache follow-up | Internal acknowledgement and cached failure carriers retain origins. Parent ran both affected suites successfully | Add the requested direct-versus-cached projection assertions; existing suite success does not replace new regression coverage. |

### Verified corrections to read-only review claims

- Updater is not missing: `D/updater/error.rs` implements typed origin and
  projection, and the desktop error tests cover its parameter/unit cases.
- Domain enums intentionally keep their legacy serde payload. Backend AppError
  uses typed `#[from]` variants and dispatches `error_origin()` to each domain;
  direct AppError serialization adds presentation. Do not add projection fields
  to every domain's legacy serde merely because they omit them.
- Client positive presentation coverage exists in
  `an_error_envelope_presentation_is_retained_on_the_typed_error` for both Server
  and Auth, and the parent ran it successfully.
- Existing terminal content and SYSTEM_REMINDER/LLM visibility are protected
  legacy contracts, not newly introduced prompt/content bugs. The approved fix
  is a separate safe UI projection, not changing those bodies.
- Removing text-based tool inference is implemented, but this is only the
  consumer half until the execution bool producer and persisted UI metadata are
  connected. No assertion of complete U-8 coverage is justified.

### Parent-executed checkpoint verification

Commands below were executed after removal of the failed tool-body edits and
restoration of the CCProxy source-chain test. Counts refer to actual test
harness results; filters can include related submodules, not just one file.

| Check | Result |
| --- | --- |
| `cargo check -p chatspeed-runtime-backend -p chatspeed --message-format short` | Passed |
| `cargo check -p chatspeed-runtime-client -p chatspeed-cli -p chatspeed-runtime --message-format short` | Passed; CLI Server status/code dead-code warning remains |
| client old-envelope and positive-presentation filters | 1 + 1 passed |
| desktop `error::tests`, `runtime_client::tests`, `commands::fs::tests::editor_` | 4 + 17 + 3 passed |
| backend `error_diagnostics::`, `ccproxy::helper::retry::tests` | 7 + 5 passed |
| backend `db::runtime::tests`, `db::main_store::tests` | 11 + 9 passed; no new cache-projection regression yet |
| backend `ai::error::tests`, `ai::chat::openai`, `ai::chat::list_models` | 3 + 53 + 3 passed |
| backend `error::tests` filter, contracts `error_` filter | 33 + 2 passed |
| desktop `runtime_chat::tests`, `runtime_capability::tests`, `runtime_automation::tests` | 8 + 6 + 7 passed |
| `cargo test -p chatspeed-cli --bin cscli error::tests` and `client::tests` | 3 + 6 passed |
| `cargo test -p chatspeed-runtime --lib` | 38 passed; does not exercise binary `main.rs::print_failure` stderr capture |
| Node error/capability/messageProjectionRules/workflowSessionHydration/workflowConstitution/workflowUiContract/codeEditorContract/General.configTransfer selected files | 68 passed |
| `git diff --check` | Passed before this documentation update |
| `pnpm build`; fallback `corepack pnpm build` | Not run: pnpm absent; Corepack cannot write `/chatspeed-home/.cache/node/corepack/v1` (EACCES). No package-manager substitution or dependency changes made. |

Desktop test builds also report five dead-code warnings in runtime capability,
logger test setup and terminal shim paths; provenance and the changed-path fix
remain to be reconciled before final acceptance. No visual desktop validation,
all-endpoint CCProxy fixture run, or actual daemon/desktop sink capture has been
completed by this audit. No full repository test suite was run.

### Repair order after the checkpoint

1. Close U-4 workflow and remaining IPC bridge presentation losses and add a
   source-to-client-to-invoke regression with localized positive reasons and
   legacy-keyword negative cases. Do not mark U-4 complete from isolated tests.
2. Complete U-6 backend carriers and canonical metadata producers without
   changing any legacy model content, visibility, retry, stop or persistence
   policy; verify realtime and history with the existing frontend adapter.
3. Complete ordinary-chat UI-only bool metadata, CLI/peripheral diagnostics,
   remaining CCProxy final boundaries, and each remaining frontend display group
   in small disjoint units. Keep implementer concurrency at or below three.
4. Add missing regressions, resolve warnings attributable to this task, perform
   build/render/sink checks when available, then reconcile all AC/INV/V items.
   U-3's boundary/log coverage and U-4 through U-9 must not be represented as
   completed solely because the checkpoint compiles.

## Verified repair checkpoint after bounded re-delegation

The earlier handoff table below is historical evidence, not the current state.
The parent reviewed each returned patch against source and ran focused tests;
child reports alone were not accepted. Implementer concurrency stayed at or below
three and explorer concurrency at or below five. No child remains running from
this repair batch. This checkpoint is not whole-system acceptance.

| Reconciled handoff | Parent acceptance evidence | Remaining boundary |
| --- | --- | --- |
| Workflow/editor/client IPC | Shared `client_error_wire` / `runtime_error_wire` preserve status/code/presentation; actual source reasons and exact frontend comparisons remain. Desktop `runtime_client::tests`: 19 passed; frontend parser negative-keyword tests pass. | U-4's listed session/editor/catalog branches are closed. An additional MCP reconnect prose classifier was discovered outside those examples and remains open under AC-4. |
| AI callbacks and model list | All error callbacks now carry typed `metadata.user_error`, including metadata-absent paths. Parent ran `ai::chat::`: 63 passed. Parent corrected origin priority so actual upstream 405 survives `AiError::user_error()` while legacy retry status remains 500; `ai::error::tests`: 4 passed. | Full four-protocol/direct-adapted loopback matrix is not yet accepted. |
| Desktop chat bridge | Parent reviewed `client_message -> client_error_wire` and supervisor mapping; child ran 11 `runtime_chat::tests`, parent separately verified shared bridge tests. | No live supervisor-to-Vue runtime was exercised. |
| CCProxy diagnostics | Parent removed request-body logging from decode frames; `from_upstream` carries normalized code/status/request id without unrelated body fields. Parent ran diagnostic 4, protocol error output 7, direct rejection/abort 2 tests. External bytes/status/header expectations are unchanged. | Existing retry source-chain report is captured while the typed source is alive; handler summaries are not reconstructed chains. Additional endpoint and sink coverage remains. |
| ReAct compression/tool/subagent | Compression legacy Display/serde/content and UI-only metadata remain separate. Parent ran durable completion 1, legacy signal 1, observation 16, DB completion persistence 2, and command reconcile 14 tests. Parent removed a newly added per-completion database lookup: UI projection uses the restored canonical pending queue or typed result only. | No real desktop visual/restart test; historical records without typed projection cannot recover lost causes. |
| ReAct frontend | Parent fixed dropped final-review `user_error` and nested live progress projection at the central adapter. Snapshot adapter reads only typed terminal metadata. Parent ran 43 workflow contract tests and 24 selected display/snapshot/store tests. Inputs and model result fields remain unchanged. | Desktop narrow/regular viewport rendering remains unverified. |
| Settings/views/stores/common | Parent inspected patches and ran 52 selected safe-message/cancel/state tests before final fixes, then reran changed display tests. Ordinary chat now prioritizes actual `metadata.user_error`, not only a legacy chunk fixture. Existing cancel, notification levels/durations and silent paths are protected. | Complete remaining outlet ledger reconciliation is still required. |
| Peripheral automation/terminal/daemon | Parent ran automation 53, terminal control mapping 8 and daemon final-failure writer capture 1 tests. Terminal unit errors were returned for correction rather than accepted as generic: precise three-locale explanations now preserve legacy tokens/status and avoid echoing permission parameters. | Daemon evidence is a writer capture, not an actual desktop file-sink capture. Existing test-build warnings remain. |
| DB cached failures | Parent added `cached_runtime_failure_preserves_the_direct_error_projection_and_legacy_shapes`: direct and cached worker failures expose the same redacted cause; legacy serde stays unchanged. Parent ran all 10 `db::main_store::tests`. | No schema or recovery policy was changed. |
| Base error unions/contracts/MCP | Parent ran backend `error::tests` filter: 39 passed, MCP error 4, capability error 2, desktop union 4, contracts old envelope 2 and typed client presentation 2. This directly validates serde flattening and both IPC exits instead of assuming compatibility. | Not a claim that every domain construction/log boundary is closed. |

### Explicitly unresolved checks

- `pnpm build` was made runnable through the existing Corepack cache at
  `/tmp/chatspeed-corepack`. The ordinary build failed with exit 134 (V8 heap
  exhaustion at roughly 256 MiB); a second run with
  `NODE_OPTIONS=--max-old-space-size=2048` failed with exit 137 (`Killed`).
  Neither attempt is a passing build. No dependency or lockfile was changed.
- The desktop fern file sink was captured through its actual formatter and
  temporary file output; the parent test confirms one parseable JSON diagnostic,
  preserved code/status and no credential canary. Daemon final-failure coverage
  is a writer capture. No live visual desktop check or cross-process
  daemon-to-desktop forwarding capture has been completed; SFC compilation does
  not replace production bundling or visual acceptance.
- The MCP raw transport/stdio logs cited by the read-only reviewer are unchanged
  lines in the current diff, not new raw logging introduced by the carrier work.
  This does not certify all historical logging as safe, and no broad logger
  rewrite has been performed.
- The MCP reconnect classifier is now typed: `McpError::from_service_error`
  snapshots an internal disposition before legacy stringification;
  `McpClient::call` no longer reads error prose. Parent reviewed and ran `mcp::`
  84 passed (one external npx/network smoke ignored), dedicated client retry
  tests 8 passed and diagnostic carrier tests 5 passed. Unknown/legacy/tool/
  cancelled/auth/permission/timeout cases do not reconnect; typed transport
  cases retain the single retry. Unknown rmcp wrappers are surfaced, not guessed.
- Backend `WorkflowError` has no typed origin to forward. The speculative claim
  that it already carried a discarded reason was rejected after source review.
  Legacy strings must not manufacture reason/status/source chains.
- Five desktop test-build warnings remain in runtime capability shims, logger
  test setup and the terminal shim. Final package checks and complete AC/INV/V
  reconciliation are still pending.

## Characterization evidence

### Fixtures added by U-1

- `contracts/src/lib.rs::error_fixture_reads_the_legacy_two_field_envelope`:
  deserialize and round-trip the old envelope, complementing the existing
  exact serialization fixture.
- `react/error.rs::terminal_error_fixture_keeps_legacy_engine_content_and_tagged_serialization`:
  exact runtime enum JSON, generic terminal content and metadata.
- `react/runtime_observation.rs::terminal_error_fixture_preserves_content_but_hides_typed_llm_projection`:
  real terminal content projected as hidden typed assistant observation versus
  deferred legacy user reminder; stored/UI content is unchanged.

### Existing evidence reused rather than duplicated

- `ccproxy/adapter/output/error_response.rs`: `maps_openai_overload_to_server_error`,
  `maps_backend_503_to_claude_overloaded_error`,
  `maps_backend_529_to_gemini_unavailable_error`, `emits_ollama_error_shape` assert
  full four-protocol JSON and HTTP status. Other tests cover Claude transport
  message/request id and status matrices.
- `ccproxy/errors.rs::backend_request_error_preserves_original_message` and
  `helper/retry.rs::test_backend_request_error_adds_context_once` protect the
  existing transport diagnostic, not every network failure class.
- `react/runtime_observation.rs::terminal_error_is_visible_in_ui_and_hidden_from_llm`
  protects metadata defaults; existing tool/subagent tests protect pairing,
  explicit hide, legacy completion and user/tool wrappers.
- `react/error.rs` already covers auth/billing categories, raw retry-exhaustion
  content and attempts, compression failure, and empty upstream body fallback.

### Verification boundary

Executed from `src-tauri` after the fixture changes:

| Command | Result |
| --- | --- |
| `cargo test -p chatspeed-contracts --lib error_fixture` | 2 passed, 40 filtered out |
| `cargo test -p chatspeed-runtime-backend --lib terminal_error_fixture` | 2 passed, 2091 filtered out |
| `cargo test -p chatspeed-runtime-backend --lib ccproxy::adapter::output::error_response::tests` | 7 passed, 2086 filtered out |

The first backend fixture invocation hit the 120-second command timeout during
compilation; the same focused filter completed successfully with a 600-second
budget (build finished in 64 seconds on retry). No test assertion failed.
`git diff --check` passed. No live service or whole-repository test was run.

The parent supplied pre-edit passing baselines for contracts `error_fixture`, backend
`backend_request_error_preserves_original_message` and
`terminal_error_is_visible_in_ui_and_hidden_from_llm` (2091 backend tests in that
baseline binary), and 42 Node tests across messageProjectionRules,
workflowUiContract, codeEditorContract and General.configTransfer. Those Node
checks are inherited evidence, not newly executed U-1 verification.


## Current reconciled coverage after implementation

The original U-1 rows above are retained as historical baseline evidence. The
following table is the current acceptance ledger and supersedes a stale
`Pending` label when its evidence names a changed boundary and a focused check.

| Active boundary | Current verdict and evidence | Protected behavior / remaining limitation |
| --- | --- | --- |
| Desktop/backend `AppError`, domain errors, HTTP/IPC and MCP JSON-RPC | **Closed for the changed exits.** Both direct `Serialize` and `From<AppError> for String` use the same optional presentation while retaining legacy module/details/message. AI, DB, tools, sensitive, MCP, HTTP and updater origins are captured before stringification. | Legacy wire/Display shapes, status/code and MCP `INTERNAL_ERROR` remain unchanged. Workflow/CCProxy legacy leaves without a typed origin are not guessed. |
| Contracts/client/runtime/CLI | **Closed for the changed control-plane paths.** Optional `UserErrorPresentation` round-trips, old envelopes still decode exactly, client preserves code/status/presentation, and CLI prefers the safe message while retaining Auth exit 4, budget exit 9 and other exit classes. | Old payloads remain display-only and do not trigger state recovery. `src/bin/cs` remains the explicitly retained legacy shim. |
| AI ordinary/streaming/Responses/model-list and chat bridge | **Closed for covered sources.** Raw upstream details remain legacy content, typed `metadata.user_error` is carried through callbacks, EOF/terminal errors remain single-shot, and the chat bridge preserves typed presentations. Tool-call parse failures now capture the live serde error for UI projection and structured diagnostics without changing the legacy chunk. | Provider integration was exercised with local/focused fixtures, not external credentials. The legacy model-visible error/tool content remains intentionally unchanged. |
| CCProxy direct/adapted/Responses/embedding/decision/list-models/stream | **Closed for tested protocol boundaries.** Parent reran chat 6, Responses 14 (including 9 stream HTTP rejection subcases), stream helper 1, embedding 3 (including 27 supported-protocol rejection subcases and the Claude unsupported case), list-models 1, decision 5, direct 6 and error-response 7 tests; status/body/type/code/filtered headers/statistics and stream termination remain protected. | Missing list-model groups retain the existing empty-list behavior because production currently unwraps `ModelAliasNotFound`; this was characterized, not changed. No live external upstream was used. |
| ReAct compression/terminal/tool/subagent/recovery | **Closed for the implemented UI sidecar.** Compression legacy Display/serde/content, terminal LLM hiding, durable/realtime `user_error`, manual stopped compression, typed source capture and cold recovery all have focused tests. No extra hot-path database lookup or model-facing metadata was introduced. | Historical records without a typed projection cannot recover a cause. No live desktop restart/viewport check was possible. |
| Capability/plugin/terminal/automation/daemon/CLI | **Closed for changed presentation exits.** Stable code/status/exit contracts remain, diagnostics are redacted and bounded, daemon final failure is one structured stderr line, and capability HTTP now has an explicit status/code/redaction/presentation regression. | The daemon test is a writer/sink capture, not a live desktop forwarding capture. Existing test-only dead-code warnings remain. |
| Frontend settings/views/common/workflow/chat | **Closed for audited active display sites and state examples.** `formatUserError` is used for user-visible errors; ProxyStats is included because it is mounted by `ProxySwitcher`; stream, snapshot, terminal, subagent, ordinary-chat tool and cancellation paths use structured projections. | Console-only `FrontendAppError.toFormattedString()` remains technical log context by design. Renderer/SFC checks are not a substitute for a live desktop visual check. |
| Ordinary-chat tool result state | **Closed for the active bool path.** Backend captures `Result`/`is_error` and known typed error fields in UI-only metadata; consolidated tool records carry `is_error` and optional snake_case `user_error`. All three renderers use the same state/display adapters; missing legacy discriminators remain `unknown`. | Model/history tool content is not rewritten. Unknown typed error payloads intentionally do not invent a cause. |
| Excluded/compatibility paths | **Explicitly excluded or protected.** DAG, old workflow event gateway, third-party/generated assets, websites and submodules are not migrated. | This is not evidence that inactive or external code has the new projection. |

### Final parent verification record

- Rust focused regression: client 84; AI error 4; OpenAI/Responses 56; model list
  4; desktop chat bridge 11; CCProxy chat 6, Responses 14, stream 1,
  embedding 3, list-models 1, decision 5, direct 6, error-response 7,
  retry 5 and diagnostics 4; ReAct error 10, terminal/recovery/manual filters,
  goal tracking 1, and tool projection/status 10; CLI error 5/client 12;
  daemon final failure 1; automation 7; capability runtime 8; base unions,
  contracts and diagnostics filters all passed in their parent commands.
- Frontend focused regression: the final 16-file display/workflow/safety set
  passed **143/143** after all bounded additions, including ProxyStats and
  ScraperTest SFC compilation, ordinary-chat tool display and snapshot canaries.
  The dock contract failure was a stale narrow regex and was corrected
  to assert teardown ordering across intervening cleanup statements; no runtime
  behavior was changed.
- `cargo check` passed for `chatspeed-runtime-backend`, `chatspeed-runtime-client`,
  `chatspeed-cli`, `chatspeed-runtime` and `chatspeed`. `git diff --check` passed,
  and no unresolved merge files, Cargo/lockfile changes, dependency changes,
  schema migrations, DAG changes or submodule changes were found.
- Production `corepack pnpm build` was attempted with the repository's existing
  pnpm 10.5.2 configuration and a 768 MiB Node heap; Vite transformation was
  killed with exit 137. Earlier normal and 2048 MiB attempts also failed
  (V8 exhaustion/exit 137). This is an environment limitation, not a passing
  build, and no build configuration or dependency was changed.
- No live provider, daemon-to-desktop forwarding, visual narrow/regular-window,
  or external-service test was run. These limitations remain explicit; the
  focused source, wire, sink, redaction and compatibility checks above are not
  being promoted to full end-to-end acceptance.

### Final parent logic-review checkpoint

The user accepted partial delivery for the environment-bound checks and is
performing production builds and real desktop acceptance locally. These checks
are handed off, not marked passed. The remaining agent-owned work was source
logic review and bounded regression verification, not another build attempt.

Three read-only review reports (transport/security, AI/proxy/MCP, and ReAct)
were collected individually. The parent checked their concrete findings against
current source and targeted diffs rather than accepting the reports as proof.
No additional implementation agents were started in this review round.

Two actual omissions were corrected during parent review:

- `src/stores/workflow.js::finalizeToolExecution` referenced an undefined
  `errorDisplay` in the failure branch. The regression test first failed with
  `ReferenceError`, then passed after deriving the safe display from the existing
  projection or formatted result. The ledger stores the same display/copy text;
  `StatusPanel.vue` prefers it. Raw result/message and the boolean-driven
  success/failure status remain unchanged, including successful output containing
  `Error:`. The regression also covers a missing ledger entry and credential
  redaction.
- `B/ai/error.rs::AiError::diagnostic` left the top-level module/operation blank.
  It now sets `module=ai` and uses the explicit origin frame's operation. All AI
  variants assert nonempty responsibility fields while retaining exact legacy
  Display/serde and upstream-status precedence.

After the final executable changes, `cargo test -p chatspeed-runtime-backend
--lib ai::error::tests` passed 4/4. The 15-file Node display/state/workflow set
passed 141/141, including the real extracted store handler, safe parser,
capability, tool state, subagent snapshot, workflow constitution/hydration,
editor, config transfer, settings, views and SFC compilation. The final
store/view test rerun passed 20/20 and also includes `StatusPanel.vue` SFC
compilation. `git diff --check` passed. These are focused checks, not a production
build.

Proxy transport diagnostics were traced through retry, handler and response
boundaries. The retry boundary captures the live source chain; later boundaries
retain stage-specific summaries/context, not reconstructed or repeated full
source chains. Multiple stage lines remain intentional; this is not a claim of
one total log line per request. Optional malformed-presentation hardening and
unchanged legacy raw payloads were not promoted into unrelated rewrites.

The user's clarified exception permits third-party prose classification only
when the third party provides no reliable type, code or status. It does not
permit first-party message-based recovery, nor require replacing the current
safer typed MCP classifier. JSON/body extraction for display remains distinct
from state classification. Historical plain strings cannot recover lost causes.

### Acceptance reconciliation (not an unconditional release sign-off)

| Contract | Evidence / disposition |
| --- | --- |
| AC-1 | Safe projections wired to both AppError exits, typed client/IPC, AI error metadata, terminal/tool/subagent adapters and audited frontend surfaces; Node display/redaction tests and union tests passed. Live viewport review remains outstanding. |
| AC-2 | Typed origins/carriers cover lossy conversions, stopped/manual compression and ordinary chat tools. Legacy-only data is explicitly not reconstructed. Focused source/wire/history tests passed. |
| AC-3 | Bounded JSON diagnostics retain explicit frames and correlation; backend facade, desktop fern file sink and daemon final-failure writer capture passed. Live cross-process forwarding and per-outlet notification/log frequency are not exhaustively verified. |
| AC-4 | Session/editor/catalog/runtime and MCP reconnect paths use validated reason/code/type/bool; localized positives and keyword negatives passed. Chat tool missing status is unknown. Remaining matched text parsing is display/protocol/model parsing, not inferred control state. |
| AC-5 | Old exact envelope and carrier String serde/Display fixtures passed; optional presentation survives Auth/Server and IPC. CLI exits unchanged. Legacy CLI target is a replacement notice shim, so its old unreferenced source tests are not counted as executed tests. |
| AC-6 | ReAct compression/terminal/tool/subagent live-to-durable adapters and ordinary-chat tool UI sidecars passed focused tests without changing model content. Live restart/visual acceptance remains outstanding. |
| AC-7 | Baseline inventory, current group coverage, corrections (including active ProxyStats), parent acceptance and explicit limitations are recorded. Exhaustive live per-outlet triggering has not been completed; this criterion is not fully signed off. |
| INV-1 | Local CCProxy fixtures pin body/status/type/code/header filtering, retry/backoff, stream failure and usage/stat behavior. Supported embedding protocols have 27 HTTP status/body subcases; native Responses stream requests have 9 HTTP rejection subcases. Cross-protocol embedding combinations and real external upstreams were not exercised; existing embedding errors return `InternalError` rather than forwarding an HTTP response, and that behavior remains unchanged. |
| INV-2 | Compression legacy content and serde, model tool messages, subagent signal fields, stop/manual lifecycle, durable terminal deduplication and UI-only metadata checked. Cold failed-child recovery reads a snapshot only when its optional projection is missing; no hot-path lookup remains. |
| INV-3 | Existing HTTP code/status, auth classification, CLI exits, typed recovery sources and unknown-error negative cases protected by focused tests. |
| INV-4 | No dependency/lockfile, DAG, submodule or schema SQL change. The two modified `db/sql/migrations/*_schema.rs` files only preserve typed query error causes; migration definitions are unchanged. No Git staging/commit performed. |
| INV-5 | Canary, JSON/Bearer/query/userinfo/env/header, Unicode/bounds and text-only fallback tests passed. Legacy model strings remain protected and are not claimed to be sanitized UI projections. |
| V-1 / V-2 / V-3 / V-4 / V-7 | Focused source, baseline, serde, message, state and peripheral/CLI checks run; named commands/results above. Existing unrelated DB fixture failures remain recorded in the U-3 section. |
| V-5 / V-6 / V-8 | Focused protocol, ReAct, projection and sink checks passed; exhaustive protocol combinations and real cross-process/desktop checks remain partial. |
| V-9 | **Blocked/partial.** Production build fails under available memory limits; real desktop narrow/regular-window, live native invoke and daemon forwarding tests were not run. No full-system acceptance claim is made. |

### Follow-up: live tool failures and child-process log filtering

The final review found a live-only tool ledger gap: `tool_failed` initially
stored legacy display text, and the later durable tool message did not replace
its display/copy/summary. Both ordinary typed errors and reinforced failures now
carry the same UI-only `error_details.user_error` as the durable observation.
The frontend consumes that sidecar and updates the ledger on the later message;
reload uses the same central display resolver. No model content, raw result,
state transition, event count or durable message write was changed. Parent
source review confirmed both producer paths and the live/reload consumers;
the independent frontend review was checked against current code.

Follow-up verification:

- `cargo test -p chatspeed-runtime-backend --lib tool_terminal_dispatch_`:
  2 passed, covering ordinary/reinforced equality, private security/sandbox
  detail hiding, unchanged model content, state and event count.
- The six-file Node workflow/display/hydration/constitution/ChatHub set:
  81 passed. The last two-file store/projection rerun passed 17 tests.
  Event-order cases cover seeded/missing ledgers and replacement by the later
  typed message; successful text containing `Error:` stays successful.
- Final `cargo test -p chatspeed --lib runtime_client::tests`: 20 passed,
  including `child_log_records_preserve_targets_levels_and_file_filtering`.
  That regression checks a real fern file sink rejects dependency
  Info/Debug/Trace while retaining Warn/Error, project logs and startup JSON
  diagnostics. Single-file rustfmt checks and `git diff --check` also passed.
- `cargo test -p chatspeed-runtime --bin chatspeed-runtime`: 4 passed for
  target boundaries, source filtering and sanitized final startup reporting.
  Third-party Warn/Error remain eligible; third-party Info and lower are
  rejected even when project debug logging is enabled.

The old desktop forwarding closure unconditionally recorded child stderr as
project Info. It predated this bounded follow-up; the evidence does not prove
it was introduced by the error-feedback migration. Runtime source filtering
now matches the intended external-log policy, and desktop forwarding preserves
parsed target and level before applying existing console/file filters. Bare
stderr remains sanitized terminal output, not project Info in the log file.
No logging framework, dependency or configuration switch was added.

The user confirmed desktop startup works after reboot, and another desktop app
had also crashed before reboot. No shortcut/startup behavioral workaround was
made on the basis of the last log line. System instability is consistent with
that observation, but no native crash dump established its precise cause.
Production build and regular/narrow viewport acceptance remain user-owned;
unit/file-sink checks are not claimed as a live cross-process launch test.

