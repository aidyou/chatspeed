# 模型目录配置说明

本文档说明 `model_catalog.json` 的用途、数据结构、字段含义、匹配规则和维护方式。

- **配置文件**：`src-tauri/assets/model_catalog/model_catalog.json`
- **说明文档**：`src-tauri/assets/model_catalog/model_catalog.md`（本文档）
- **设计背景**：`docs/model-catalog-design.md`
- **解析实现**：`src-tauri/src/ai/model_catalog.rs`

## 1. 文档和配置文件各自的作用

### 1.1 `model_catalog.json` 的作用

JSON 是 ChatSpeed 后端使用的机器可读模型目录，集中记录两类信息：

1. **模型能力画像（Profile）**：根据模型 ID 识别模型家族、推理能力、函数调用、图片输入、上下文和输出限制，以及新建模型时可使用的推荐温度。
2. **请求传输适配（Transport）**：根据模型画像、实际请求端点和协议，选择已经在 Rust 中实现的 thinking 参数适配器。

模型目录不是供应商配置，也不是用户运行时配置：

- 不要在其中写入 API Key、密钥、密码、令牌或其他敏感信息；
- 不要把供应商 Base URL、账号或模型实例配置写进目录；
- 目录描述的是可复用的模型事实和协议规则，用户自己的模型配置仍由数据库和前端模型设置保存。

Rust 使用 `include_str!` 在编译期将 JSON 内嵌到应用中，并在运行时解析和缓存。因此，修改 JSON 后必须重新构建应用才会生效；仅修改已安装程序旁边的文件不会改变已经编译出的目录。

### 1.2 `model_catalog.md` 的作用

本文档是给开发者和维护者阅读的 schema/运维说明，不会被 Rust 解析，也不会影响模型匹配结果。它用于：

- 解释 JSON 每个字段的含义、类型和使用边界；
- 说明通配符、优先级、冲突和端点匹配规则；
- 记录 `thinkingAdapter` 与 Rust 实现之间的对应关系；
- 指导新增模型、供应商和来源证据时如何维护目录；
- 避免把能力判断、请求适配和用户配置混为一谈。

如果字段结构或实现规则发生变化，必须同步更新本文档、JSON 校验和相关测试。

## 2. 完整结构

当前文件的顶层结构如下：

```json
{
  "version": 1,
  "defaults": { "capabilities": {} },
  "profiles": [],
  "transports": []
}
```

### 2.1 顶层字段

| 字段 | 类型 | 必填 | 作用 |
| --- | --- | --- | --- |
| `version` | 整数 | 是 | 目录 schema 版本。目前只支持 `1`。不支持的版本会导致目录解析失败，而不是静默按旧格式解析。 |
| `defaults` | 对象 | 是 | 未匹配到任何画像时使用的基础结果。目前用于声明能力字段的未知默认值。 |
| `profiles` | 对象数组 | 是 | 模型能力画像规则。规则通过模型 ID 匹配，可跨供应商复用。 |
| `transports` | 对象数组 | 是 | 端点传输适配规则。规则决定实际请求使用哪个 Rust thinking adapter。 |

`profiles` 和 `transports` 可以为空数组，但必须保留字段；解析器会校验规则 ID、引用关系和字段冲突。

## 3. `defaults` 默认能力

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

| 字段 | 类型 | 作用 |
| --- | --- | --- |
| `defaults.capabilities` | 对象 | 设置没有足够目录证据时的能力结果。 |
| `defaults.capabilities.reasoning` | `true` / `false` / `null` | 是否支持推理。`null` 表示未知，不等于不支持。 |
| `defaults.capabilities.functionCall` | `true` / `false` / `null` | 是否支持函数调用。 |
| `defaults.capabilities.imageInput` | `true` / `false` / `null` | 是否支持图片输入。 |

能力字段是三态值：

- `true`：已有可靠证据确认支持；
- `false`：已有可靠证据确认不支持；
- `null` 或省略：没有足够证据，必须保留为未知。

不要为了填满字段而把未知能力写成 `false`。上层可以对未知值使用通用启发式或让用户选择，但未知和明确不支持在语义上不同。

