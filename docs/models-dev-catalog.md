# Models.dev catalog integration

ChatSpeed ships a validated Models.dev `catalog.json` snapshot under `src-tauri/assets/models_dev/`, together with the upstream MIT attribution in `LICENSE`. The snapshot is the offline source for provider/model capabilities, limits, reasoning options, and pricing metadata.

At startup the application loads the last known-good app-data snapshot, falling back to the bundled snapshot when it is missing, oversized, or invalid. Queries use the in-memory index and never access the network on a chat or model-resolution critical path. A background refresh is attempted only after 24 hours since the last successful snapshot write. It follows the global `proxy_type` setting (`none`, `system`, or authenticated `http`); download, parse, and atomic replacement failures retain the previous snapshot.

User-saved model values remain authoritative. Catalog data only pre-fills new model forms, while provider API facts remain higher priority when explicitly returned. Existing custom provider logos and model settings are not silently replaced.

Usage costs are persisted with the request using the effective pricing snapshot at completion time. New v17 columns are additive and old rows retain zero/unknown detail values; later catalog updates do not recalculate historical costs. Canonical input/output totals include recognized detail subsets, and detail tokens are subtracted before specialized prices are applied.

Models.dev does not control CCProxy request rewriting. Thinking adapters remain local, typed, endpoint-bound transport policy in `src-tauri/src/ai/transport.rs`; provider templates or model names cannot authorize arbitrary request fields. Aggregator endpoints remain fail-closed unless an explicit safe override matches.

Source: https://github.com/anomalyco/models.dev
API snapshot: https://models.dev/catalog.json
