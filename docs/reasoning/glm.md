> ## Documentation Index
> Fetch the complete documentation index at: https://docs.bigmodel.cn/llms.txt
> Use this file to discover all available pages before exploring further.

# 深度思考

深度思考（Thinking）高级推理功能，通过启用思维链（Chain of Thought）机制，让模型在回答问题前进行深层次的分析和推理。这种方式能显著提升模型在复杂任务中的准确性和可解释性，特别适用于需要多步推理、逻辑分析和问题解决的场景。

## 功能特性

深度思考功能目前支持 GLM-5.3、GLM-5.3-FLASH、GLM-5.2、GLM-5.1、GLM-5、GLM-5-Turbo、GLM-5V-Turbo、GLM-4.7、GLM-4.6、GLM-4.5 等系列最新模型。通过启用深度思考，模型可以：

* **多步推理**：将复杂问题分解为多个步骤，逐步分析解决
* **逻辑分析**：提供清晰的推理过程和逻辑链条
* **提升准确性**：通过深度思考减少错误，提高回答质量
* **增强可解释性**：展示思考过程，让用户理解模型的推理逻辑
* **智能判断**：模型自动判断是否需要深度思考，优化响应效率

### 核心参数说明

<Note>
  注：GLM-5.3 GLM-5.3-FLASH 不再支持关闭思考（API 请求中 thinking.type 传 disabled 将会报错），请确保开启思考。
</Note>

* **`thinking.type`**: 控制深度思考模式
  * `enabled`（默认）：启用动态思考(`GLM-5.2` `GLM-5.1` `GLM-5` `GLM-5-Turbo` `GLM-5v-Turbo` `GLM-4.6` `GLM-4.6V` `GLM-4.5` 为模型自动判断是否思考，`GLM-5.3` `GLM-5.3-FLASH` `GLM-4.7` `GLM-4.5V` 为强制思考)
  * `disabled`：禁用思考，直接给出回答
* **`reasoning_effort`**: 控制开启思维链下的推理程度，仅 `GLM-5.2` 及以上支持
  * 可选值：`max`（默认且推荐，深度推理）、`high`（增强推理）、`low`（轻度推理，仅 `GLM-5.3` 支持）
  * 在 API 请求中
    * 针对 `GLM-5.3` `GLM-5.3-FLASH`，仅支持 `max`、`high`、`low`，其余输入将报错；
    * 针对 `GLM-5.2`，支持 `max`（默认且推荐，深度推理）、`xhigh`、`high`（增强推理）、`medium`、`low`、`minimal`、`none`，其中 `none` 或 `minimal` 代表模型放弃思考；`low` / `medium` 映射为 `high`；`xhigh` 映射为 `max`
  * 在 Coding Plan 请求中
    * 针对 `GLM-5.3` `GLM-5.3-FLASH`，`none`、`minimal`、`low` 映射为 `low`；`medium`、`high` 映射为 `high`；`xhigh`、`max` 映射为 `max`
    * 针对 `GLM-5.2`，`none` 或 `minimal` 代表模型放弃思考；`low` / `medium` 映射为 `high`；`xhigh` 映射为 `max`
* **`model`**: 支持深度思考的模型，`GLM-4.5` 及其以上版本支持。

## 代码示例

