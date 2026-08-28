# 模型目录（Model Catalog）设计建议

> 状态：提案（尚未实施）
> 目的：为模型能力识别、模型列表补全与 CCProxy 厂商推理参数适配建立一个可维护的唯一事实来源。

## 背景

当前供应商预设与模型能力、厂商传输字段属于不同层级的问题：

- 供应商预设描述名称、协议、Base URL、文档地址和默认参数。
- 模型能力描述推理、函数调用、图片输入、上下文与输出限制等模型固有属性。
- 传输适配描述同一个 canonical thinking 请求如何在特定实际端点序列化为厂商 wire fields。

聚合供应商（例如 OpenRouter、ModelScope、SiliconFlow）可提供多个模型厂商的模型，因此不能根据供应商名称直接推断模型能力或套用厂商专属字段。

## 建议结论

新增一个独立的模型目录作为唯一事实来源，建议位置：

```text
src-tauri/resources/model_catalog.json
```

不要将这类规则重新放回 `public/presetTextAiProvider.json`：

- 后者应继续只保存前端供应商导入预设。
- Rust 的 `list_models` 和 CCProxy 运行于后端，不能可靠依赖 Vite 的 `public/` 文件。
- 同一份目录由 Rust 编译期内嵌并解析，前端通过 Tauri 命令获取解析结果，避免 TypeScript 与 Rust 各维护一套匹配逻辑。

添加既有厂商的新模型时，通常只需更新这一份目录。

## 两种规则必须分离

### 1. 模型能力画像（Profile）

能力画像主要根据真实模型 ID 识别，并可跨供应商复用。典型字段包括：

- 是否支持推理、函数调用、图片输入；
- 上下文长度与最大输出长度；
- 新模型配置的推荐温度；
- 推理支持档位与默认档位。

例如，通过 OpenRouter 使用 `deepseek-r1` 时，仍可识别它是推理模型。

### 2. 端点传输适配（Transport）

传输适配必须同时根据模型与实际 backend Base URL（必要时也包括协议及显式 metadata）决定。它只负责选择已经实现的、类型安全的 Rust 适配器。

例如，通过 OpenRouter 使用 `deepseek-r1` 时，可以应用 DeepSeek 的能力画像，但不能发送 DeepSeek 官方 API 独有的请求字段。只有请求发往 DeepSeek 官方或明确兼容该协议的端点时，才应选择 DeepSeek thinking adapter。

这也避免了仅凭模型名误命中厂商规则：例如 OpenRouter 暴露 `hy4-preview` 时，不应自动写入腾讯混元官方端点的字段。

## 建议的目录结构

以下为结构示例。没有经官方文档确认的数值应省略，不应填入推测值。

```json
{
  "version": 1,
  "defaults": {
    "capabilities": {
      "reasoning": null,
      "functionCall": null,
      "imageInput": null
    }
  },
  "profiles": [
    {
      "id": "hunyuan-hy4-preview",
      "priority": 200,
      "match": {
        "model": [
          "hy4-preview",
          "hunyuan/hy4-preview",
          "hunyuan-hy4-preview"
        ]
      },
      "capabilities": {
        "reasoning": true
      },
      "reasoning": {
        "supportedEfforts": ["none", "high"],
        "defaultEffort": "high"
      },
      "sources": [
        {
          "url": "docs/reasoning/hy.md",
          "verifiedAt": "2026-08-28"
        }
      ]
    },
    {
      "id": "qwen-vision-family",
      "priority": 80,
      "match": {
        "model": ["qwen*-vl-*", "qwen*-vision-*"]
      },
      "capabilities": {
        "imageInput": true
      }
    },
    {
      "id": "deepseek-r1-family",
      "priority": 100,
      "match": {
        "model": ["deepseek-r1*", "deepseek/deepseek-r1*"]
      },
      "capabilities": {
        "reasoning": true
      }
    }
  ],
  "transports": [
    {
      "id": "hunyuan-official-hy4-preview",
      "priority": 200,
      "match": {
        "profile": ["hunyuan-hy4-preview"],
        "endpointHost": [
          "api.hunyuan.cloud.tencent.com",
          "api.lkeap.cloud.tencent.com"
        ]
      },
      "thinkingAdapter": "hunyuan_hy4_preview"
    },
    {
      "id": "deepseek-official",
      "priority": 100,
      "match": {
        "model": ["deepseek*"],
        "endpointHost": ["api.deepseek.com", "api.deepseek.cn"]
      },
      "thinkingAdapter": "deepseek"
    },
    {
      "id": "qwen-dashscope",
      "priority": 100,
      "match": {
        "model": ["qwen*", "qwq*"],
        "endpointHost": ["dashscope.aliyuncs.com"]
      },
      "thinkingAdapter": "qwen"
    }
  ]
}
```

`thinkingAdapter` 应映射至 Rust 的受限枚举，而不是让配置直接描述任意 JSON 重写：

```rust
enum ThinkingAdapter {
    HunyuanHy4Preview,
    DeepSeek,
    Qwen,
    Glm,
    Kimi,
    StepFun,
    Claude,
    Gemini,
    OpenAi,
    Doubao,
    Sensenova,
}
```

因此，配置只负责识别、能力与适配器选择；现有 Rust thinking 模块仍负责各厂商协议字段的安全归一化与序列化。

## 匹配与优先级

目录 resolver 应遵循以下规则：