## 4. `profiles` 模型能力画像

每个 profile 描述一组模型 ID 的共同能力。画像不绑定实际供应商端点，所以同一个模型通过官方 API 或聚合服务访问时，都可以复用同一个 profile。

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

### 4.1 Profile 字段

| 字段 | 类型 | 必填 | 作用 |
| --- | --- | --- | --- |
| `id` | 非空字符串 | 是 | 画像唯一标识。Transport 可通过此 ID 引用画像；同类规则应使用稳定、可读的名称。 |
| `priority` | 整数 | 是 | 画像合并优先级。数值越大越晚应用，通常用于让精确型号覆盖家族规则。 |
| `match` | 对象 | 是 | 画像匹配条件。当前主要使用 `model`，匹配模型 ID。 |
| `family` | 字符串，可省略 | 否 | 模型家族名称，供能力推断、传输适配匹配和诊断使用。它不是供应商名称。 |
| `capabilities` | 对象，可省略 | 否 | 画像提供的能力字段。只填写已经确认的值；未填写字段保持未知或继承已有值。 |
| `contextSize` | 非负整数，可省略 | 否 | 模型上下文长度。没有可靠来源时省略，不要猜测。 |
| `maxOutputTokens` | 非负整数，可省略 | 否 | 模型允许的最大输出 token 数。供应商 API 明确返回的值优先于目录补全值。 |
| `recommendedTemperature` | 数字，可省略 | 否 | 新建或导入模型时的推荐温度，仅用于 UI 预填。不会覆盖用户已保存的配置，也不会覆盖客户端请求中的显式温度。 |
| `reasoning` | 对象，可省略 | 否 | 推理 effort 策略，描述支持的档位及默认档位。只有确认模型支持可配置 effort 时才填写。 |
| `sources` | 对象数组，可省略 | 否 | 记录该规则的来源和核验日期，便于审计和后续更新。 |

### 4.2 `capabilities` 字段

Profile 中的 `capabilities` 与 `defaults.capabilities` 使用同一组字段：

- `reasoning`：是否支持 reasoning/thinking；
- `functionCall`：是否支持工具/函数调用；
- `imageInput`：是否接受图片输入。

字段名使用 camelCase，因为 JSON 和 Tauri 输出面向前端；Rust 结构体使用 `function_call`、`image_input` 等 snake_case，并通过 serde 映射。

### 4.3 `reasoning` 字段

```json
{
  "reasoning": {
    "supportedEfforts": ["none", "low", "high", "max"],
    "defaultEffort": "high"
  }
}
```

| 字段 | 类型 | 作用 |
| --- | --- | --- |
| `supportedEfforts` | 非空字符串数组 | 模型和该协议允许的 reasoning effort 值。值为 `none` 通常表示关闭推理。 |
| `defaultEffort` | 字符串 | 默认 effort，必须出现在 `supportedEfforts` 中，否则目录校验失败。 |

这里的 effort 是模型能力和协议策略描述，不是把某个值强行写入每一次请求。用户显式选择和客户端已有配置仍应优先于目录提供的默认建议。

## 5. `transports` 端点传输适配

Transport 描述“某模型发往某个实际端点时，如何序列化 thinking 参数”。模型名本身不足以决定适配器：聚合端点可能使用 OpenAI 兼容协议，但不一定接受某个厂商的专属字段。

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

### 5.1 Transport 字段

| 字段 | 类型 | 必填 | 作用 |
| --- | --- | --- | --- |
| `id` | 非空字符串 | 是 | Transport 唯一标识，也可被用户 metadata 显式指定。 |
| `priority` | 整数 | 是 | Transport 选择优先级。更高优先级规则优先于更低优先级规则。 |
| `match` | 对象 | 是 | 模型画像、模型 ID、端点主机和后端协议的组合匹配条件。 |
| `thinkingAdapter` | 枚举字符串 | 是 | 要使用的 Rust 类型安全适配器。不能配置任意 JSON 重写逻辑。 |
| `sources` | 对象数组，可省略 | 否 | 记录传输字段规则的来源和核验日期。 |