<Tabs>
  <Tab title="cURL">
    **基础调用（启用深度思考）**

    ```bash theme={null}
    curl --location 'https://open.bigmodel.cn/api/paas/v4/chat/completions' \
    --header 'Authorization: Bearer YOUR_API_KEY' \
    --header 'Content-Type: application/json' \
    --data '{
        "model": "glm-5.3",
        "messages": [
            {
                "role": "user",
                "content": "详细解释量子计算的基本原理，并分析其在密码学领域的潜在影响"
            }
        ],
        "thinking": {
            "type": "enabled"
        },
        "max_tokens": 4096,
        "temperature": 1.0
    }'
    ```

    **流式调用（深度思考 + 流式输出）**

    ```bash theme={null}
    curl --location 'https://open.bigmodel.cn/api/paas/v4/chat/completions' \
    --header 'Authorization: Bearer YOUR_API_KEY' \
    --header 'Content-Type: application/json' \
    --data '{
        "model": "glm-5.3",
        "messages": [
            {
                "role": "user",
                "content": "设计一个电商网站的推荐系统架构，考虑用户行为、商品特征和实时性要求"
            }
        ],
        "thinking": {
            "type": "enabled"
        },
        "stream": true,
        "max_tokens": 4096,
        "temperature": 1.0
    }'
    ```

    **控制推理程度（reasoning\_effort）**

    ```bash theme={null}
    curl --location 'https://open.bigmodel.cn/api/paas/v4/chat/completions' \
    --header 'Authorization: Bearer YOUR_API_KEY' \
    --header 'Content-Type: application/json' \
    --data '{
        "model": "glm-5.3",
        "messages": [
            {
                "role": "user",
                "content": "分析一下这道数学题的解题思路"
            }
        ],
        "thinking": {
            "type": "enabled"
        },
        "reasoning_effort": "max"
    }'
    ```

    **禁用深度思考（GLM-5.3 已不再支持，仅 GLM-5.2 支持）**

    ```bash theme={null}
    curl --location 'https://open.bigmodel.cn/api/paas/v4/chat/completions' \
    --header 'Authorization: Bearer YOUR_API_KEY' \
    --header 'Content-Type: application/json' \
    --data '{
        "model": "glm-5.2",
        "messages": [
            {
                "role": "user",
                "content": "今天天气怎么样？"
            }
        ],
        "thinking": {
            "type": "disabled"
        }
    }'
    ```
  </Tab>

  <Tab title="Python SDK">
    **安装 SDK**

    ```bash theme={null}
    # 安装最新版本
    pip install zai-sdk

    # 或指定版本
    pip install zai-sdk==0.2.3
    ```

    **验证安装**

    ```python theme={null}
    import zai
    print(zai.__version__)
    ```

    **基础调用（启用深度思考）**

    ```python theme={null}
    from zai import ZhipuAiClient

    # 初始化客户端
    client = ZhipuAiClient(api_key='YOUR_API_KEY')

    # 创建深度思考请求
    response = client.chat.completions.create(
        model="glm-5.3",
        messages=[
            {"role": "user", "content": "详细解释量子计算的基本原理，并分析其在密码学领域的潜在影响"}
        ],
        thinking={
            "type": "enabled"  # 启用深度思考模式
        },
        max_tokens=4096,
        temperature=1.0
    )

    print("模型响应:")
    print(response.choices[0].message.content)
    print("\n---")
    print(response.choices[0].message.reasoning_content)
    ```

    **流式调用（深度思考 + 流式输出）**

    ```python theme={null}
    from zai import ZhipuAiClient

    # 初始化客户端
    client = ZhipuAiClient(api_key='YOUR_API_KEY')

    # 创建流式深度思考请求
    response = client.chat.completions.create(
        model="glm-5.3",
        messages=[
            {"role": "user", "content": "设计一个电商网站的推荐系统架构，考虑用户行为、商品特征和实时性要求"}
        ],
        thinking={
            "type": "enabled"  # 启用深度思考模式
        },
        stream=True,  # 启用流式输出
        max_tokens=4096,
        temperature=1.0
    )

    # 处理流式响应
    reasoning_content = ""
    thinking_phase = True

    for chunk in response:
        if not chunk.choices:
            continue
        
        delta = chunk.choices[0].delta
        
        # 处理思考过程（如果有）
        if hasattr(delta, 'reasoning_content') and delta.reasoning_content:
            reasoning_content += delta.reasoning_content
            if thinking_phase:
                print("🧠 思考中...", end="", flush=True)
                thinking_phase = False
            print(delta.reasoning_content, end="", flush=True)
        
        # 处理回答内容
        if hasattr(delta, 'content') and delta.content:
            if thinking_phase:
                print("\n\n💡 回答:")
                thinking_phase = False
            print(delta.content, end="", flush=True)

    ```

    **控制推理程度（reasoning\_effort）**

    ```python theme={null}
    from zai import ZhipuAiClient

    # 初始化客户端
    client = ZhipuAiClient(api_key='YOUR_API_KEY')

    # 使用 reasoning_effort 控制推理程度
    response = client.chat.completions.create(
        model="glm-5.3",
        messages=[
            {"role": "user", "content": "分析一下这道数学题的解题思路"}
        ],
        thinking={
            "type": "enabled"
        },
        reasoning_effort="high"  # GLM-5.3 仅支持: max, high, low；GLM-5.2 另支持 xhigh, medium, low, minimal, none 并自动映射
    )

    print(response.choices[0].message.content)
    print(response.choices[0].message.reasoning_content)
    ```

    **禁用深度思考**

    ```python theme={null}
    from zai import ZhipuAiClient

    # 初始化客户端
    client = ZhipuAiClient(api_key='YOUR_API_KEY')

    # 禁用深度思考，快速响应
    response = client.chat.completions.create(
        model="glm-5.2",
        messages=[
            {"role": "user", "content": "今天天气怎么样？"}
        ],
        thinking={
            "type": "disabled"  # 禁用深度思考模式
        }
    )

    print(response.choices[0].message.content)
    ```
  </Tab>
</Tabs>

### 响应示例

启用深度思考的响应格式：

```json theme={null}
{
  "created": 1677652288,
  "model": "glm-5.3",
  "choices": [
    {
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "人工智能在医疗诊断中具有巨大的应用前景...",
        "reasoning_content": "让我从多个角度来分析这个问题。首先，我需要考虑AI在医疗诊断中的技术优势..."
      },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "completion_tokens": 239,
    "prompt_tokens": 8,
    "prompt_tokens_details": {
      "cached_tokens": 0
    },
    "total_tokens": 247
  }
}
```

## 最佳实践

**推荐启用的场景：**

* 复杂问题分析和解决
* 多步骤推理任务
* 技术方案设计
* 策略规划和决策
* 学术研究和分析
* 创意写作和内容创作

**可以禁用的场景：**

* 简单事实查询
* 基础翻译任务
* 简单分类判断
* 快速问答需求

## 注意事项

1. **响应时间**：启用深度思考会增加响应时间，特别是复杂任务
2. **Token 消耗**：思考过程会消耗额外的 Token，请合理规划使用
3. **模型支持**：确保使用支持深度思考功能的模型版本
4. **任务匹配**：根据任务复杂度选择是否启用深度思考
5. **流式输出**：结合流式输出可以实时查看思考过程，改善用户体验
