# Runtime Boundary Ownership Manifest (U-1)

> 状态：**只读审计 manifest**。本文只记录当前 worktree 的基线、逐文件 checksum 与计划分类，不修改任何源码、不改变 git index、不触碰数据或构建产物。
> 生成日期：2026-10-03；仓库：`/home/xc/dev/rust/chatspeed-plugin`（`chatspeed` 仓库的 **linked worktree**，git 公共目录为 `/home/xc/dev/rust/chatspeed/.git/worktrees/chatspeed-plugin`）。
> 关联计划：`work/quick/`（U-1 计划源）与 `work/plugin-phase-1-final-plan.md`；前置盘点：`work/runtime-ownership-inventory.md`。
> 写入范围：仅本文件。除本文外本次任务没有创建或修改任何文件。

## 0. 目的与边界

本 manifest 是逐文件迁移（U-2/U-3/U-5 及后续真实源码 ownership 迁移）的闸门。它回答四个问题：

1. 当前基线是什么（HEAD / index / worktree 差异）；
2. 每个 dirty source file 的 working checksum 与 baseline blob 是什么；
3. 每个文件属哪个 ownership 类别，计划 preserve / move / rewrite / delete 是哪种；
4. 哪些内容属于**保护边界**（prompt literals、sensitive regex、DB schema/migrations、既有测试），以及哪些目录必须排除。

本任务**没有**删除、移动或重写任何源码；所有 planned action 均为分类投影，需由后续已授权任务逐文件执行。

## 1. 当前基线快照

| 项 | 值 | 证据 |
|---|---|---|
| 分支 | `feature/plugin` | `git status -sb` → `## feature/plugin` |
| HEAD commit | `3b5ebc80cbd34d48dd5c56446d5cbb312df00a2f` | `git rev-parse HEAD` |
| HEAD 标题 | `fix(ai): mark stoppable trait async` | `git log --oneline -1` |
| HEAD~1 | `78007e34 Preserve runtime lifecycle changes` | `git log --oneline -5` |
| 计划中提到的历史基线 | 与 `3b5ebc80` 一致；`78007e34` 为前一提交；无 tag 指向 HEAD | `git tag --points-at HEAD` → 空 |
| index 状态 | **干净**：无 staged 变更 | `git diff --cached --stat` → 空 |
| 每个 modified 文件的 index blob | **等于** HEAD blob（全部 73 个 `1 .M N...`，`hH == hI`） | `git status --porcelain=v2` |
| worktree 变更 | **73 modified + 2 untracked** | `git status --porcelain=v1 \| wc -l` → 75 |
| 空白错误 | 无 | `git diff --check` → 退出码 0，无输出 |
| 非 Rust dirty 文件 | 无 | 75 条 status 全部为 `*.rs` |

### 1.1 index / staged 空集证据

- `git diff --cached --stat` 无输出 → 没有任何进入 index 的中间状态；因此每个文件的 `baseline blob` 同时是 **index blob 与 HEAD blob**（porcelain v2 中两列哈希逐文件相等，见 §3 表格来源）。
- 结论：本 worktree 的改动全部停留在 **worktree（unstaged）**，没有部分暂存导致的“混合 index”。

### 1.2 变更规模（`git diff --numstat` 聚合）

- 73 个 modified 文件；2 个 untracked（新增）文件。
- `#[cfg(test)]` → `#[cfg(all(test, not(feature = "desktop")))]` 转换：**移除 28，新增 28**，1:1 对应。
- 新增 `#[cfg(not(feature = "desktop"))]` 注解：**819** 处（仅统计本 worktree diff 新增行）。
- 删除的测试函数 / 测试模块：`-#[test]` = 0，`-fn test_` = 0，`-mod tests` = 0。

> 重要背景：cfg 切分**不是本次 worktree 独有**。HEAD 已含大量 `#[cfg(not(feature = "desktop"))]` 与 `#[cfg(all(test, not(feature = "desktop")))]`（例如 `src-tauri/src/workflow/react/client/http/server.rs`、`src-tauri/src/db/backup.rs`、`src-tauri/src/ccproxy/launcher.rs`、`src-tauri/src/ai/interaction/chat_completion.rs` 均为未 dirty 但已含 cfg 的文件）。本 dirty 树是“cfg 临时切分”的**延续**，必须在真实源码迁移中被清除，不能固化。

## 2. 已读取的 applicable guidance

- 根 `AGENTS.md`（ChatSpeed Root Rules）：最小改动、复用既有模式、不擅自 commit、LF、i18n、DB 谨慎、ccproxy 头部过滤、workflow/ccproxy/db constitution 优先。
- `src-tauri/AGENTS.md`：`Result`+`?`、`mod.rs` 只声明/再导出、DB 必须经 `MainStore`/`DbRuntime`、窗口事件用 emit。
- `src-tauri/src/ccproxy/CONSTITUTION.md`：单一 canonical 执行路径、`UnifiedRequest/Response` 适配边界、兼容层只能是 adapter、**路由顺序**、**头部过滤**。
- `src-tauri/src/db/CONSTITUTION.md`：只经 `MainStore`/`DbRuntime`、WAL + busy_timeout、reader `query_only`、维护路径 `checkpoint_for_maintenance`/`pause_runtime`/`resume_runtime`、`atomic_restore`。
- `src-tauri/src/workflow/react/CONSTITUTION.md`：后端权威、结构化状态优先、每个关注点单一 canonical path、兼容层只能是 adapter。

以上约束在 §5 的“保护边界”中逐条落地。

## 3. 逐文件 manifest

列含义：

- `baseline blob`：HEAD/index blob（sha1）；untracked 文件无 baseline。
- `worktree sha256`：当前工作区文件内容 sha256（本次审计计算）。
- `+/-`：`git diff --numstat` 的增/删行数。
- `cat`：ownership 类别 —— `RT`=runtime-owned、`SHARED`=两端共享、`CLIENT`=Tauri/客户端专属、`MIXED`=同一文件混合。
- `plan`：`move+rewrite`=真实迁移到 runtime 源码并去掉 cfg 临时门；`keep`=新增共享模块保持；`split+rewrite`=按 runtime/client 拆分；`preserve`=内容不得改动；`delete`=删除（**本次没有任何 delete 提议**，见 §6）。
- `prot`：保护边界标记 —— `regex`、`prompt`、`tests`、`schema`、`ccproxy-inv`、`shape`。