1. 模型 ID、主机名与 pattern 匹配前统一转小写并去除首尾空格。
2. 使用现有 `wildmatch` 语义支持 `*` 与 `?`，并匹配完整模型 ID；不做隐式子串匹配。
3. 按优先级从低到高叠加：基础家族规则先应用，精确型号规则后覆盖。
4. 同一优先级的规则若为同一字段产生冲突，目录加载或测试必须失败；不得依赖 `HashMap` 遍历顺序静默选择结果。
5. transport 在最高优先级必须解析出唯一 adapter。无法唯一解析时，安全地不写厂商专属字段，并记录可诊断日志。
6. 官方厂商 transport 应优先匹配精确 host，避免以宽泛通配符误匹配聚合端点。

可以为用户明确已知兼容某厂商 API 的自定义网关提供 `modelCatalogTransport` metadata 覆盖。它的优先级高于 host 匹配；没有显式覆盖时，聚合端点只能应用模型能力画像，不能自动应用官方字段重写。

## Resolver 输出与字段优先级

建议 resolver 接收：

```text
modelId, baseUrl, backendProtocol, metadataOverride
```

并返回：

```text
ResolvedModelProfile {
    profile_id,
    capabilities: { reasoning?, function_call?, image_input? },
    context_size?,
    max_output_tokens?,
    recommended_temperature?,
    reasoning_policy?,
    thinking_adapter?
}
```

字段应按以下优先级确定：

1. 用户已保存的模型配置；
2. provider `list_models` 明确返回的能力及上下文/输出限制；
3. 精确 Model Catalog 规则；
4. 家族级 Catalog 通配符规则；
5. 当前通用启发式，仅作为未知模型 fallback；
6. 仅用于 UI 显示的默认值。

补充约束：

- `recommendedTemperature` 仅用于导入或新建模型时预填，绝不覆盖客户端显式请求的 `temperature`。
- 客户端显式采样参数仍高于模型配置 fallback；只有目录明确要求的协议归一化可以改写字段。
- 能力字段必须为三态：`true`、`false`、未知（`null`/缺失）。未知不能等价于不支持。

## 建议接入点

### Rust 侧 catalog resolver

新增可独立测试的模块，例如：

```text
src-tauri/src/ai/model_catalog.rs
```

职责应仅限于：加载目录、schema 校验、通配符匹配、优先级合并，以及返回 `ResolvedModelProfile`。目录可通过 `include_str!` 编译期内嵌，通过 `OnceLock` 缓存解析结果。

### `list_models` 能力补全

`src-tauri/src/ai/chat/list_models.rs` 目前使用模型名启发式判断推理、函数调用和图片输入能力。应改为先调用 catalog resolver，再保留通用启发式作为未知模型 fallback。

API 明确返回的限制（例如 Claude 的输入/输出 token 限制）应保留为高优先级事实；catalog 仅补齐缺失信息或提供已确认的静态信息。

### 前端模型导入与手动添加

模型导入继续复用 `list_models` 的增强结果。对于不支持 `list_models` 的供应商或手动输入模型 ID，可增加窄 Tauri 命令：

```text
resolve_model_profile(modelId, baseUrl, protocol)
```

前端只用它预填已确认字段，不自行实现通配符与优先级规则。

### CCProxy thinking 适配

`src-tauri/src/ccproxy/helper/thinking/mod.rs` 的厂商判定可演进为：

```text
catalog.resolve_transport(model, base_url, protocol, metadata)
    -> ThinkingAdapter
    -> 现有 thinking/*.rs 模块
```

各 vendor 模块保持当前的 typed normalization 责任。输入适配与 backend serializer 都应读取同一个 canonical `UnifiedThinking` 与 `reasoning_policy`，不在 handler 重新建立平行转换路径。

## 需要补齐的持久化字段

前端与 `list_models` 均存在 `imageInput` 能力，但持久化的 `ModelConfig` 若没有对应字段，导入后的该能力可能在保存配置后丢失。

若要让图片输入能力可维护且可持久化，建议在 `ModelConfig` 中增加：

```rust
#[serde(rename = "imageInput", skip_serializing_if = "Option::is_none")]
pub image_input: Option<bool>,
```

并同步导入、导出、模型编辑与前端创建路径。

## 分阶段实施建议

1. 新增 catalog schema 与 resolver，并为精确匹配、`*`、`?`、优先级与冲突建立单元测试。
2. 先覆盖当前 thinking normalization 已支持的 GLM、Kimi、Qwen、DeepSeek、StepFun、Claude、Gemini、Hunyuan、Doubao、Sensenova 与 OpenAI 规则。
3. 将 `list_models` 的厂商模型名启发式迁入 catalog，同时保留未知模型的通用 fallback。
4. 让模型导入和手动添加调用 Rust resolver，并补齐 `imageInput` 持久化。
5. 改由 catalog 根据模型与实际端点选择 CCProxy thinking adapter，保留各厂商模块内的类型安全字段转换。
6. 建立端到端覆盖：官方端点适配、聚合端点不发送官方字段、用户显式 transport 覆盖，以及列表结果与 catalog overlay 的优先级。

## 当前实施边界

本文件记录的是后续架构演进建议，不代表该 catalog 已经实现。当前 CCProxy 已有的供应商 thinking normalization 模块应继续作为协议字段归一化的实现基础；本提案只建议未来以统一 catalog 驱动模型识别和端点适配器选择。
