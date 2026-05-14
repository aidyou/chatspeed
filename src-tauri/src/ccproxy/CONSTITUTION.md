# CCProxy Constitution

This document is the highest-priority maintenance contract for `src-tauri/src/ccproxy`.

It exists to keep the proxy engine aligned with its real runtime architecture, prevent protocol drift, and stop convenience patches from creating silent compatibility regressions across OpenAI, Claude, Gemini, and Ollama paths.

If this document conflicts with ad-hoc local behavior, this document wins.

## 1. Scope

This constitution governs:

- `src-tauri/src/ccproxy/*`
- route handlers and middleware mounted through `src-tauri/src/ccproxy/router.rs`
- adapters under `src-tauri/src/ccproxy/adapter/*`
- stream, retry, auth, model-resolution, and header-filtering logic used by ccproxy

This constitution does not replace implementation guides or protocol notes. It constrains them.

## 2. Primary Goal

The ccproxy module is a protocol-faithful transport and adaptation engine, not a best-effort chat wrapper.

Its first responsibility is correctness of:

- route resolution
- model resolution
- authentication
- direct forwarding
- cross-protocol normalization
- tool compatibility behavior
- stream conversion
- response/header safety

New features must not weaken those guarantees.

## 3. Non-Negotiable Invariants

### 3.1 One request has one canonical execution path

Every chat request must collapse into exactly one of these paths:

- direct forward
- unified adaptation

The module must not grow a third semi-canonical path with slightly different semantics.

### 3.2 `UnifiedRequest` and `UnifiedResponse` are the canonical adaptation boundary

Cross-protocol behavior must normalize through the unified model defined in `adapter/unified.rs`.

If a request or response is being adapted across protocols, correctness must come from:

- `UnifiedRequest`
- `UnifiedResponse`
- `UnifiedStreamChunk`
- typed adapter traits

It must not come from protocol-specific string surgery spread across handlers.

### 3.3 Compatibility logic is an adapter, not a new protocol

Tool compat mode, legacy field support, and provider quirks are allowed only as normalization layers.

They must collapse immediately into:

- canonical route intent
- canonical unified structures
- canonical stream/output semantics

No new feature may treat compat mode as an independent main protocol.

### 3.4 Proxy metadata may override behavior, but not architecture

Model metadata, prompt injection settings, custom params, and header injection may alter request behavior.

They must not bypass:

- authentication
- model resolution
- direct-vs-adapt path selection
- header filtering
- statistics/logging obligations

## 4. Routing Law

### 4.1 Route order is architectural, not cosmetic

The router composition order is part of correctness.

The priority must remain:

1. fixed prefixes
2. direct protocol routes
3. global compat routes
4. grouped routes

Any change that can cause route shadowing is prohibited unless the full route matrix is revalidated.

### 4.2 `/switch` is an alias resolver, not a second routing system

Dynamic switching must resolve immediately to the active configured group.

After resolution, downstream logic must operate on the resolved group exactly as if the client had requested it directly.

### 4.3 Route-level protocol identity must stay explicit

Handlers must continue to enter the engine with an explicit client protocol and explicit compat intent.

The engine must not guess client protocol from body shape when the route already defines it.

## 5. Authentication Law

### 5.1 Authentication is mandatory unless an explicit trusted bypass applies

Requests must be authenticated unless they are:

- explicitly allowed local loopback traffic
- validated internal requests
- validated workflow-session requests

Adding a new bypass path without equivalent trust guarantees is prohibited.

### 5.2 Token source precedence must stay deterministic

Authentication must keep a single deterministic lookup order across:

- `Authorization: Bearer`
- `x-api-key`
- `key` query parameter

Do not add ambiguous “best effort” token discovery logic.

### 5.3 Auth failure must fail closed

Missing, invalid, or mismatched credentials must stop the request before any provider call is attempted.

Silent downgrade to anonymous access is prohibited.

## 6. Model Resolution Law