### 3.1 ccproxy（runtime-owned AI proxy engine）

| file | baseline blob | worktree sha256 | +/- | cat | plan | prot |
|---|---|---|---|---|---|---|
| `src-tauri/src/ccproxy/adapter/backend/mod.rs` | `acd6443d83b85edc146c9dcff10ba57c5ab1910a` | `aec2463e53057f9cfb3547fa86af10a0e4904bd1903ace8226859fe0813b68dc` | 14/2 | RT | move+rewrite | ccproxy-inv, tests |
| `src-tauri/src/ccproxy/adapter/input/mod.rs` | `9d3bb31cfb41173d60cb20b7f6d10ff2faccdc7d` | `65ce6e2da9dd2204da245a0d0369496fd372803abb4322d49da5113cb8159a1b` | 10/0 | RT | move+rewrite | ccproxy-inv |
| `src-tauri/src/ccproxy/adapter/mod.rs` | `61e64c887d2f6b257c478d17704fd9e57c627c11` | `afc6db261a9c25c47ce175103a721e25deedb0de7bd9273c1e0cee5cac8c371b` | 3/0 | RT | move+rewrite | ccproxy-inv |
| `src-tauri/src/ccproxy/helper/mod.rs` | `cc367b512548e8ac7de72ae4ad1a565fac87ea3e` | `a58be5a701b496e97e253b3bfe4f609dd1d645d092ccad3c90f907c4935b8514` | 6/0 | RT | move+rewrite | ccproxy-inv |
| `src-tauri/src/ccproxy/mod.rs` | `d0a3d0075e2d854f4fbd6603328d51f6af248a5d` | `878c7ed183b38c8588bb238d8d47ad0e958067c9dc073e8d2c0c2048c809f053` | 11/1 | RT | move+rewrite | ccproxy-inv |
| `src-tauri/src/ccproxy/types/mod.rs` | `bbbdb55fc75e7a90c91f8b21a4b252f026bdcee5` | `9afd907b8ce68d19c557eab2a579fbb7ff8aa2d8b10625de62b28b146f815489` | 3/0 | RT | move+rewrite | ccproxy-inv |
| `src-tauri/src/ccproxy/utils/mod.rs` | `d531ac5423ab0f8aee26f87608e9c06aceef62d7` | `fca15115b145ddce6dacb77d47651126178a6823425f3969185d12987b905cd1` | 1/0 | RT | move+rewrite | ccproxy-inv |

### 3.2 commands（runtime canonical + Tauri adapter）

| file | baseline blob | worktree sha256 | +/- | cat | plan | prot |
|---|---|---|---|---|---|---|
| `src-tauri/src/commands/workflow.rs` | `124ca47de3a5b977c340e2297954bc37f8eeea33` | `aed3562da6bec6fd6e80d410769628e449f8237b5a728d7ba7c3af37891a9e91` | 220/11 | MIXED | split+rewrite | tests |

- 本文件删除行仅为 import 合并与 `#[cfg(test)]` 门替换（详见 §6），未删除业务分支。

### 3.3 db（runtime-owned persistence）

| file | baseline blob | worktree sha256 | +/- | cat | plan | prot |
|---|---|---|---|---|---|---|
| `src-tauri/src/db/agent.rs` | `aeabdd53b9c233772e947027c81e8bfa93b98ea4` | `fa6a0227f5268e3efbd7ee85710785d9643d80abba6a0995c74d19668e403a42` | 12/1 | RT | move+rewrite | schema, tests |
| `src-tauri/src/db/config_transfer.rs` | `717c7b0c59161ca120ad50ff053a7c6b8dbf5c58` | `98d9cb61eee28764b459baee05377bab51063da0cd4629141db3eae0d226054d` | 29/2 | RT | move+rewrite | schema, tests |
| `src-tauri/src/db/main_store.rs` | `da6e02507b73a13b660fb1915bacd81c81f1a47b` | `59c1ab280a59796507253cf4238a97f51aa8a0d0945d4a7f2e66e66b285c54ad` | 21/2 | RT | move+rewrite | schema, tests |
| `src-tauri/src/db/mod.rs` | `b3f8ac118bd1969616fcd7bd5e721f59956f827f` | `35e5a70a46660ffd5ff55fe90e0223830647aa9b42e854c4bbbed5d0b7ffd06c` | 21/5 | RT | move+rewrite | schema |
| `src-tauri/src/db/note.rs` | `8c894c748591c2a76a9bb2d99dcaad9eb496be96` | `90907901b7f4a0ad83ed3b6d316e75ba236232c8ecf78939eb619db85ddecd30` | 9/0 | RT | move+rewrite | schema |
| `src-tauri/src/db/runtime.rs` | `2b612963b0334c54120d24476c38ce361d98daa3` | `4363848a6993a3eb2c66333d07c12570eddad507a442085fc1f1d4118098b538` | 26/9 | RT | move+rewrite | schema, tests |
| `src-tauri/src/db/types.rs` | `39af470955b27eaca4f78c8c388062d7a41afe9e` | `76ebf8af331b6d4d22781ff957fcabd6a5f9a9a131362e9aea06e2ca01552a07` | 5/1 | RT | move+rewrite | schema, tests |
| `src-tauri/src/db/workflow.rs` | `4c277b1af760665cc0c0244d37e7468d49bc6b3f` | `3aee245f698d7f728b20b897b8f1c0ea2f17fa51509f7ed54136ae5f46bd5730` | 13/3 | RT | move+rewrite | schema, tests |

- **DB schema/migrations 未改动**：`git status --porcelain -- src-tauri/src/db/sql/` 为空 → `db/sql/migrations/v1..vN` 无 diff（见 §4）。