Transport 至少应绑定实际 `endpointHost`，除非它是明确的兼容性覆盖规则。没有端点约束的宽泛规则不能自动把官方厂商字段发送到聚合服务。

### 5.2 `thinkingAdapter` 可用值

当前实现中的值及用途如下：

| JSON 值 | Rust 适配器 | 用途 |
| --- | --- | --- |
| `open_ai` 或 `openai` | `OpenAi` | OpenAI reasoning 请求字段 |
| `claude` | `Claude` | Anthropic Claude thinking 字段 |
| `gemini` | `Gemini` | Gemini thinking 配置 |
| `deep_seek` 或 `deepseek` | `DeepSeek` | DeepSeek thinking 字段 |
| `qwen` | `Qwen` | 通义千问 thinking 字段 |
| `glm` | `Glm` | 智谱 GLM 字段 |
| `kimi` | `Kimi` | Kimi/Moonshot 字段 |
| `step_fun` 或 `stepfun` | `StepFun` | StepFun 字段 |
| `hunyuan_hy4_preview` | `HunyuanHy4Preview` | 腾讯混元 HY4 Preview 字段 |
| `doubao` | `Doubao` | 豆包/火山引擎字段 |
| `sense_nova` 或 `sensenova` | `SenseNova` | 商汤 SenseNova 字段 |
| `mistral` | `Mistral` | Mistral reasoning 字段 |
| `mimo` | `Mimo` | Xiaomi MiMo 字段 |
| `minimax` | `Minimax` | MiniMax 字段 |
| `nvidia_nim` | `NvidiaNim` | NVIDIA NIM 兼容端点字段 |

适配器名称只是选择 Rust 中已有的实现；新增名称前必须先实现对应的类型安全转换，并补充输入和输出路径测试。

## 6. `match` 匹配条件

`match` 是一个对象，所有已填写的条件必须同时满足；同一条件中的数组元素是“或”关系。

| 字段 | 类型 | 适用规则 | 作用 |
| --- | --- | --- | --- |
| `model` | 字符串数组 | Profile/Transport | 匹配模型 ID。适合写规范名称、带命名空间名称和必要的别名。 |
| `profile` | 字符串数组 | Transport | 匹配已经命中的 profile ID。用于把能力画像和端点适配关联起来。 |
| `endpointHost` | 字符串数组 | Transport | 匹配 Base URL 解析出的主机名。应优先使用精确官方域名。 |
| `backendProtocol` | 字符串数组 | Transport | 匹配后端协议，例如 `openai`、`claude` 或 `gemini`，用于区分同一端点的不同请求路径。 |

匹配前会对输入和 pattern 做首尾空白清理并转为 ASCII 小写。匹配使用完整字符串语义：

- `*` 匹配任意长度字符（包括空字符串）；
- `?` 匹配一个字符；
- 没有通配符时要求完整匹配，不是隐式子串匹配；
- 例如 `qwen3-*` 可以匹配 `qwen3-7b`，但不会匹配 `my-qwen3-7b`；
- pattern 数组是“或”关系，不同 match 字段之间是“且”关系；
- 未填写某个 match 字段表示不限制该条件。

## 7. 合并、优先级和冲突

### 7.1 Profile 合并

Resolver 会从默认能力开始，应用所有匹配的 profile，并按优先级从低到高处理：

1. 家族级通配规则先提供基础信息；
2. 更高优先级的精确型号规则覆盖或补充基础信息；
3. 对同一优先级产生不同值的规则，不能依赖数组顺序或 `HashMap` 顺序静默取值；
4. 冲突应由目录校验或测试发现并使解析失败。

因此，新增精确型号规则时应使用高于通用家族规则的 `priority`。Profile 的 `family`、能力、限制、推荐温度和 reasoning policy 都属于画像结果；未知字段应继续保持 `null`/缺失。

### 7.2 Transport 选择

Transport 选择需要同时考虑模型、命中的 profile、实际端点和后端协议：