### 6.1 `ModelResolver` is the authority for backend selection

Provider selection, alias lookup, group lookup, key rotation, HTTP client construction, parameter merging, and proxy header injection must remain centralized in `ModelResolver` and its direct helpers.

Handlers must not become a second model-resolution engine.

### 6.2 Header-based provider targeting is explicit override logic

If `x-cs-provider-id` and related model headers are supplied, they must remain a first-class explicit targeting path.

That path must not silently diverge from alias-based resolution semantics.

### 6.3 Alias parsing must normalize early

Forms like `group@alias` and route-bound Gemini model aliases must be normalized before backend dispatch.

Downstream layers must receive resolved backend intent, not raw client ambiguity.

## 7. Request Normalization Law

### 7.1 Protocol-specific input parsing belongs at the edge

OpenAI, Claude, Gemini, and Ollama request bodies may differ.

Those differences must be normalized at input adapters or request preprocessors, not leaked deep into unrelated handler code.

### 7.2 Preprocessing must be provider-specific and narrowly scoped

Provider workarounds such as DeepSeek tool-choice relaxation or reasoning replay repair must stay isolated in preprocessing logic.

Provider quirks must not become broad mutations applied to unrelated backends.

### 7.3 Sampling parameter merge order is fixed

The effective parameter hierarchy must remain:

1. client-provided values
2. proxy/model-config fallback values
3. protocol defaults

A config layer must not silently overwrite an explicit client value unless the rule is explicitly defined as scaling or normalization.

## 8. Direct Forward Law

### 8.1 Direct forwarding is the only fast path

Direct forwarding is allowed only when:

- client protocol equals backend protocol
- final tool compat mode is off

If either condition is false, the request must go through unified adaptation.

### 8.2 Direct forwarding still owes proxy guarantees

The fast path is not a bypass around correctness obligations.

Direct forwarding must still preserve:

- proxy header injection
- request-body normalization
- model field correction
- retry policy
- logging/stat recording
- response header filtering

### 8.3 Direct and adapted paths must remain behaviorally comparable

A feature added to one path that affects retries, stats, header safety, or model targeting must be reviewed for the other path in the same change.

## 9. Adaptation Law

### 9.1 Backend adapters own protocol translation

Backend-specific request/response/stream conversion must remain inside `BackendAdapter` implementations.

Handlers may orchestrate adapters, but they must not reimplement protocol translation inline.

### 9.2 Output adapters own client-format emission

Once the engine has a unified response or unified stream chunk, client-format serialization must be handled by output adapters.

Do not scatter client wire formatting across stream helpers or route handlers.

### 9.3 Type safety beats permissive guessing

Protocol fields with known instability or provider variance must keep tolerant but typed representations, such as optional fields and signed indices where required.

Deserialization shortcuts that trade type safety for temporary convenience are prohibited.

## 10. Tool Compatibility Law

### 10.1 Compat mode is resolved once

The final compat decision must come from the combination of:

- route compat intent
- proxy model compat metadata

After that decision is made, downstream logic must use the resolved value consistently.

### 10.2 Prompt injection is a structured transformation, not transcript magic

Compat prompt generation, tool prompt construction, prompt enhancement, and prompt replacement must happen as request transformations before backend dispatch.

Tool support must not depend on later reparsing assistant prose unless the adapter explicitly defines that conversion boundary.

### 10.3 Tool payload semantics must survive conversion

When native tools or compat-generated tools are emitted, the system must preserve:

- stable tool ids
- tool names
- argument structure
- stream ordering

Lossy conversion that makes tool execution ambiguous is prohibited.

## 11. Streaming Law

### 11.1 `UnifiedStreamChunk` is the canonical stream event model

Backend SSE or chunked responses must normalize into unified stream chunks before client-format emission.

The stream pipeline must not emit protocol-specific semantics directly from backend bytes when adaptation is active.

### 11.2 `SseStatus` is the canonical per-stream state carrier