### 3.4 libs / mcp

| file | baseline blob | worktree sha256 | +/- | cat | plan | prot |
|---|---|---|---|---|---|---|
| `src-tauri/src/libs/ai_temp.rs` | `4acae8369b8549780159df0f78ecda455ad10182` | `a7df078a1e1dde72a1d4fe65936975c8744941357207003c7b195151bd865eaa` | 25/3 | RT | move+rewrite | - |
| `src-tauri/src/libs/mod.rs` | `37a6a19bdc5b40b4e95fd828982876ba743e2a66` | `36f761bfeccee05b26d21a5403048ec0dfafcaa57ec02d86b8e894ad4b2e6d56` | 4/0 | RT | move+rewrite | - |
| `src-tauri/src/mcp/client/stdio.rs` | `94086b789aa64fb7c596041ef2ff8e17fd615a2c` | `fbb2b4d676d34650dca57dc0df7251122a7a50a8ac785f448446a0cbbc97268a` | 3/1 | RT | move+rewrite | tests |
| `src-tauri/src/mcp/error.rs` | `d62bf718932964e039432717f19db267938f007a` | `42c48b183be000b5310038aa782e266f9e223c02a4c793c2452dfb6299dc0e16` | 6/0 | RT | move+rewrite | - |
| `src-tauri/src/mcp/server/mod.rs` | `767db94aec8e861556b926da34f3c2f304785974` | `ff65571a6379f02c50f26e56fefedc30606dbe86cdc60df28900d12e0eaf8fd5` | 8/0 | RT | move+rewrite | - |

### 3.5 runtime data-command dispatcher

| file | baseline blob | worktree sha256 | +/- | cat | plan | prot |
|---|---|---|---|---|---|---|
| `src-tauri/src/runtime_data.rs` | `cbdfe3847a48cece905beb6955fa58cd73ec6244` | `6ba7e112f21f3046bd5205dcb0a9ed208c1bdb387238457f7a3d72c64c5dba8b` | 87/4 | MIXED | move+rewrite | shape, tests |

- 该文件经 `#[path]` 同时被 `chatspeed` 与 `runtime-backend` 编译（`runtime-backend/src/lib.rs` 中 `pub use data::runtime_data;`）。保留“历史 Tauri 响应形状（camelCase、opaque `Value`）”是兼容边界。

### 3.6 sensitive（runtime-owned PII filtering；regex 保护）

