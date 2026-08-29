# Model Catalog Configuration Guide

This document explains the purpose, data structure, field meanings, matching rules, and maintenance workflow of `model_catalog.json`.

- **Config file**: `src-tauri/assets/model_catalog/model_catalog.json`
- **Documentation**: `src-tauri/assets/model_catalog/model_catalog.md` (this document)
- **Design background**: `docs/model-catalog-design.md`
- **Parsing implementation**: `src-tauri/src/ai/model_catalog.rs`

## 1. Roles of the documentation and the configuration file

### 1.1 Role of `model_catalog.json`

The JSON is the machine-readable model catalog used by the ChatSpeed backend and centrally records two kinds of information:

1. **Model capability profiles (Profile)**: identifies the model family, reasoning capability, function calling, image input, context and output limits, and the recommended temperature available when creating new models, based on the model ID.
2. **Request transport adaptation (Transport)**: selects the thinking parameter adapter already implemented in Rust based on the model profile, the actual request endpoint, and the protocol.

The model catalog is neither a provider configuration nor a user runtime configuration:

- Do not write API keys, secrets, passwords, tokens, or other sensitive information into it;
- Do not write provider Base URLs, account, or model instance configuration into the catalog;
- The catalog describes reusable model facts and protocol rules; the user's own model configuration is still persisted by the database and the frontend model settings.

Rust embeds the JSON into the application at compile time with `include_str!`, then parses and caches it at runtime. Therefore, modifying the JSON requires rebuilding the application for the change to take effect; editing the file next to an installed program alone does not change the already-compiled catalog.

### 1.2 Role of `model_catalog.md`

This document is the schema/operational guide intended for developers and maintainers. It is not parsed by Rust and does not affect model matching results. It is used to:

- explain the meaning, type, and usage boundary of every JSON field;
- describe the wildcard, priority, conflict, and endpoint matching rules;
- record the correspondence between `thinkingAdapter` and its Rust implementation;
- guide how to maintain the catalog when adding new models, providers, and source evidence;
- avoid conflating capability detection, request adaptation, and user configuration.

If the field structure or implementation rules change, this document, the JSON validation, and the related tests must be updated together.

## 2. Overall structure

The top-level structure of the current file is as follows:

```json
{
  "version": 1,
  "defaults": { "capabilities": {} },
  "profiles": [],
  "transports": []
}
```

### 2.1 Top-level fields

| Field | Type | Required | Purpose |
| --- | --- | --- | --- |
| `version` | integer | yes | Catalog schema version. Only `1` is supported. An unsupported version fails catalog parsing instead of silently parsing with an old format. |
| `defaults` | object | yes | The base result used when no profile matches. Currently declares the unknown defaults of the capability fields. |
| `profiles` | array of objects | yes | Model capability profile rules. Rules match by model ID and can be reused across providers. |
| `transports` | array of objects | yes | Endpoint transport adaptation rules. The rules decide which Rust thinking adapter an actual request uses. |

`profiles` and `transports` may be empty arrays, but the fields must be present; the parser validates rule IDs, references, and field conflicts.

## 3. `defaults` default capabilities

```json
{
  "defaults": {
    "capabilities": {
      "reasoning": null,
      "functionCall": null,
      "imageInput": null
    }
  }
}
```

| Field | Type | Purpose |
| --- | --- | --- |
| `defaults.capabilities` | object | Sets the capability result used when there is not enough catalog evidence. |
| `defaults.capabilities.reasoning` | `true` / `false` / `null` | Whether reasoning is supported. `null` means unknown, not unsupported. |
| `defaults.capabilities.functionCall` | `true` / `false` / `null` | Whether function calling is supported. |
| `defaults.capabilities.imageInput` | `true` / `false` / `null` | Whether image input is supported. |

Capability fields are three-state values:

- `true`: reliably confirmed as supported;
- `false`: reliably confirmed as not supported;
- `null` or omitted: not enough evidence; must remain unknown.

Do not write unknown capabilities as `false` just to fill the fields. Upper layers may apply generic heuristics to unknown values or let the user choose, but unknown and explicitly unsupported are semantically different.

## 4. `profiles` model capability profiles

