# SQLite 数据库运行时改造计划草案

## 核心决策
- 分支：实施第一步从当前基线创建 `refactor/sqlite-db-runtime`；若工作区不干净或分支同名且指向不明，停止并询问用户。
- 不新增连接池依赖：基于现有 Tokio channel + dedicated OS threads 实现 1 writer + 默认 2 reader workers；避免把同步 rusqlite 放在 Tokio worker 上。
- 普通文件数据库：唯一 writer Connection；reader 各自独占只读/query_only Connection。`:memory:` 使用单连接退化模式，避免不同 Connection 得到不同数据库。
- DbRuntime 提供泛型 owned job：write/read job 在 worker 内执行，oneshot 返回 `Result<T, StoreError>`；复杂事务完整进入 writer job。
- 两类有界写入口：durable（await + 错误传播）与 telemetry（Drop 可 try_send，批量事务，溢出必须告警/计数）；最终仍只有一个 writer。
- 所有生产数据库调用迁移到 DbRuntime；最终 MainStore 改为 `Arc<MainStore>` + 内部 ConfigCache 锁，移除公开 conn 和外层 `Arc<RwLock<MainStore>>`。
- backup/restore/migration 使用 maintenance gate：阻止新任务、排空、关闭连接、操作文件、重开并刷新 cache。
- 先修统计：DATE(request_at,'localtime') WHERE 改 UTC [start,end) 原列范围；轮询禁止重入/窗口不可见暂停；趋势 7/30 日改一次范围 IPC；Title Bar 使用窄化统计 API。
- 不先加表达式索引或汇总表；通过 EXPLAIN 和 10万/50万/100万基准再决定普通复合索引/rollup，若需要 schema migration 再提交窄化变更。

## 主要目标文件
- 新增 `src-tauri/src/db/runtime.rs`
- `src-tauri/src/db/{mod.rs,error.rs,main_store.rs,ccproxy.rs,workflow.rs,config.rs,chat.rs,note.rs,agent.rs,automation.rs,mcp.rs,proxy_group.rs}`
- `src-tauri/src/db/sql/migrations/{mod.rs,manager.rs}`（仅基准证明需索引时增加 v12）
- `src-tauri/src/lib.rs`, `src-tauri/src/commands/{ccproxy.rs,workflow.rs,message.rs,setting.rs,note.rs,proxy_group.rs,...}`
- workflow runtime callsites: `workflow/react/{sinks.rs,orchestrator.rs,context.rs,replay.rs,engine.rs}`
- CCProxy: handler stat callsites + `helper/stat_guard.rs`
- frontend: `src/views/{ProxySwitcher.vue,Workflow.vue}`, `src/components/setting/ProxyStats.vue`
- guidance: `src-tauri/AGENTS.md` 更新旧“multiple connections”禁令为“one controlled writer + bounded readers”。

## 验证重点
- git branch/status baseline
- runtime unit tests: FIFO, reader concurrency, writer/read overlap, ID/error, panic/cancel/queue-full/shutdown, in-memory fallback
- transaction and read-after-write tests
- config cache DB failure consistency
- backup/restore tests with queued/in-flight jobs
- DB/workflow/ccproxy focused cargo tests, full cargo test, pnpm test:workflow, pnpm build
- CCProxy routing suite only if handler behavior touched beyond stat submission; otherwise focused stat tests + manual API smoke
- EXPLAIN QUERY PLAN and seeded data benchmarks; Windows manual/CI evidence required before calling UI freeze solved