Stream state such as message start, tool ids, content-block progress, token estimation, and compat buffers must remain centralized in `SseStatus`.

Do not create parallel hidden stream-state trackers with different semantics.

### 11.3 Stream processing must preserve tool and text ordering

Reassembly, adaptation, and output emission must not reorder:

- text deltas
- thinking deltas
- tool start/delta/end events
- message stop events

If a backend requires buffering, the buffer must preserve observable order.

### 11.4 Stream completion must remain stat-safe

Whether the path is direct or adapted, stream completion must still record usable usage/stat data and keep enough information for postmortem debugging.

## 12. Header and Response Law

### 12.1 Header filtering is mandatory on every provider response

Any response returned to the client after passing through ccproxy must filter transport-managed headers through `filter_proxy_headers`.

No exception is allowed for “simple pass-through” cases.

### 12.2 Business-critical metadata must survive filtering

Filtering must remove transport-dangerous headers while preserving useful upstream metadata such as:

- `x-*`
- `retry-after`
- request identifiers
- rate-limit headers

### 12.3 Error responses must stay protocol-safe and observable

If the backend returns an error, ccproxy must return a client-safe error payload or direct error body while still:

- preserving relevant upstream metadata
- recording stats
- logging enough context for diagnosis

## 13. Embedding and Model-List Law

### 13.1 Non-chat endpoints still obey the same authority boundaries

Embedding and model-list handlers may be simpler than chat completion, but they must still respect:

- authenticated entry
- centralized model lookup
- adapter boundaries
- retry behavior
- response safety

### 13.2 Protocol gaps must fail explicitly

If a protocol is unsupported for a capability, such as Claude embeddings, the module must fail explicitly and consistently.

Fake support through hidden fallback behavior is prohibited.

## 14. Observability Law

Any change affecting routing, model selection, adaptation, compat logic, stream handling, or provider retries must keep logs good enough to answer:

- what client protocol entered
- what backend protocol was selected
- whether the request used direct or adapted flow
- whether compat mode was active
- what model/group/provider was resolved
- whether the failure happened before send, during send, during adaptation, or during emission

If a change reduces traceability, it is not acceptable.

## 15. Forbidden Patterns

The following patterns are forbidden unless they are strictly isolated compatibility shims:

- modifying router order without proving route-shadow safety
- adding a protocol-specific fast path outside direct forward or adapters
- reparsing assistant prose to reconstruct tool semantics when structured data exists
- forwarding `Content-Length`, `Transfer-Encoding`, `Connection`, or `Content-Encoding` from upstream
- applying provider-specific hacks globally
- making compat mode and native mode diverge in tool id or stream semantics without an explicit reason
- adding stats, retry, or header behavior to only one execution path by accident

## 16. Required Review Questions

Every change touching this module must answer:

1. Is this request staying on the direct path or the unified path, and why?
2. What is the single authoritative layer for this behavior: router, resolver, preprocessor, backend adapter, output adapter, or stream helper?
3. Does this preserve route-shadow safety?
4. Does this preserve header safety?
5. Does this preserve tool and stream semantics across protocols?
6. Is this a normalization rule or the start of a parallel subsystem?

If these answers are not explicit, the change is not ready.

## 17. Minimum Validation Before Merge

Changes touching routing, model selection, direct forwarding, adaptation, compat mode, streaming, or response handling must validate the relevant scenarios:

1. authenticated request success
2. authentication failure
3. direct same-protocol request
4. cross-protocol adapted request
5. compat-mode request with tools
6. streaming request
7. backend error propagation
8. header preservation and header filtering
9. provider/group selection if affected

Manual validation is acceptable, but it must be stated.

## 18. Amendment Rule

This constitution may be changed only when:

- the new rule is more explicit than the old one
- the change is justified by architecture, not convenience
- the change reduces ambiguity instead of introducing it

If a future patch needs to bypass this constitution to “quickly fix” provider compatibility, the correct assumption is that the patch is probably wrong.