Each profile describes the common capabilities of a set of model IDs. A profile is not bound to an actual provider endpoint, so the same model can reuse the same profile whether it is accessed through an official API or an aggregator service.

```json
{
  "id": "qwen3-thinking-family",
  "priority": 100,
  "match": { "model": ["qwen3.5*", "qwen3-*"] },
  "family": "qwen",
  "capabilities": {
    "reasoning": true,
    "functionCall": true
  },
  "reasoning": {
    "supportedEfforts": ["none", "high"],
    "defaultEffort": "high"
  },
  "sources": [
    { "url": "docs/reasoning/qwen.md", "verifiedAt": "2026-08-28" }
  ]
}
```

### 4.1 Profile fields

| Field | Type | Required | Purpose |
| --- | --- | --- | --- |
| `id` | non-empty string | yes | Unique profile identifier. Transports can reference profiles by this ID; rules of the same kind should use stable, readable names. |
| `priority` | integer | yes | Profile merge priority. The larger the value, the later it is applied, usually to let exact model rules override family rules. |
| `match` | object | yes | Profile matching conditions. Currently mainly uses `model` to match model IDs. |
| `family` | string, optional | no | Model family name, used for capability inference, transport adaptation matching, and diagnostics. It is not a provider name. |
| `capabilities` | object, optional | no | Capability fields provided by the profile. Only fill in confirmed values; omitted fields remain unknown or inherit existing values. |
| `contextSize` | non-negative integer, optional | no | Model context size. Omit it when there is no reliable source; do not guess. |
| `maxOutputTokens` | non-negative integer, optional | no | The maximum number of output tokens the model allows. Values explicitly returned by the provider API take precedence over catalog-completed values. |
| `recommendedTemperature` | number, optional | no | Recommended temperature when creating or importing a model; used only for UI prefill. It never overrides user-saved configuration or an explicit temperature in a client request. |
| `reasoning` | object, optional | no | Reasoning effort policy describing the supported levels and the default level. Only fill it in when a configurable effort is confirmed for the model. |
| `sources` | array of objects, optional | no | Records the source and verification date of the rule for auditing and future updates. |

### 4.2 `capabilities` fields

`capabilities` in a profile uses the same fields as `defaults.capabilities`:

- `reasoning`: whether reasoning/thinking is supported;
- `functionCall`: whether tool/function calling is supported;
- `imageInput`: whether image input is accepted.

The field names use camelCase because the JSON and the Tauri output face the frontend; the Rust structs use snake_case names such as `function_call` and `image_input`, mapped via serde.

### 4.3 `reasoning` field

```json
{
  "reasoning": {
    "supportedEfforts": ["none", "low", "high", "max"],
    "defaultEffort": "high"
  }
}
```

| Field | Type | Purpose |
| --- | --- | --- |
| `supportedEfforts` | non-empty array of strings | The reasoning effort values allowed by the model and the protocol. `none` usually means reasoning is disabled. |
| `defaultEffort` | string | The default effort; it must appear in `supportedEfforts`, otherwise catalog validation fails. |

The effort here describes model capability and protocol policy; it is not a value forcibly written into every request. User-selected values and existing client configuration should still take precedence over the default suggestions provided by the catalog.

## 5. `transports` endpoint transport adaptation

A transport describes "how thinking parameters are serialized when a model is sent to a specific actual endpoint". The model name alone is not enough to decide the adapter: an aggregator endpoint may use an OpenAI-compatible protocol but may not accept a specific vendor's proprietary fields.

```json
{
  "id": "deepseek-official",
  "priority": 100,
  "match": {
    "profile": ["deepseek-reasoning-family"],
    "endpointHost": ["api.deepseek.com", "api.deepseek.cn"]
  },
  "thinkingAdapter": "deepseek",
  "sources": [
    { "url": "docs/reasoning/deepseek.md", "verifiedAt": "2026-08-28" }
  ]
}
```

### 5.1 Transport fields