| file | baseline blob | worktree sha256 | +/- | cat | plan | prot |
|---|---|---|---|---|---|---|
| `src-tauri/src/sensitive/filters/common/credit_card.rs` | `62425238871d0cc4808480286aed22241e150530` | `514f34c8563317e365ff4d2f7eb9da2d79620a20079f0b4dd473274273a3dc9a` | 22/11 | RT | move+rewrite | regex, tests |
| `src-tauri/src/sensitive/filters/common/email.rs` | `dacd0959e4dce0722310607689b7bea9f9bd952d` | `88a0ec49ff893f9fc8cb9730b3a02bf0751928aa3db27eff25f1914ae94489a7` | 24/13 | RT | move+rewrite | regex, tests |
| `src-tauri/src/sensitive/filters/common/international_credit_card.rs` | `fbbe32b5531539d24044dac6e75b268ad19699ae` | `e96ebc623c20150ae01f0c33a09b4dc5d5691cc1512b5f3d74086d456af82257` | 29/18 | RT | move+rewrite | regex, tests |
| `src-tauri/src/sensitive/filters/common/ip_address.rs` | `f3003ecc345cef73d701591ec218b138479090ce` | `8e2b0c91b732bcd6b7a4705bfbeeaa87cd719f5784e3e8a7d0b428bb5d93c197` | 26/15 | RT | move+rewrite | regex, tests |
| `src-tauri/src/sensitive/filters/localized/en/address.rs` | `7f174a794627d1cf49437559a43a15e0109d8bb2` | `1c155f6ef0629ba85ea03f736c3b4b9ca8c062be6feef6b24c8a9ad98acd9cc4` | 24/12 | RT | move+rewrite | regex |
| `src-tauri/src/sensitive/filters/localized/en/company.rs` | `639bc4511b9b908d404a99d9e418135d03b33751` | `b04c3149a78181dc905f47ef895dbbe9b9d72e5908ca249281a17e4401d7d77a` | 24/12 | RT | move+rewrite | regex |
| `src-tauri/src/sensitive/filters/localized/en/financial.rs` | `c262ff3f7c068729bd5c35480727fca3b8067ca8` | `817f6bcd10137b3a2b3981e9c546669e93b56c8d9c04014f8297c307ce733632` | 29/17 | RT | move+rewrite | regex |
| `src-tauri/src/sensitive/filters/localized/en/mobile.rs` | `4508f3094d1e692c63284be3e553a5bc2bd985b2` | `158725ccc341b08476091b3f8d1481136b915570010b60a0d5060bda1b2288a1` | 21/10 | RT | move+rewrite | regex |
| `src-tauri/src/sensitive/filters/localized/en/name.rs` | `ce239fded50aea3a64422630f2e17c0f621444bc` | `571cc80085473fe63068113a268d4ed7642061eb8e050a74cbc1fa3023414b32` | 39/27 | RT | move+rewrite | regex, tests |
| `src-tauri/src/sensitive/filters/localized/en/project.rs` | `af85e34293720415062e6b3a4a75ef0c71e77e35` | `5735c26dc473bc8064b2da6938b32c16bd807ca2be4b243ed207b0843ad0e197` | 18/7 | RT | move+rewrite | regex |
| `src-tauri/src/sensitive/filters/localized/en/social.rs` | `bb71feea1d0fa8d3178db482cf24a3f92c12c9f8` | `8ac3fa16164e15d3d0ed475cca914853ad8346151c5369d395afb05080a764f3` | 18/7 | RT | move+rewrite | regex |
| `src-tauri/src/sensitive/filters/localized/en/ssn.rs` | `4f48fb63f2b347595c725c53119c9a9eb5cbdca2` | `eed2fd94380b55cda7f569fbea521880cf337ceea6341cd3755cafdd8c4cb1ed` | 23/12 | RT | move+rewrite | regex, tests |
| `src-tauri/src/sensitive/filters/localized/zh/address.rs` | `db6fcd65eb5b66d0af02176160d5f52940960310` | `b586474bcf9b64d1059b8aeb2f0644b3afcedf3ae644ae63a02f53cc1aa48c1f` | 27/13 | RT | move+rewrite | regex |
| `src-tauri/src/sensitive/filters/localized/zh/company.rs` | `fa193440ae2f3534a47dd3c9a942a9f22257ae43` | `c05dd6df5e0a88afb488d5ab44f6d1d6bfec786fb5887465d400b81da2b50c3c` | 28/16 | RT | move+rewrite | regex |
| `src-tauri/src/sensitive/filters/localized/zh/financial.rs` | `a7ef38bbe3be15312e39416ca7fc98c8ec700c03` | `482f21949bec34aafff5ff32873ba44ecbdaf05cfe515b9b9cf20f650968253c` | 40/27 | RT | move+rewrite | regex |
| `src-tauri/src/sensitive/filters/localized/zh/id_card.rs` | `0f7e28dbd98a4dfa27eb3e0331eb10713f9f4ed6` | `ed5095eb139dca05d15792ba85fd097f6ac15b8f9a4af7aa6b2c55fb4cfc0c47` | 22/11 | RT | move+rewrite | regex, tests |
| `src-tauri/src/sensitive/filters/localized/zh/landline.rs` | `f65be4b217563f99fc0a0babea8d528673a1be03` | `bcb5e330480f4a1775d7168b8653d37967b576b8c0b745f069a4b85b9e277207` | 23/12 | RT | move+rewrite | regex, tests |
| `src-tauri/src/sensitive/filters/localized/zh/mobile.rs` | `d5e8113861d834ea2f19d65dfcac346f9484fb06` | `9828e9947e16fb02bdb926364a9a99500d25cdc3b2da2a1abe4877d053a6e9ac` | 22/11 | RT | move+rewrite | regex, tests |
| `src-tauri/src/sensitive/filters/localized/zh/name.rs` | `c3a83e715fee1a2f983c866c3a8ef82e373b665c` | `6eb0a28006943db6036e52f849f220f82fd59defbc9220d6ab64f0f7663930fb` | 52/40 | RT | move+rewrite | regex, tests |
| `src-tauri/src/sensitive/filters/localized/zh/project.rs` | `e008bb88436bfcdd8dfa11fe65259e8d484d8f04` | `314fc8cb6b8514ed5c779911610cb5b1c166965dde07cf705b70c4fb487de79f` | 21/10 | RT | move+rewrite | regex |
| `src-tauri/src/sensitive/filters/localized/zh/social.rs` | `020335c8496db75886661aca5a8279e90975109f` | `aa9739ee6184065acd8656348664899986f4b522a8dd3c5c94610f5f90b2a842` | 31/19 | RT | move+rewrite | regex |
| `src-tauri/src/sensitive/filters/localized/zh/unionpay.rs` | `cc84464f5430b2309e07306e1300a524ad558a5f` | `4698dd419cf68a56cb862be066ef46bc631498d04e2cf71255bca10fbbf10cf6` | 27/16 | RT | move+rewrite | regex, tests |
| `src-tauri/src/sensitive/manager.rs` | `a0bbee38030a45f617a8b4e849792e47eeef0574` | `9cab80587c565a64dee504fe39a7a129735441b0d19b034cb0a2af99b5320199` | 22/2 | RT | move+rewrite | regex, tests |
| `src-tauri/src/sensitive/mod.rs` | `fddc1f2d50eba02e8df781682b512f2f79816074` | `a4aa3d500d3816d5e39225fed20647fab64a50cf4f379da91fc313b823b06963` | 3/0 | RT | move+rewrite | regex |
| `src-tauri/src/sensitive/traits.rs` | `26e7844dbea7b835a38240c325ce88fcd9ebafc6` | `38d3471bf3e3f7ec0e5c6de6f2870d433f614e512a93b06ae3176eceef357c00` | 9/1 | RT | move+rewrite | regex, tests |

### 3.7 tools（runtime-owned core tools + 共享审批 DTO）

| file | baseline blob | worktree sha256 | +/- | cat | plan | prot |
|---|---|---|---|---|---|---|
| `src-tauri/src/tools/constants.rs` | `131e6c6a21f5dca2abb659ec3b0e6dde97d4a924` | `d4ff8be510e82cf823f266d0399339e82fcf9797032545256ba917bf7bb62a3f` | 11/0 | RT | move+rewrite | - |
| `src-tauri/src/tools/fs.rs` | `997c213cf11bea28ee0c42a243d772c280ce58c8` | `dd5f17e417799d89599bcd3f500b3c4ef833c52c1920c282a4fba86fdd23d05f` | 9/0 | RT | move+rewrite | - |
| `src-tauri/src/tools/mod.rs` | `2b6b3974590ecc468ae9762e657cb3a34a8b9d87` | `4d3b6da717b4a8a780abb20138439b91022fd22bdecdffe2023152e0d52601ea` | 27/0 | RT | move+rewrite | - |
| `src-tauri/src/tools/sandbox/mod.rs` | `3cbb4c3d07012876bc56e81e1ade8cce1903c491` | `a181bbd621f71190e99ef37496069e729336295046251346692f87b122582174` | 9/1 | RT | move+rewrite | - |
| `src-tauri/src/tools/sandbox/types.rs` | `5f03a505497f998523a7420067c5c7e4e5a7308c` | `9f41e1e7560bd9b023c3555c6365899555c8f73015f5ce29602c09e8f72052bf` | 4/0 | RT | move+rewrite | - |
| `src-tauri/src/tools/search.rs` | `670721280a2f50c6efc57cf1f0b9e81672f56276` | `2e0b645ae0aad66930b776bdf66ff4419eaf8645d69c54fc8b3001c147ff10d6` | 1/0 | RT | move+rewrite | - |
| `src-tauri/src/tools/shell.rs` | `f101358219c988e79d9d8d6367c9335e3df8f1f2` | `b4fd632dcea8fbe1678c77477a28b290a77745844cabb4114186cef7a08b9b8b` | 3/60 | RT | move+rewrite | shape |
| `src-tauri/src/tools/tool_manager.rs` | `19d9aacf994e974c6b54b0fa4ffe988b97bfc74f` | `a1508b14644fdf0d9d284de5a5ed449da25801711e3a2dbd3f573096be5bb1c9` | 6/1 | RT | move+rewrite | tests |
| `src-tauri/src/tools/shell_policy.rs` | **(untracked，无 baseline)** | `a608c90f0aa044dcb19e9fc35dd8f771a115fa84791070f650655af8080b86c0` | +67/0（新增） | SHARED | keep | shape |