1. 优先处理显式 metadata 覆盖；
2. 没有覆盖时，按端点和协议匹配 Transport；
3. 同一优先级必须得到唯一适配器；
4. 无法唯一确定或没有匹配时返回 `null`，不发送厂商专属字段；
5. 官方端点规则应使用精确 host，避免误命中聚合服务。

显式覆盖键为 `modelCatalogTransport`，值必须是已存在的 Transport ID。例如：

```json
{
  "modelCatalogTransport": "deepseek-official"
}
```

此覆盖表示用户明确确认当前自定义网关兼容该厂商字段。它不是供应商全局默认值，只影响携带该 metadata 的模型请求。不要把覆盖写入目录的普通 profile 规则中。

## 8. 配置值与“未设置”

模型目录中的 `recommendedTemperature` 是推荐值，不是强制默认值，也不是所有模型都必须存在的字段。

- 目录没有 `recommendedTemperature` 时，结果为未设置；
- 新建/导入模型时，只有表单字段本身为空才可使用推荐值预填；
- 用户已经保存的温度、请求中明确传入的温度都不能被推荐值覆盖；
- 对支持“未设置”的模型参数，配置文件省略该参数或使用对应的未设置表示时，应保留为 `None`，请求序列化时省略字段，而不是替换成通用默认值；
- `null`/缺失能力表示“未知”，不要把它解释为 `false`；这与采样参数的“未设置”是不同语义。

目录只提供模型事实和推荐信息，不能绕过用户设置或请求层的参数优先级。

## 9. `sources` 来源记录

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

| 字段 | 类型 | 作用 |
| --- | --- | --- |
| `sources[].url` | 字符串 | 证据来源地址或仓库内文档路径。应能说明模型能力或传输字段的依据。 |
| `sources[].verifiedAt` | `YYYY-MM-DD` 字符串 | 最近一次核验日期。模型供应商变更协议后应重新核验。 |

来源记录是维护和审查依据，不会自动联网验证，也不会改变 resolver 的匹配行为。

## 10. 维护流程和注意事项

新增或更新规则时：

1. 先确认模型官方 ID、能力和协议字段，并在 `sources` 中记录证据；
2. 选择最窄的模型 pattern，避免把不相关模型纳入同一家族；
3. 只有确认值才填写能力、上下文、输出限制和推荐温度；
4. 官方端点 Transport 必须绑定精确 `endpointHost`；聚合端点不得自动套用官方字段；
5. 需要兼容自定义网关时，优先使用 `modelCatalogTransport` 显式覆盖；
6. 新增适配器前先检查 Rust 枚举和对应 thinking 模块是否已经实现；
7. 补充匹配、优先级、冲突、官方端点、聚合端点和显式覆盖的测试；
8. 修改 JSON 后重新构建应用，确认嵌入资源和 Tauri 命令输出一致；
9. 同步更新本文档和 `docs/model-catalog-design.md`（若设计边界发生变化）。

不要做以下事情：

- 不要在 JSON 中保存凭据或用户私有数据；
- 不要用模型名称直接决定官方传输字段；
- 不要用 `false` 代替未知能力；
- 不要填写未经核验的 token 限制或推荐参数；
- 不要通过任意 JSON patch 绕过受限的 Rust 适配器；
- 不要依赖规则数组顺序解决同优先级冲突。

## 11. 与代码的关系

主要使用路径如下：

- `src-tauri/src/ai/model_catalog.rs`：加载、解析、校验、匹配和合并目录；
- `src-tauri/src/ai/chat/list_models.rs`：补全供应商模型列表中的能力和限制；
- `src-tauri/src/commands/model_catalog.rs`：向前端提供 profile 查询命令；
- `src/components/setting/Model.vue`、`src/stores/model.js`：模型新建、导入和配置预填；
- `src-tauri/src/ccproxy/helper/thinking/`：各厂商 thinking 参数的类型安全归一化；
- `src-tauri/src/ccproxy/handler/` 与 backend adapter：在统一请求和直连请求中使用解析出的 Transport。

当目录匹配不到或解析结果不确定时，安全行为是保留未知能力，并跳过不确定的厂商专属传输适配，而不是猜测并发送可能不兼容的字段。