| Field | Type | Required | Purpose |
| --- | --- | --- | --- |
| `id` | non-empty string | yes | Unique transport identifier; it can also be explicitly specified by user metadata. |
| `priority` | integer | yes | Transport selection priority. Higher-priority rules win over lower-priority rules. |
| `match` | object | yes | Combined matching conditions on the model profile, model ID, endpoint host, and backend protocol. |
| `thinkingAdapter` | enum string | yes | The type-safe Rust adapter to use. Arbitrary JSON rewriting logic cannot be configured. |
| `sources` | array of objects, optional | no | Records the source and verification date of the transport field rules. |

A transport should at least bind an actual `endpointHost`, unless it is an explicit compatibility override rule. Broad rules without endpoint constraints must not automatically send official vendor fields to aggregator services.

### 5.2 Available `thinkingAdapter` values

The values and their uses in the current implementation are as follows:

| JSON value | Rust adapter | Purpose |
| --- | --- | --- |
| `open_ai` or `openai` | `OpenAi` | OpenAI reasoning request fields |
| `claude` | `Claude` | Anthropic Claude thinking fields |
| `gemini` | `Gemini` | Gemini thinking configuration |
| `deep_seek` or `deepseek` | `DeepSeek` | DeepSeek thinking fields |
| `qwen` | `Qwen` | Alibaba Qwen thinking fields |
| `glm` | `Glm` | Zhipu GLM fields |
| `kimi` | `Kimi` | Kimi/Moonshot fields |
| `step_fun` or `stepfun` | `StepFun` | StepFun fields |
| `hunyuan_hy4_preview` | `HunyuanHy4Preview` | Tencent Hunyuan HY4 Preview fields |
| `doubao` | `Doubao` | Doubao/Volcano Engine fields |
| `sense_nova` or `sensenova` | `SenseNova` | SenseTime SenseNova fields |
| `mistral` | `Mistral` | Mistral reasoning fields |
| `mimo` | `Mimo` | Xiaomi MiMo fields |
| `minimax` | `Minimax` | MiniMax fields |
| `nvidia_nim` | `NvidiaNim` | NVIDIA NIM compatible endpoint fields |

An adapter name only selects an existing Rust implementation; before adding a new name, you must first implement the corresponding type-safe conversion and add input and output path tests.

## 6. `match` matching conditions

`match` is an object in which every filled condition must be satisfied simultaneously; array elements within the same condition are in an OR relationship.

| Field | Type | Applies to | Purpose |
| --- | --- | --- | --- |
| `model` | array of strings | Profile/Transport | Matches model IDs. Suitable for canonical names, namespaced names, and necessary aliases. |
| `profile` | array of strings | Transport | Matches already-hit profile IDs. Used to associate capability profiles with endpoint adaptation. |
| `endpointHost` | array of strings | Transport | Matches the host name resolved from the Base URL. Prefer the precise official domain. |
| `backendProtocol` | array of strings | Transport | Matches the backend protocol, e.g. `openai`, `claude`, or `gemini`, to distinguish different request paths on the same endpoint. |

Inputs and patterns are trimmed of leading/trailing whitespace and converted to ASCII lowercase before matching. Matching uses whole-string semantics:

- `*` matches any number of characters (including the empty string);
- `?` matches exactly one character;
- without a wildcard the whole value must match exactly; there is no implicit substring matching;
- for example, `qwen3-*` matches `qwen3-7b` but not `my-qwen3-7b`;
- pattern arrays are in an OR relationship, while different `match` fields are in an AND relationship;
- leaving a `match` field empty means the condition is not restricted.

## 7. Merging, priority, and conflicts

### 7.1 Profile merging

The resolver starts from the default capabilities, applies all matching profiles, and processes them by priority from low to high:

1. family-level wildcard rules provide the base information first;
2. higher-priority exact model rules override or supplement the base information;
3. rules producing different values at the same priority must not be silently resolved by array order or `HashMap` order;
4. conflicts must be surfaced by catalog validation or tests and make resolution fail.

Therefore, new exact-model rules should use a `priority` higher than the generic family rule. The profile's `family`, capabilities, limits, recommended temperature, and reasoning policy are all part of the profile result; unknown fields should remain `null`/omitted.

### 7.2 Transport selection

Transport selection must consider the model, the matched profiles, the actual endpoint, and the backend protocol at the same time:

1. explicit metadata overrides are handled first;
2. without an override, transports are matched by endpoint and protocol;
3. the same priority must resolve to a unique adapter;
4. when the resolution is not unique or nothing matches, return `null` and do not send vendor-specific fields;
5. official endpoint rules should use precise hosts to avoid accidentally matching aggregator services.

The explicit override key is `modelCatalogTransport`, and its value must be an existing transport ID. For example:

```json
{
  "modelCatalogTransport": "deepseek-official"
}
```

This override means the user has explicitly confirmed that the current custom gateway is compatible with that vendor's fields. It is not a provider-wide default and only affects requests of the model carrying this metadata. Do not write overrides into normal profile rules of the catalog.

## 8. Configuration values and "not set"

`recommendedTemperature` in the model catalog is a recommendation, not a mandatory default, and it is not a field every model must have.

- when the catalog has no `recommendedTemperature`, the result is not set;
- when creating/importing a model, the recommended value may be used for prefill only when the form field itself is empty;
- neither user-saved temperatures nor temperatures explicitly passed in requests may be overridden by the recommended value;
- for model parameters that support "not set", if the configuration file omits the parameter or uses the corresponding not-set representation, keep it as `None` and omit the field during request serialization instead of replacing it with a generic default;
- a `null`/omitted capability means "unknown"; do not interpret it as `false`. This is a different semantic from "not set" for sampling parameters.

The catalog only provides model facts and recommendations; it cannot bypass user settings or the parameter precedence of the request layer.

## 9. `sources` provenance records

```json
{
  "sources": [
    {
      "url": "docs/reasoning/qwen.md",
      "verifiedAt": "2026-08-28"
    }
  ]
}
```

| Field | Type | Purpose |
| --- | --- | --- |
| `sources[].url` | string | The evidence source address or an in-repo document path. It should explain the basis of the model capability or transport field. |
| `sources[].verifiedAt` | `YYYY-MM-DD` string | The date of the most recent verification. Re-verify when a model provider changes its protocol. |

Source records are the basis for maintenance and review; they are not automatically checked online and do not change the resolver's matching behavior.

## 10. Maintenance workflow and notes

When adding or updating rules:

1. first confirm the model's official ID, capabilities, and protocol fields, and record the evidence in `sources`;
2. choose the narrowest model pattern to avoid pulling unrelated models into the same family;
3. fill in capabilities, context, output limits, and recommended temperature only with confirmed values;
4. official endpoint transports must bind a precise `endpointHost`; aggregator endpoints must not automatically apply official fields;
5. when compatibility with a custom gateway is needed, prefer the explicit `modelCatalogTransport` override;
6. before adding an adapter, check whether the Rust enum and the corresponding thinking module are already implemented;
7. add tests for matching, priority, conflict, official endpoints, aggregator endpoints, and explicit overrides;
8. rebuild the application after changing the JSON and confirm that the embedded resource and the Tauri command output stay consistent;
9. update this document and `docs/model-catalog-design.md` in sync (when the design boundary changes).

Do not do the following:

- do not store credentials or user private data in the JSON;
- do not decide official transport fields directly from the model name;
- do not use `false` in place of unknown capabilities;
- do not fill in unverified token limits or recommended parameters;
- do not bypass the restricted Rust adapters with arbitrary JSON patches;
- do not rely on rule array order to resolve same-priority conflicts.

## 11. Relationship with the code

The main usage paths are as follows:

- `src-tauri/src/ai/model_catalog.rs`: loads, parses, validates, matches, and merges the catalog;
- `src-tauri/src/ai/chat/list_models.rs`: completes capabilities and limits in provider model lists;
- `src-tauri/src/commands/model_catalog.rs`: provides profile query commands to the frontend;
- `src/components/setting/Model.vue`, `src/stores/model.js`: model creation, import, and configuration prefill;
- `src-tauri/src/ccproxy/helper/thinking/`: type-safe normalization of per-vendor thinking parameters;
- `src-tauri/src/ccproxy/handler/` and the backend adapters: use the resolved transport in unified and direct requests.

When the catalog does not match or the resolution is uncertain, the safe behavior is to keep capabilities unknown and skip uncertain vendor-specific transport adaptation, rather than guessing and sending potentially incompatible fields.