- `tools/shell.rs` 的 60 行删除是 `ShellDecision`/`ShellPolicyRule` **迁移**到新文件 `tools/shell_policy.rs`（`serde` impl 原样搬移），不是删除；`tools/mod.rs` 增加 `mod shell_policy;` 与 `pub use shell_policy::*;`。

### 3.8 workflow/automation

| file | baseline blob | worktree sha256 | +/- | cat | plan | prot |
|---|---|---|---|---|---|---|
| `src-tauri/src/workflow/automation/application.rs` | `e7c962fbf0ca3deceb6c52525c8b6ff636344349` | `b1b56c0d8c0ac0dbf4a83b07dc478142fdc526e78364167ab9911a5752ffd132` | 0/63 | RT | move+rewrite | ⚠ 见 §6.1 |
| `src-tauri/src/workflow/automation/errors.rs` | `295d76b983975a676fc88e86c94f3bc6e761a403` | `c1ad86b433c6868bfeb680226f5fabf38f86e23026c964c145d4d831b028d9fb` | 14/0 | RT | move+rewrite | - |
| `src-tauri/src/workflow/automation/mod.rs` | `3fc4bb3d129a3cfce343291592ae3e76f444ae2e` | `8dc76713d647b641f16a9d5448211181625865a2b878b01be2a2f09b2ed39798` | 5/0 | RT | move+rewrite | - |
| `src-tauri/src/workflow/automation/types.rs` | `dc49624c820d6bf21353ef7a18342839cd07c450` | `64738e3befb773a194745653b920c1cf05112f0178aaaa83d8a9b3bb1e12b788` | 7/0 | MIXED | split+rewrite | - |

### 3.9 workflow/react

| file | baseline blob | worktree sha256 | +/- | cat | plan | prot |
|---|---|---|---|---|---|---|
| `src-tauri/src/workflow/react/application.rs` | `bd3cb0158f38449468308c27fbfa377c4d4bb1a9` | `e9ca633af15d0047ca8bf4cda98038d16d58c8878928b10f1c3ec55224dec778` | 35/114 | RT | move+rewrite | ⚠ 见 §6.2 |
| `src-tauri/src/workflow/react/client/mod.rs` | `219ffda35d59aea7dc04a86ed2389b066a100906` | `e7b2908db7ebe6c41ca2705e053e7057362cdb7cdaf9b78a9363175f42c3745e` | 5/0 | MIXED | split+rewrite | - |
| `src-tauri/src/workflow/react/client/tauri/mod.rs` | `269fe7ed1f1b4c1035df368f813026cee8c38a41` | `2c85467fef3e90e612ade5ecec45fefb256308fcb7576491772a77c0f46fe632` | 10/1 | CLIENT | preserve | - |
| `src-tauri/src/workflow/react/constants.rs` | `ad49f8d5d394f86d4a01e71c2f51015372034e81` | `185a64700df0f2a46ad3f705ed419ab1de5cc0541eb37fd4e7fe2981ce8f1771` | 1/0 | RT | move+rewrite | - |
| `src-tauri/src/workflow/react/decision.rs` | `6d2bd4203abab2eeb5e221c210cfa9398e57f7b9` | `cb0d626166e98b00c127c8449af004c002396412b8f4d30697f1e33bfdf985e9` | 2/2 | RT | move+rewrite | - |
| `src-tauri/src/workflow/react/dispatcher.rs` | `a53449bc4dce86f28df59b362581064d92e36f1c` | `b2782841a73b68c2c52fca1b99dfca3f90b514f506a7aa4acae0f20013008c33` | 30/1 | RT | move+rewrite | tests |
| `src-tauri/src/workflow/react/gateway.rs` | `7a0a71b58b6980cde878ab3e162bb0b3107bb108` | `edf1580597f581383e6af8652170648b76e9412ac1c36cdc6c5f0faa09a36600` | 4/0 | CLIENT | preserve | - |
| `src-tauri/src/workflow/react/idle_sleep.rs` | `8632dc88fa28076e22bfc71bcd4e68d574d73f50` | `848638fd96fb4bacbfdc31dca1c10568f60135466e3873e8907d674a839f8fef` | 10/1 | RT | move+rewrite | tests |
| `src-tauri/src/workflow/react/mod.rs` | `b6d7239612b9a129d38eaade498e891d6f12a91d` | `920113c27b7c8eb70dbfd914e5988dc53eac371557ce937d759aed6192d5094a` | 27/1 | RT | move+rewrite | tests |
| `src-tauri/src/workflow/react/prompts.rs` | `3f548ba97827dba72d6070dba1c653665356ab26` | `636aadc1a59ae3a76d6a740f46203d2844e10a92b2280d653f46bed370800c97` | 37/1 | RT | move+rewrite | prompt, tests |
| `src-tauri/src/workflow/react/replay.rs` | `d69f26d05456d2bf15dd949ee3e4b91fd1bbfbfc` | `410a33479f9832dd4a1ead55b7831c6d189cf5c7d7a475c2b31c9e9cc3873b3e` | 10/1 | RT | move+rewrite | tests |
| `src-tauri/src/workflow/react/security.rs` | `35a17a94994afdc8e81a39dca30ba81d934c89c9` | `79f785d97c78b0d580e9ccc730d1ab89a33b6646f44b6e99a837727807dd1f24` | 17/2 | RT | move+rewrite | tests |
| `src-tauri/src/workflow/react/sinks.rs` | `35e7dc86a6239a2638f59e399b8bdf42e8037ef8` | `1fc40fd5b7b4669ff01d9e7ea85179398bebfca94ab5c7b3a322dc62fc2c39b6` | 4/0 | RT | move+rewrite | - |
| `src-tauri/src/workflow/react/types.rs` | `9a43f00cdba833d426543236e3637ac9faf8e96d` | `c1b4c656a899d98addf1c0bae314bc02a4e244c32161d0d586a76d4493433b8e` | 20/1 | RT | move+rewrite | - |
| `src-tauri/src/workflow/react/application_types.rs` | **(untracked，无 baseline)** | `abd9b7561cb0a037b8c8a6610d2f0d96d69372f956c3cc0d6713fd359248cc50` | +130/0（新增） | SHARED | keep | shape |

- `workflow/react/application.rs` 删除的 `ApplicationErrorKind`、`ApplicationError`（含 impl）、`WorkflowEventsQuery` 已**迁移**到新文件 `workflow/react/application_types.rs`（`mod.rs` 中 `pub mod application_types;`，`application.rs` 中 `pub use super::application_types::{...}`），不是删除。

## 4. 保护边界（不得改动）

### 4.1 DB schema / migrations

- `src-tauri/src/db/sql/**`（含 `migrations/v1.rs`–`vN.rs`）**零 diff**：`git status --porcelain -- src-tauri/src/db/sql/` 输出为空。
- 结论：本次 worktree 未触碰 schema、migration、表结构或数据。后续迁移必须保持该零改动，任何 schema 变更需单独批准。
- 仍生效的 DB 约束（`src-tauri/src/db/CONSTITUTION.md`）：仅经 `MainStore`/`DbRuntime`、WAL + `busy_timeout`、reader `query_only`、写只走 runtime writer、维护路径 `checkpoint_for_maintenance`/`pause_runtime`/`resume_runtime`、`atomic_restore`。

### 4.2 Prompt literals

- `src-tauri/src/workflow/react/prompts.rs`（`prompt` 标记）：本 diff 仅新增/移动常量与把 `#[cfg(test)]` 改为 `#[cfg(all(test, not(feature = "desktop")))]`；已有 prompt 字符串字面量保持不变。
- 保护要求：真实迁移时 prompt 文本字面量必须逐字节保留，禁止改写、裁剪或 i18n 化。

### 4.3 Sensitive regex

- 全部 `src-tauri/src/sensitive/**`（`regex` 标记）：抽查 `filters/common/credit_card.rs` 显示 regex 字面量（如 `r#"\b(?:4\d{3}|5[1-5]\d{2})[-\s]?\d{4}[-\s]?\d{4}[-\s]?\d{4}\b"#`）**原样保留**，仅被移入 `#[cfg(not(feature = "desktop"))]` 块。
- 保护要求：所有 regex 模式字符串不得修改；迁移只能改变其编译位置，不得改变模式或语义。
- ⚠ 副作用见 §6.3（desktop 构建下该层被 stub）。

### 4.4 既有测试

- 本 worktree **未删除任何测试**：移除 `#[cfg(test)]` 28 处、新增 `#[cfg(all(test, not(feature = "desktop")))]` 28 处（1:1）；`-#[test]`=0、`-fn test_`=0、`-mod tests`=0。
- ⚠ 但所有 28 个测试模块被**重新门控**为“仅在非 desktop（runtime-backend）测试目标下编译”。这意味着 `chatspeed`（desktop）测试运行不再覆盖这些用例。见 §5、§6.3。
- 覆盖面统计：当前工作区含 `#[cfg(all(test, not(feature = "desktop")))]` 的文件共 31 个（其中 30 个在本 dirty 集内，另 1 个 `workflow/react/client/http/server.rs` 已在 HEAD 提交）。命中 dirty 文件的清单：
  `commands/workflow.rs`、`runtime_data.rs`、`ccproxy/adapter/backend/mod.rs`、`db/{agent,config_transfer,main_store,runtime,types,workflow}.rs`、`mcp/client/stdio.rs`、`tools/tool_manager.rs`、`sensitive/traits.rs`、`sensitive/filters/common/{credit_card,email,international_credit_card,ip_address}.rs`、`sensitive/filters/localized/en/{name,ssn}.rs`、`sensitive/filters/localized/zh/{id_card,landline,mobile,name,unionpay}.rs`、`workflow/react/{application,dispatcher,idle_sleep,mod,prompts,replay,security}.rs`。

## 5. 排除集（必须排除，不得读取/输出/纳入 manifest）

以下路径为数据、生成物或构建产物，本任务**未读取、未修改、未纳入 checksum**：

| 排除项 | 依据 | 说明 |
|---|---|---|
| `dev_data/` | `.gitignore` `dev_data*/` | 运行期用户数据；`.csignore` 允许工具访问，但审计基线必须排除 |
| `mcp_sessions/`、`src-tauri/mcp_sessions/` | 运行期 MCP 会话数据 | 不读取、不哈希 |
| `target/`、`src-tauri/target/` | `.gitignore` `**/target/*`、`src-tauri/.gitignore` `/target/` | 构建产物（`src-tauri/target` 被安全策略拒绝访问，未触碰） |
| `dist/`、`dist-ssr/` | `.gitignore` | 前端构建产物 |
| `src-tauri/gen/`、`gen/schemas` | `src-tauri/.gitignore` | Tauri 生成 schema |
| `node_modules/` | `.gitignore` | 依赖 |
| `.cs/` | `.gitignore` | 工具会话目录 |
| `work/*smoke*`、`work/msb/`、`work/agent-cli-harbor-smoke/` 等 smoke 输出 | 计划排除 | smoke/评测输出，不纳入源码基线 |
| 任何 `*.env`、`.env*`、密钥文件 | 安全约束 | **不读取、不输出** |

本任务未读取任何 secrets，未访问任何运行期数据或构建产物。

## 6. 异常 / 非纯 cfg 变更（需父 agent 确认）

以下项是**超出纯 cfg 门控**的行为变更，均已核查，属 Rust 源码、可分离，按要求“报告而不停止”。整合前需父 agent 明确接受。

### 6.1 automation legacy compat 适配器被移除

`src-tauri/src/workflow/automation/application.rs`（0 增 / 63 删）移除了三个 legacy camelCase 兼容适配器：

- `AutomationApplicationService::list_rows`（INV-9 只读 legacy 行读）
- `AutomationApplicationService::compat_save`（INV-2/INV-9 legacy 编辑器 save）
- 自由函数 `request_to_spec`

证据：`git diff` 明确删除上述函数；`automation/mod.rs` 以 `#[cfg(not(feature = "desktop"))] pub mod application;` 表明 desktop 改为经 control plane 处理 automation，因此这些 desktop 直连适配器被移除。**这是行为收缩（desktop 不再本地兼容路径），不是文件删除**；需确认是否符合批准范围。

### 6.2 workflow/react 共享类型抽取

`workflow/react/application.rs` 的 114 行删除为 `ApplicationErrorKind`/`ApplicationError`/`WorkflowEventsQuery` **迁移**至新文件 `workflow/react/application_types.rs`（已核对新文件含同名符号）。属 move，非 delete。

### 6.3 desktop 构建下的功能 stub（风险）

- sensitive 层：`CreditCardFilter::new` 在 `desktop` feature 下返回 `Ok(Self {})`，`filter`/`supported_languages` 等方法被 `#[cfg(not(feature = "desktop"))]` 门控掉；`FilterManager::filter_text` 同样被门控。
- 含义：desktop 进程不再在本地执行 PII 过滤（交由 runtime），但 **desktop 的敏感过滤测试一并被门控为不编译**。
- 风险：若迁移未完成而 desktop 已按 stub 行为发布，将产生功能回归。必须由 U-2/U-3 以真实 runtime 路径补齐，cfg 只能临时。

### 6.4 runtime-backend 仍以 `#[path]` 复用 `../../src`

- `src-tauri/runtime-backend/src/lib.rs` 通过 `#[path = "../../src/..."]` 复用 desktop 源码（`db/mod.rs`、`sensitive/mod.rs`、`mcp/mod.rs`、`ai/mod.rs`、`tools/mod.rs`、`capability/mod.rs`、`ccproxy/mod.rs`、`workflow/mod.rs` 等）并以 `#![allow(...)]` 抑制告警。
- 依据计划：这是**临时证明边界**，不是真实 ownership 迁移；后续必须做真实源码迁移（本 manifest 的 `move+rewrite` 分类即为此预留）。
- `runtime-core` 仍在 workspace members（`src-tauri/Cargo.toml`：`[".", "contracts", "cli", "runtime", "runtime-core", "runtime-backend", "runtime-client"]`）。

## 7. 停止条件（stop conditions）

出现下列任一情况必须停止并回报，不得自行扩展：

1. 发现计划未涵盖的**非 Rust** 用户改动（本次无：75 条 status 全部为 `*.rs`）。
2. 发现无法与 runtime 迁移分离的用户独立业务 diff（本次无：已识别 §6.1 为可分离项）。
3. 任何需要改动 `db/sql/migrations`、prompt 字面量、sensitive regex 模式或删除既有测试的实现。
4. 任何需要修改 index/staging、reset、commit、或触碰 §5 排除集的实现。
5. runtime 真实迁移无法在不共享 DB、不改变协议 major、不引入 OS daemon 的前提下完成（对应 `work/runtime-ownership-inventory.md` 的闸门）。

## 8. 本次验证清单

| 检查 | 命令 | 结果 |
|---|---|---|
| HEAD | `git rev-parse HEAD` | `3b5ebc80cbd34d48dd5c56446d5cbb312df00a2f` |
| 分支 | `git status -sb` | `feature/plugin` |
| index 为空 | `git diff --cached --stat` | 空（无 staged） |
| index==HEAD blob | `git status --porcelain=v2` | 73 × `1 .M`，两列哈希相等 |
| dirty 计数 | `git status --porcelain \| wc -l` | 75（73 M + 2 ??） |
| 空白错误 | `git diff --check` | 退出码 0，无输出 |
| schema 未动 | `git status --porcelain -- src-tauri/src/db/sql/` | 空 |
| 测试未删 | `git diff -U0 \| grep -cE '^-(#\[test\]\|fn test_\|mod tests)'` | 0 |
| 测试门控 1:1 | 移除 `#[cfg(test)]`=28，新增 `#[cfg(all(test,...))]`=28 | 28/28 |
| cfg 注解规模 | `git diff -U0 \| grep -c '^+.*cfg(not(feature = "desktop"))'` | 819 |
| 逐文件 checksum | `sha256sum <75 files>` | 已记录于 §3 |
| baseline blob | `git status --porcelain=v2` header/index hash | 已记录于 §3 |
| manifest 覆盖 | 见 §8.1 | 75/75 |

### 8.1 manifest 逐文件覆盖核对

- §3 表格行数 = 73（modified）+ 2（untracked）= **75**，与 `git status --porcelain` 的 75 条一一对应。
- 每个 modified 行含 baseline blob（index==HEAD）与 worktree sha256；两个 untracked 行标注“无 baseline”，仅含 worktree sha256。
- 无遗漏、无多余项。

## 9. Handoff 结论（U-1）

- 基线已锁定：`feature/plugin` @ `3b5ebc80`，index 干净，worktree 73 M + 2 ??，无空白错误，无非 Rust 改动，schema/migrations 零 diff，测试零删除。
- 逐文件 checksum 与 baseline blob 已记录，覆盖 75/75。
- 保护边界已明确：prompt literals 原样、sensitive regex 模式原样、DB schema/migrations 未触碰、既有测试保留但被重新门控为非 desktop。
- 排除集已声明（`dev_data`/`mcp_sessions`/`target`/`dist`/`gen`/`node_modules`/smoke 输出/secrets）且未被触碰。
- 待父 agent 裁决项：§6.1（automation legacy compat 移除）、§6.3（desktop stub 风险）、§6.4（`#[path]` 复用仍为临时）。
- 后续动作：U-2/U-3 需按 §3 的 `move+rewrite` 分类执行真实源码 ownership 迁移并移除 cfg 临时门；不得以本 worktree 的 cfg 状态作为完成态。

## 10. AC-8 — Desktop loopback Web MCP provider (as-built record + evidence)

> Added by the AC-8 implementation unit. This section is the developer-facing
> (English) record the user asked for: what shipped, why it stays on the
> canonical path, and the explicit TODO for a later independent external Web MCP.

### 10.1 What shipped

The desktop now hosts the two fixed web tools (`web_fetch`, `web_search`) as a
dedicated MCP server on an ephemeral `127.0.0.1:0` port and registers it with the
runtime over the control plane. The runtime reaches it as an ordinary
streamable-HTTP MCP server through the canonical `ToolManager` MCP client path.
The previous client-pull capability bridge is no longer a main path: the runtime
no longer registers `ClientBridgeWebTool`, and `ClientBridgeRegistry::enqueue`
is not on the fixed-tool call path.

Hard boundaries preserved:

- Registration body carries **only** `port`; the runtime derives
  `http://127.0.0.1:<port>/mcp` itself. No arbitrary host, path, query or
  endpoint input exists.
- The provider proof token, client id, lease id and instance id travel in
  dedicated headers (`X-Web-Mcp-*`) and the MCP `Authorization` header - never a
  URL, query string, body or log line.
- The proof token is random, short-lived, in memory only, and bound to
  instance + client + lease. It is never persisted and never serialized.
- One provider slot. A second live desktop is rejected with a structured
  `conflict` (HTTP 409); it is never silently replaced or rerouted.
- Same-lease re-registration is idempotent; a new port on the same lease is an
  explicit generation replacement.
- The slot is dropped when its lease is released or expires (control-plane
  sweeper), and the provider client is built with a single bounded connect
  attempt, so a dead or released provider fails closed as `unavailable` instead
  of silently reconnecting.
- Tool execution reuses the existing `WebBridgeDispatcher` body (runtime-config
  source plus the shared strict capability schema), validated **before** the
  config is read. No second DB, `ChatState`, executor or generic RPC.
- The reserved server name `chatspeed_web` and the exact aliases `web_fetch` /
  `web_search` cannot be occupied by an ordinary, user-configured MCP server.

### 10.2 Files (implementation chain)

- `src-tauri/contracts/src/web_mcp.rs` (+ `lib.rs` exports): port-only DTOs,
  response/status/error types, header + route constants, canonical loopback
  endpoint builder, port validation.
- `src-tauri/runtime-client/src/lib.rs`: typed
  `register_web_mcp_provider` / `unregister_web_mcp_provider` /
  `web_mcp_provider_status` (header-only proof).
- `src-tauri/src/workflow/react/client/http/web_mcp_commands.rs` (+ `mod.rs`,
  `server.rs` wiring): `RuntimeWebMcpPlane` + `WebProviderLeaseCheck` traits,
  bearer + lease + proof routes, control-plane sweeper integration.
- `src-tauri/runtime-backend/src/web_provider.rs` (+ `lib.rs`): single-slot,
  lease-bound, generation-fenced registry; `ToolManagerProviderInstaller`
  installs/removes the provider through the canonical `ToolManager` MCP path.
- `src-tauri/src/tools/tool_manager.rs`, `src/mcp/client/streamable_http.rs`:
  reserved server name guard, exact web alias allocation for the reserved
  provider, `register_web_mcp_provider` with a bounded (single-attempt) client.
- `src-tauri/src/runtime_web_mcp_provider.rs` (+ `src/lib.rs` setup,
  `src/runtime_client.rs` lifecycle): loopback rmcp `StreamableHttpService` +
  `LocalSessionManager` handler, header/origin proof, register on connect /
  unregister before lease release.
- `src-tauri/runtime/src/lib.rs`: passes the provider plane into the runtime
  control plane.

### 10.3 Evidence (focused tests actually run)

- `cargo test -p chatspeed-contracts` - 39 passed (includes web_mcp DTO tests).
- `cargo test -p chatspeed-runtime-client` - 54 passed (includes header-only
  provider registration/status tests).
- `cargo test -p chatspeed-runtime-backend -- provider` - 32 passed (registry
  semantics, reserved-name guard, exact aliases, control-plane routes).
- `cargo test -p chatspeed-runtime-backend web_provider_is_registered_through_the_canonical_mcp_path`
  - passed (real rmcp end-to-end + `ToolManager` list/alias/call path).
- `cargo test -p chatspeed --lib runtime_web_mcp_provider` - 3 passed.
- `cargo check` (desktop) and `cargo check -p chatspeed-runtime` /
  `-p chatspeed-runtime-backend` - clean (only pre-existing warnings).

### 10.4 TODO — independent external Web MCP (out of scope for AC-8)

AC-8 deliberately binds the provider to the desktop process and its live runtime
lease. A later, separate unit may add an **external** Web MCP:

- an independently launched MCP process (or a user-configured streamable-HTTP
  server) that is **not** tied to the desktop lease, so it survives desktop
  restarts;
- an explicit trust/onboarding model distinct from the lease-bound proof token
  (e.g. user-approved server descriptor with its own credential storage);
- a decision on whether the single-slot reservation should stay exclusive to the
  desktop provider or accept multiple external servers under distinct reserved
  names;
- refresh/upgrade semantics for an external provider that the runtime does not
  host.

Until such a unit exists, the desktop Web MCP provider remains the only owner of
the reserved server name and the exact `web_fetch` / `web_search` aliases.