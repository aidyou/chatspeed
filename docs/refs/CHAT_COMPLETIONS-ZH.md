

# **主流大语言模型聊天完成API对比分析报告**

信息来源：Gemini 深度研究 - [https://g.co/gemini/share/eb34cd4f188d](https://g.co/gemini/share/eb34cd4f188d)

## **执行摘要**

本报告旨在对OpenAI、Anthropic和Google三大领先提供商的聊天完成API进行全面比较。这些API是构建对话式人工智能应用的核心，它们使模型能够理解并生成人类语言，从而实现多轮对话、内容创作和自动化任务。

OpenAI的聊天完成API（/v1/chat/completions）以其广泛的模型支持和相对直观的请求结构而闻名，其流式响应采用标准服务器发送事件（SSE）格式。Anthropic的Claude消息API（/v1/messages）则在对话结构和安全性方面表现出严格的设计，通过详细的事件类型在流式响应中提供对模型内部思考过程的可见性。Google的Gemini API（generateContent方法）在多模态输入处理方面具有显著优势，其流式响应以增量JSON对象而非显式SSE事件类型呈现，同时提供了全面的安全和归因信息。

总体而言，虽然这些API都旨在实现聊天完成功能，但它们在消息结构、参数命名、流式传输机制以及对多模态和工具使用的支持方式上存在显著差异。这些差异决定了它们在不同应用场景下的适用性，并对开发者的集成策略产生了直接影响。

## **1\. 大语言模型聊天API简介**

聊天完成API是现代人工智能应用的关键组成部分，它们能够实现与大型语言模型（LLM）的动态、上下文感知对话。这些API广泛应用于构建智能客服系统、虚拟助手、交互式内容生成工具、教育平台以及任何需要模拟人类对话的应用。它们的核心价值在于能够维持对话上下文，并根据用户输入生成连贯、相关的响应，从而极大地提升了用户体验和自动化水平。

OpenAI、Anthropic和Google作为生成式AI领域的领导者，各自提供了强大的聊天完成API。OpenAI以其广泛的模型产品组合和易于集成的特性，推动了LLM技术的普及。Anthropic则以其对模型安全性和可控性的强调而著称，致力于构建更安全、更可解释的AI系统。Google凭借其在AI研究领域的深厚积累，通过Gemini模型在多模态理解和生成方面展现出强大能力。

以下表格总结了这三大API的基本接入信息：

**表1：API端点与认证摘要**

| 提供商        | API名称                             | 端点                                                                                                                                                           | 认证方法                                            | API版本控制方法                       |
| :------------ | :---------------------------------- | :------------------------------------------------------------------------------------------------------------------------------------------------------------- | :-------------------------------------------------- | :------------------------------------ |
| OpenAI        | OpenAI聊天完成API                   | https://api.openai.com/v1/chat/completions                                                                                                                     | Bearer Token (API Key)                              | Azure OpenAI使用api-version查询参数 1 |
| Anthropic     | Anthropic Claude消息API             | {{baseUrl}}/{{version}}/messages (例如，https://api.anthropic.com/v1/messages)                                                                                 | Bearer Token (x-api-key header)                     | 使用anthropic-version header 2        |
| Google Gemini | Google Gemini API (generateContent) | https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent 或 https://generativelanguage.googleapis.com/v1/models/{model}:generateContent | API Key (x-goog-api-key header) 或 Google Cloud认证 | SDK中通过apiVersion参数设置 4         |

此表格为开发者提供了快速参考，概括了集成这些API所需的基本信息，包括端点URL、认证机制以及版本控制方法。它揭示了API密钥认证的普遍性，同时也指出了特定于提供商的头部要求或更复杂的认证流程（如Google Cloud Vertex AI）。

## **2\. OpenAI聊天完成API (/v1/chat/completions)**

OpenAI的聊天完成API是其核心对话式AI接口，专为处理多轮对话而设计，其请求输入需要遵循特定的消息历史格式 10。这与早期用于单字符串提示的

/v1/completions端点有所不同 1。

### **2.1. 请求参数：完整规范**

该API的请求方法为POST，端点为https://api.openai.com/v1/chat/completions 10。认证通过在请求头中包含

Authorization: Bearer YOUR\_API\_KEY实现 12。

**核心对话字段：**

* messages (对象数组，**必需**): 构成对话的消息列表。模型根据特定角色进行训练。
  * **约束：** 每个对象必须包含role和content。
  * **角色：** system、user、assistant、tool 11。
    * system角色：提供初始指令或上下文 13。
    * user角色：代表用户输入。
    * assistant角色：代表模型响应。
    * tool角色：用于工具输出。
  * content (字符串或内容部分数组，必需): 消息的文本内容。对于多模态模型（如gpt-4o），可以是字符串或内容部分数组（支持图像）。
  * name (字符串，可选): 参与者的可选名称，用于区分相同角色的不同参与者 13。
  * tool\_calls (对象数组，可选): 助手消息中包含的工具调用，包含id和function（名称、参数）。
  * tool\_call\_id (字符串，可选): 对于工具消息，此消息响应的工具调用的ID。
* model (字符串，**必需**): 要使用的模型ID（例如，gpt-3.5-turbo、gpt-4o、gpt-4o-mini） 11。
  * **约束：** 支持特定模型；请查阅文档获取最新列表 13。

**生成控制参数：**

* max\_tokens (整数，可选): 完成中可生成的最大令牌数。
  * **约束：** 提示令牌数 \+ max\_tokens不能超过模型的上下文长度 1。
  * **默认值：** 16 1。
* temperature (数字，可选): 采样温度。值越高（例如，0.8）输出越随机，值越低（例如，0.2）输出越集中和确定。
  * **约束：** 范围0到2 12。
  * **默认值：** 1.0。
* top\_p (数字，可选): 核采样。模型考虑累积概率超过top\_p的令牌。
  * **约束：** 范围0到1 12。
  * **默认值：** 1.0。
* n (整数，可选): 为每个输入消息生成多少个聊天完成选项。
  * **约束：** 可能会迅速消耗令牌配额 1。
  * **默认值：** 1 1。
  * **注意：** 某些兼容API（如Langdock）不支持此参数 13。
* stream (布尔值，可选): 如果为true，将以服务器发送事件（SSE）的形式发送部分消息增量。
  * **默认值：** false 12。
* stop (字符串或字符串数组，可选): 最多四个序列，API将在遇到这些序列时停止生成更多令牌。返回的文本将不包含停止序列 1。
* seed (整数，可选): 如果指定，系统将尽力确定性采样，但不能保证每次请求都返回相同结果 1。

**高级功能：**

* presence\_penalty (数字，可选): 根据新令牌是否已出现在文本中来惩罚它们。正值会增加模型谈论新主题的可能性。
  * **约束：** 范围-2.0到2.0 1。
  * **默认值：** 0.0 1。
* frequency\_penalty (数字，可选): 根据新令牌在文本中已有的频率来惩罚它们。正值会降低模型逐字重复相同行的可能性。
  * **约束：** 范围-2.0到2.0 1。
  * **默认值：** 0.0 1。
* logit\_bias (对象，可选): 通过将令牌ID映射到偏置值（-100到100）来修改指定令牌出现的可能性。
  * **约束：** 偏置值范围-100到100 1。
* user (字符串，可选): 代表最终用户的唯一标识符，有助于监控和检测滥用 1。
* tools (对象数组，可选): 模型可能调用的工具列表。目前仅支持function类型的工具。每个工具包含name、description（推荐）和parameters（工具输入的JSON schema） 16。
* tool\_choice (字符串或对象，可选): 控制模型调用哪个（如果有）工具。
  * **值：** none（模型不调用工具，而是生成消息）、auto（模型自行决定）、required（模型必须调用一个或多个工具）。也可以通过{"type": "function", "function": {"name": "my\_function"}}指定特定工具 13。
  * **默认值：** 如果没有工具，则为none；如果存在工具，则为auto 13。
* response\_format (对象，可选): 指定模型必须输出的格式。
  * **type：** (枚举\<字符串\>，必需) text或json\_object。
  * **约束：** 使用json\_object时，必须在提示中明确指示模型生成JSON，以避免无限空白字符的生成 13。
* logprobs (布尔值，可选): 是否返回输出令牌的对数概率。
  * **默认值：** false 16。
  * **注意：** 在流式模式下，仅返回有关所选令牌的信息，不返回完整的对数概率 16。
* stream\_options (对象，可选): 流式响应的选项，仅在stream: true时设置。
  * **include\_usage** (布尔值，可选): 如果为true，将在data:消息之前流式传输一个额外的块，其中包含整个请求的令牌使用统计信息 16。
  * **注意：** 某些兼容API（如Langdock）不支持此参数 13。
* service\_tier (枚举\<字符串\>，可选): 确定是使用优先容量（priority\_only）还是标准容量（standard\_only）。
  * **可用选项：** auto、standard\_only。
  * **注意：** 某些兼容API（如Langdock）不支持此参数 13。

**表2：OpenAI聊天完成API \- 请求参数**

| 参数名称           | 类型               | 必需/可选 | 描述                                     | 约束/默认值                                                                                           | 示例值                                                                                                    |
| :----------------- | :----------------- | :-------- | :--------------------------------------- | :---------------------------------------------------------------------------------------------------- | :-------------------------------------------------------------------------------------------------------- |
| messages           | 对象数组           | 必需      | 构成对话的消息列表。                     | 每个对象包含role和content。role可为system, user, assistant, tool。content可为字符串或多模态内容数组。 | \[{"role": "user", "content": "Hello"}\]                                                                  |
| model              | 字符串             | 必需      | 要使用的模型ID。                         | 长度1-256字符。                                                                                       | "gpt-4o"                                                                                                  |
| max\_tokens        | 整数               | 可选      | 完成中可生成的最大令牌数。               | 提示令牌 \+ max\_tokens \<= 模型上下文长度。默认值: 16。                                              | 1024                                                                                                      |
| temperature        | 数字               | 可选      | 采样温度。                               | 范围: 0到2。默认值: 1.0。                                                                             | 0.7                                                                                                       |
| top\_p             | 数字               | 可选      | 核采样。                                 | 范围: 0到1。默认值: 1.0。                                                                             | 0.9                                                                                                       |
| n                  | 整数               | 可选      | 为每个输入消息生成多少个聊天完成选项。   | 默认值: 1。注意：某些兼容API不支持。                                                                  | 1                                                                                                         |
| stream             | 布尔值             | 可选      | 是否以SSE形式流式传输响应。              | 默认值: false。                                                                                       | true                                                                                                      |
| stop               | 字符串或字符串数组 | 可选      | 停止生成令牌的序列。                     | 最多4个序列。                                                                                         | \["\\nUser:", "\#\#\#"\]                                                                                  |
| seed               | 整数               | 可选      | 用于确定性采样的随机种子。               | 不保证完全确定性。                                                                                    | 1234                                                                                                      |
| presence\_penalty  | 数字               | 可选      | 根据新令牌是否已出现来惩罚它们。         | 范围: \-2.0到2.0。默认值: 0.0。                                                                       | 0.5                                                                                                       |
| frequency\_penalty | 数字               | 可选      | 根据新令牌在文本中已有的频率来惩罚它们。 | 范围: \-2.0到2.0。默认值: 0.0。                                                                       | 0.5                                                                                                       |
| logit\_bias        | 对象               | 可选      | 修改指定令牌出现的可能性。               | 映射令牌ID到偏置值（-100到100）。                                                                     | {"1234": 50}                                                                                              |
| user               | 字符串             | 可选      | 代表最终用户的唯一标识符。               |                                                                                                       | "user-123"                                                                                                |
| tools              | 对象数组           | 可选      | 模型可能调用的工具列表。                 | 仅支持function类型。包含name, description, parameters。                                               | \[{"type": "function", "function": {"name": "get\_weather", "description": "...", "parameters": {...}}}\] |
| tool\_choice       | 字符串或对象       | 可选      | 控制模型调用哪个工具。                   | none, auto, required或指定工具。默认值: none或auto。                                                  | "auto" 或 {"type": "function", "function": {"name": "get\_weather"}}                                      |
| response\_format   | 对象               | 可选      | 指定模型输出格式。                       | type可为text或json\_object。json\_object需提示中明确指示。                                            | {"type": "json\_object"}                                                                                  |
| logprobs           | 布尔值             | 可选      | 是否返回输出令牌的对数概率。             | 默认值: false。流式模式下仅返回所选令牌信息。                                                         | true                                                                                                      |
| stream\_options    | 对象               | 可选      | 流式响应选项。                           | 仅在stream: true时设置。包含include\_usage (布尔值)。                                                 | {"include\_usage": true}                                                                                  |
| service\_tier      | 枚举\<字符串\>     | 可选      | 确定容量使用类型。                       | auto, standard\_only, priority\_only。                                                                | "standard\_only"                                                                                          |

OpenAI的API设计演进体现了其对对话式AI范式的适应和优化。从早期接受单一字符串提示的/completions端点，到如今要求结构化消息历史的/chat/completions端点，这表明了模型底层训练和应用场景的根本性转变 1。这种转变不仅影响了请求的格式，也反映出模型处理多轮对话和维护上下文的能力得到了增强。对于开发者而言，这意味着从旧有集成迁移到新接口时，需要对请求和响应处理逻辑进行重大调整。然而，OpenAI通过保留旧有端点，在一定程度上维护了对非聊天用例的向后兼容性，同时推动了更强大、更适合对话式AI的新标准。

此外，OpenAI在工具集成方面的成熟度也值得关注。请求参数中包含tools和tool\_choice，响应中包含tool\_calls 13，这表明其函数调用能力已经相当完善。

tool\_choice参数的required选项 13 允许开发者强制模型调用特定工具，这对于构建复杂的智能体工作流具有强大作用。模型响应中标准化

tool\_calls结构为应用程序解析和执行外部函数提供了清晰的接口，从而实现了与真实世界数据或服务的复杂交互。流式传输对tool\_calls的支持（通过delta.tool\_calls隐式实现）意味着应用程序可以增量地响应工具调用，从而可能加速复杂智能体循环的执行。

### **2.2. 同步响应格式 (JSON)**

对于同步请求（stream: false），API返回一个表示已完成聊天交互的JSON对象。

**根对象结构：**

* id (字符串，必需): 聊天响应的唯一ID 14。
* object (字符串，必需): 对象类型，始终为"chat.completion" 14。
* created (整数，必需): 聊天完成创建时的Unix时间戳 14。
* model (字符串，必需): 用于创建聊天完成的模型ID 14。
* choices (对象数组，必需): 模型响应选项的列表。长度与请求体中的n参数对应（默认为1） 14。
  * index (整数，必需): 列表中选项的索引 16。
  * message (对象，必需): 模型生成的聊天完成消息 16。
    * role (字符串，必需): 消息作者的角色，通常为"assistant" 16。
    * content (字符串，必需): 消息的内容 16。如果存在
      tool\_calls，则可能为null。
    * tool\_calls (对象数组，可选): 模型生成的工具调用数组 16。
      * id (字符串，必需): 工具调用的ID。
      * type (字符串，必需): 工具调用的类型，始终为"function"。
      * function (对象，必需): 模型希望调用的函数。
        * name (字符串，必需): 要调用的函数名称。
        * arguments (字符串，必需): 调用函数时使用的参数，以JSON字符串形式表示。
  * finish\_reason (字符串或null，必需): 模型停止生成令牌的原因。
    * **可能值：** stop（自然停止或停止序列）、length（达到最大令牌数）、tool\_calls（模型请求工具调用）、content\_filter（内容违规）、function\_call（已弃用，由tool\_calls取代） 16。
  * logprobs (对象或null，可选): 选项的对数概率信息 16。
    * content (对象数组，可选): 完成中每个令牌的对数概率数据列表。
      * token (字符串，必需): 令牌。
      * logprob (数字，必需): 令牌的对数概率。
      * bytes (整数数组，可选): 令牌的字节表示。
      * top\_logprobs (对象数组，可选): 此位置最有可能的令牌及其对数概率列表。
* usage (对象，必需): 完成请求的使用统计信息 14。
  * prompt\_tokens (整数，必需): 提示中的令牌数 14。
  * completion\_tokens (整数，必需): 生成完成中的令牌数 14。
  * total\_tokens (整数，必需): 使用的总令牌数（提示 \+ 完成） 16。
* system\_fingerprint (字符串，可选): 生成响应的后端配置的唯一标识符。可用于监控后端变化 1。

**表3：OpenAI聊天完成API \- 同步响应字段**

| 字段路径                                       | 类型         | 描述                                 | 可能值/结构                                                  |
| :--------------------------------------------- | :----------- | :----------------------------------- | :----------------------------------------------------------- |
| id                                             | 字符串       | 聊天响应的唯一ID。                   |                                                              |
| object                                         | 字符串       | 对象类型。                           | "chat.completion"                                            |
| created                                        | 整数         | 聊天完成创建时的Unix时间戳。         |                                                              |
| model                                          | 字符串       | 用于创建聊天完成的模型ID。           |                                                              |
| choices                                        | 对象数组     | 模型响应选项列表。                   | 长度与n参数对应。                                            |
| choices.index                                  | 整数         | 列表中选项的索引。                   |                                                              |
| choices.message                                | 对象         | 模型生成的聊天完成消息。             |                                                              |
| choices.message.role                           | 字符串       | 消息作者的角色。                     | "assistant"                                                  |
| choices.message.content                        | 字符串       | 消息的内容。                         | 如果存在tool\_calls，可能为null。                            |
| choices.message.tool\_calls                    | 对象数组     | 模型生成的工具调用数组。             |                                                              |
| choices.message.tool\_calls.id                 | 字符串       | 工具调用的ID。                       |                                                              |
| choices.message.tool\_calls.type               | 字符串       | 工具调用的类型。                     | "function"                                                   |
| choices.message.tool\_calls.function           | 对象         | 模型希望调用的函数。                 |                                                              |
| choices.message.tool\_calls.function.name      | 字符串       | 要调用的函数名称。                   |                                                              |
| choices.message.tool\_calls.function.arguments | 字符串       | 调用函数时使用的参数（JSON字符串）。 |                                                              |
| choices.finish\_reason                         | 字符串或null | 模型停止生成令牌的原因。             | stop, length, tool\_calls, content\_filter, function\_call。 |
| choices.logprobs                               | 对象或null   | 选项的对数概率信息。                 | content数组，包含token, logprob, bytes, top\_logprobs。      |
| usage                                          | 对象         | 完成请求的使用统计信息。             |                                                              |
| usage.prompt\_tokens                           | 整数         | 提示中的令牌数。                     |                                                              |
| usage.completion\_tokens                       | 整数         | 生成完成中的令牌数。                 |                                                              |
| usage.total\_tokens                            | 整数         | 使用的总令牌数。                     |                                                              |
| system\_fingerprint                            | 字符串       | 生成响应的后端配置的唯一标识符。     |                                                              |

### **2.3. 流式响应格式 (Server-Sent Events)**

当请求中设置stream: true时，OpenAI的API以服务器发送事件（SSE）的形式发送部分消息增量。每个事件都是以data: 开头的JSON对象，流以data:消息终止 16。

**SSE事件结构：**

* 每个事件都是以data:开头的行，后跟一个JSON对象。
* 最终事件是data:。

**choices.delta对象与同步message对象的区别：**

在流式传输中，choices数组包含delta对象而不是message对象 16。

delta对象表示消息的增量变化。它可能包含role（通常只在第一个块中）、content（部分字符串）或tool\_calls（部分工具调用对象）。delta中的content字段是一个字符串片段，需要进行拼接以形成完整的消息内容。delta中的tool\_calls也可以增量流式传输，需要进行累积。finish\_reason在生成停止之前将为null，停止时将在最终的delta块中填充 16。

logprobs也以增量方式流式传输 16。

**增量更新和最终usage块：**

大多数流式块的usage字段将为null。如果在请求中设置了stream\_options.include\_usage，则在data:之前将流式传输一个额外的块，其中包含整个请求的总使用统计信息 16。这对于实时令牌核算是一个关键功能。

**表4：OpenAI聊天完成API \- 流式事件类型与增量**

| 事件类型     | 描述                           | 增量中的关键字段                                                                        | 示例增量结构                                                                                                                                                                                               |
| :----------- | :----------------------------- | :-------------------------------------------------------------------------------------- | :--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| data: {... } | 初始或中间消息增量。           | choices.index, choices.delta (role, content, tool\_calls), model, created, id, object。 | {"id": "chatcmpl-...", "object": "chat.completion.chunk", "created": 1234567890, "model": "gpt-4o", "choices": \[{"index": 0, "delta": {"role": "assistant"}, "logprobs": null, "finish\_reason": null}\]} |
| data: {... } | 文本内容增量。                 | choices.delta.content                                                                   | {"choices": \[{"index": 0, "delta": {"content": "Hello"}}\], "id": "...", "object": "chat.completion.chunk",...}                                                                                           |
| data: {... } | 工具调用增量。                 | choices.delta.tool\_calls                                                               | {"choices":}}\], "id": "...", "object": "chat.completion.chunk",...}                                                                                                                                       |
| data: {... } | 最终消息增量（包含停止原因）。 | choices.finish\_reason                                                                  | {"choices": \[{"index": 0, "delta": {}, "finish\_reason": "stop"}\], "id": "...", "object": "chat.completion.chunk",...}                                                                                   |
| data: {... } | 最终使用统计信息块。           | usage (prompt\_tokens, completion\_tokens, total\_tokens)                               | {"id": "...", "object": "chat.completion.chunk", "created": 1234567890, "model": "gpt-4o", "choices":, "usage": {"prompt\_tokens": 10, "completion\_tokens": 20, "total\_tokens": 30}}                     |
| data:        | 流结束标记。                   | 无                                                                                      | data:                                                                                                                                                                                                      |

OpenAI在流式传输设计上，通过stream\_options.include\_usage参数提供了精细控制与简化实现的平衡。这种设计避免了在每个小的增量块中重复包含usage数据，从而减小了载荷大小，同时又能在不发送额外非流式请求的情况下提供总的使用量信息 16。对于需要实时令牌成本估算的应用，此功能至关重要，因为它能够在流式对话结束时提供准确的计费和配额管理。这表明OpenAI在流式传输效率方面做出了权衡，同时仍提供了重要的元数据。

## **3\. Anthropic Claude消息API (/v1/messages)**

Anthropic的Messages API专为强大的对话式AI设计，其请求和响应结构都体现了对对话流程和安全性的严格控制。

### **3.1. 请求参数：完整规范**

Anthropic Claude Messages API使用/v1/messages端点 2。

* **端点：** POST {{baseUrl}}/{{version}}/messages (例如，https://api.anthropic.com/v1/messages) 2。
* **认证：** 请求头中包含x-api-key (字符串，**必需**) 3。
* **请求头：**
  * anthropic-version (字符串，**必需**): 指定API版本，例如2023-06-01 2。
  * anthropic-beta (字符串，可选): 指定beta版本，逗号分隔或多个头部 3。
  * content-type: application/json 2。

**核心对话字段：**

* messages (对象数组，**必需**): 结构化的输入消息列表 2。
  * **约束：** 模型在交替的user和assistant轮次上进行训练。请求中连续的相同角色轮次将被合并。
  * **每个消息对象：** 必须包含role和content 3。
  * role：(user或assistant) 3。
  * content：(字符串或内容块数组，必需)
    * 可以是单个string（\[{"type": "text", "text": "..."}\]的简写） 3。
    * 可以是内容块数组，每个块都有一个type。
    * **type: "text"：** text (字符串，必需) \- 纯文本内容 3。
    * **type: "image"：** source (对象，必需) \- 图像内容。
      * type (字符串，必需): base64。
      * media\_type (字符串，必需): image/jpeg、image/png、image/gif、image/webp 3。
      * data (字符串，必需): Base64编码的图像数据 3。
    * **type: "tool\_use"：** 用于模型生成的工具调用。包含id、name、input (JSON对象)。
    * **type: "tool\_result"：** 用于将工具输出返回给模型。包含tool\_use\_id和content（文本块数组或字符串）。
  * **约束：** 如果最后一条消息使用assistant角色，响应内容将立即从该消息的内容继续 3。
  * **限制：** 单个请求中最多100,000条消息 3。
* model (字符串，**必需**): 将完成提示的模型（例如，claude-3-opus-20240229、claude-3-5-sonnet-latest） 2。
  * **约束：** 长度1-256个字符 3。
* max\_tokens (整数，**必需**): 停止生成前的最大令牌数。
  * **约束：** x \>= 1。不同模型有不同的最大值 2。

**上下文和控制参数：**

* system (字符串或文本内容块数组，可选): 提供上下文和指令的系统提示 3。
* temperature (数字，可选): 响应中的随机性量。
  * **约束：** 范围0.0到1.0。接近0.0用于分析任务，接近1.0用于创意任务 3。
  * **默认值：** 1.0 3。
* stop\_sequences (字符串数组，可选): 导致模型停止生成的自定义文本序列。如果匹配，stop\_reason将为"stop\_sequence" 3。
* top\_k (整数，可选): 仅从每个后续令牌的前K个选项中采样。
  * **约束：** x \>= 0。仅推荐用于高级用例 3。
* top\_p (数字，可选): 核采样。
  * **约束：** 范围0到1。仅推荐用于高级用例 3。

**高级功能：**

* stream (布尔值，可选): 是否使用服务器发送事件增量流式传输响应。
  * **默认值：** false。
* metadata (对象，可选): 描述请求元数据的对象。
  * **user\_id** (字符串，可选): 用户的外部标识符（UUID、哈希或不透明标识符）。最大长度256 3。
* service\_tier (枚举\<字符串\>，可选): 确定是使用优先容量（priority\_only）还是标准容量（standard\_only）。
  * **可用选项：** auto、standard\_only 3。
  * **默认值：** auto 3。
* thinking (对象，可选): 启用Claude扩展思考的配置。
  * **约束：** 需要至少1,024个令牌的预算，并计入max\_tokens限制 3。
  * **type** (枚举\<字符串\>，必需): enabled 3。
  * **budget\_tokens** (整数，必需): Claude可用于内部推理的令牌数量。x \>= 1024且小于max\_tokens 3。
* tool\_choice (对象，可选): 模型应如何使用提供的工具。
  * **type** (枚举\<字符串\>，必需): auto 3。
  * **disable\_parallel\_tool\_use** (布尔值，可选): 如果为true，模型最多输出一个工具使用。默认false 3。
* tools (对象数组，可选): 模型可能使用的工具定义。
  * 每个工具：name (字符串)、description (字符串，强烈推荐)、input\_schema (工具输入形状的JSON schema) 3。
  * **支持的工具类型：** custom、Bash tool、Code execution tool、Computer use tool、Text editor tool、Web search tool 3。
* cache\_control (对象，可选): 在此内容块创建缓存控制断点。
  * **type** (枚举\<字符串\>，必需): ephemeral 3。
  * **ttl** (枚举\<字符串\>，可选): 断点的生存时间。选项：5m、1h。默认5m 3。
* container (字符串或null，可选): 用于跨请求重用的容器标识符 3。
* mcp\_servers (对象数组，可选): 将使用的MCP服务器。
  * 每个服务器：name (字符串，必需)、type (枚举\<字符串\>，必需，url)、url (字符串，必需)、authorization\_token (字符串或null，可选)、tool\_configuration (对象或null，可选) 3。

**表5：Anthropic Claude消息API \- 请求参数**

| 参数名称        | 类型                   | 必需/可选 | 描述                               | 约束/默认值                                                                                             | 示例值                                                                                                                           |
| :-------------- | :--------------------- | :-------- | :--------------------------------- | :------------------------------------------------------------------------------------------------------ | :------------------------------------------------------------------------------------------------------------------------------- |
| messages        | 对象数组               | 必需      | 输入消息列表。                     | 包含role (user/assistant) 和 content。content可为字符串或内容块数组（文本、图像、工具使用、工具结果）。 | \[{"role": "user", "content": "Hello"}, {"role": "assistant", "content": "Hi\!"}, {"role": "user", "content": "Explain LLMs."}\] |
| model           | 字符串                 | 必需      | 将完成提示的模型。                 | 长度1-256字符。                                                                                         | "claude-3-opus-20240229"                                                                                                         |
| max\_tokens     | 整数                   | 必需      | 停止生成前的最大令牌数。           | x \>= 1。                                                                                               | 1024                                                                                                                             |
| system          | 字符串或文本内容块数组 | 可选      | 提供上下文和指令的系统提示。       |                                                                                                         | "You are a helpful assistant."                                                                                                   |
| temperature     | 数字                   | 可选      | 响应中的随机性量。                 | 范围: 0.0到1.0。默认值: 1.0。                                                                           | 0.7                                                                                                                              |
| stop\_sequences | 字符串数组             | 可选      | 导致模型停止生成的自定义文本序列。 |                                                                                                         | \["\\nUser:", "\#\#\#"\]                                                                                                         |
| top\_k          | 整数                   | 可选      | 仅从前K个选项中采样。              | x \>= 0。仅推荐用于高级用例。                                                                           | 50                                                                                                                               |
| top\_p          | 数字                   | 可选      | 核采样。                           | 范围: 0到1。仅推荐用于高级用例。                                                                        | 0.9                                                                                                                              |
| stream          | 布尔值                 | 可选      | 是否使用SSE流式传输响应。          | 默认值: false。                                                                                         | true                                                                                                                             |
| metadata        | 对象                   | 可选      | 请求的元数据。                     | 包含user\_id (字符串，最大长度256)。                                                                    | {"user\_id": "user-abc-123"}                                                                                                     |
| service\_tier   | 枚举\<字符串\>         | 可选      | 确定容量使用类型。                 | auto, standard\_only, priority\_only。默认值: auto。                                                    | "priority\_only"                                                                                                                 |
| thinking        | 对象                   | 可选      | 启用扩展思考的配置。               | 包含type (enabled) 和 budget\_tokens (整数, \>=1024且\<max\_tokens)。                                   | {"type": "enabled", "budget\_tokens": 2048}                                                                                      |
| tool\_choice    | 对象                   | 可选      | 模型应如何使用提供的工具。         | 包含type (auto) 和 disable\_parallel\_tool\_use (布尔值)。                                              | {"type": "auto", "disable\_parallel\_tool\_use": true}                                                                           |
| tools           | 对象数组               | 可选      | 模型可能使用的工具定义。           | 包含name, description, input\_schema。                                                                  | \[{"name": "get\_weather", "description": "...", "input\_schema": {...}}\]                                                       |
| cache\_control  | 对象                   | 可选      | 创建缓存控制断点。                 | 包含type (ephemeral) 和 ttl (5m, 1h)。                                                                  | {"type": "ephemeral", "ttl": "1h"}                                                                                               |
| container       | 字符串或null           | 可选      | 用于重用的容器标识符。             |                                                                                                         | "my-container-id"                                                                                                                |
| mcp\_servers    | 对象数组               | 可选      | 将使用的MCP服务器。                | 包含name, type (url), url, authorization\_token, tool\_configuration。                                  | \[{"name": "server1", "type": "url", "url": "https://..."}\]                                                                     |

Anthropic在API设计上对显式对话结构和安全性表现出高度重视。其messages参数严格要求user和assistant角色的交替出现 3，这比OpenAI更为严格。此外，

system提示是一个独立的顶级参数 3，与消息数组分开。这种设计选择表明Anthropic致力于控制对话流程，并为模型提供清晰、一致的上下文。独立的

system提示可能有助于模型行为更稳健，减少因对话轮次而导致的指令稀释。这可能是Claude在特定任务中更具“可对齐性”或可控性，以及其安全优先方法的原因之一。

Anthropic对多模态和工具的集成也体现了其作为核心功能的设计理念。messages中的content字段明确支持内容块数组，包括各种media\_type的image 3。同样，

tool\_use和tool\_result也被定义为一流的内容块类型 3。这表明多模态和工具调用被深度集成到Claude的核心API设计中，而非作为附加功能。结构化的内容块使得在单个对话轮次中组合不同模态和管理工具交互变得简单。这种设计促进了构建多模态、智能体应用程序的统一方法，与那些可能需要通过单独端点或较少集成结构来处理这些功能的API相比，可能简化了开发。

### **3.2. 同步响应格式 (JSON)**

对于stream: false的同步请求，响应是一个单一的Message对象 3。

**根对象结构：**

* id (字符串，必需): 唯一的对象标识符 3。
* type (枚举\<字符串\>，必需): 对象类型，始终为"message" 3。
* role (枚举\<字符串\>，必需): 生成消息的对话角色，始终为"assistant" 3。
* content (对象数组，必需): 模型生成的内容。这是一个内容块数组，每个块都有一个type 3。
  * **type: "text"：** text (字符串，必需) \- 例如，\[{"type": "text", "text": "Hi, I'm Claude."}\] 3。
  * **type: "thinking"：** thinking (字符串，必需) \- Claude的内部推理过程 3。
  * **type: "redacted\_thinking"：** thinking (字符串，必需) \- 经过编辑的思考内容。
  * **type: "tool\_use"：** id (字符串，必需)、name (字符串，必需)、input (对象，必需) \- 模型对工具的使用 3。
  * **type: "server\_tool\_use"：** tool\_name (字符串，必需)、tool\_id (字符串，必需)、input (对象，必需) \- 服务器端工具使用。
  * **type: "web\_search\_tool\_result"：** tool\_name (字符串，必需)、tool\_id (字符串，必需)、output (对象，必需) \- 来自网络搜索工具的结果。
  * **type: "code\_execution\_tool\_result"：** tool\_name (字符串，必需)、tool\_id (字符串，必需)、output (对象，必需) \- 来自代码执行工具的结果。
  * **type: "mcp\_tool\_use"：** id (字符串，必需)、tool\_name (字符串，必需)、input (对象，必需) \- MCP工具使用。
  * **type: "mcp\_tool\_result"：** tool\_use\_id (字符串，必需)、output (对象，必需) \- MCP工具结果。
  * **type: "container\_upload"：** id (字符串，必需)、tool\_name (字符串，必需)、input (对象，必需) \- 容器上传。
* model (字符串，必需): 处理请求的模型 3。
* stop\_reason (枚举\<字符串\>或null，必需): 模型停止的原因。在非流式模式下非空 3。
  * **可能值：** "end\_turn"、"max\_tokens"、"stop\_sequence"、"tool\_use"、"pause\_turn"、"refusal" 3。
* stop\_sequence (字符串或null，必需): 如果有，哪个自定义停止序列被生成 3。
* usage (对象，必需): 计费和速率限制使用情况 3。
  * **input\_tokens** (整数，必需): 使用的输入令牌数 3。
  * **output\_tokens** (整数，必需): 使用的输出令牌数 3。
  * **cache\_creation\_input\_tokens** (整数或null，必需): 用于创建缓存条目的输入令牌数 3。
  * **cache\_read\_input\_tokens** (整数或null，必需): 从缓存读取的输入令牌数 3。
  * **cache\_creation** (对象或null，必需): 按TTL划分的缓存令牌细分 3。
    * **ephemeral\_1h\_input\_tokens** (整数，必需): 1小时缓存的输入令牌数 3。
    * **ephemeral\_5m\_input\_tokens** (整数，必需): 5分钟缓存的输入令牌数 3。
  * **server\_tool\_use** (对象或null，必需): 服务器工具请求的数量 3。
    * **web\_search\_requests** (整数，必需): 网络搜索工具请求的数量 3。
  * **service\_tier** (枚举\<字符串\>或null，必需): 使用的层级（priority\_only、standard\_only、batch） 3。
* container (对象或null，必需): 如果使用了容器工具，则包含有关所用容器的信息 3。
  * **expires\_at** (字符串，必需): 容器过期的时间 3。
  * **id** (字符串，必需): 容器的标识符 3。

**表6：Anthropic Claude消息API \- 同步响应字段**

| 字段路径                             | 类型                 | 描述                                                           | 可能值/结构                                                                                                                                                                                |
| :----------------------------------- | :------------------- | :------------------------------------------------------------- | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| id                                   | 字符串               | 唯一的对象标识符。                                             |                                                                                                                                                                                            |
| type                                 | 枚举\<字符串\>       | 对象类型。                                                     | "message"                                                                                                                                                                                  |
| role                                 | 枚举\<字符串\>       | 生成消息的对话角色。                                           | "assistant"                                                                                                                                                                                |
| content                              | 对象数组             | 模型生成的内容块数组。                                         | type可为text, thinking, redacted\_thinking, tool\_use, server\_tool\_use, web\_search\_tool\_result, code\_execution\_tool\_result, mcp\_tool\_use, mcp\_tool\_result, container\_upload。 |
| content.type                         | 字符串               | 内容块类型。                                                   | 例如"text", "tool\_use"。                                                                                                                                                                  |
| content.text                         | 字符串               | 文本内容（当type为text时）。                                   |                                                                                                                                                                                            |
| content.thinking                     | 字符串               | 模型的内部推理过程（当type为thinking或redacted\_thinking时）。 |                                                                                                                                                                                            |
| content.id                           | 字符串               | 工具使用或容器上传的ID。                                       |                                                                                                                                                                                            |
| content.name                         | 字符串               | 工具名称（当type为tool\_use时）。                              |                                                                                                                                                                                            |
| content.input                        | 对象                 | 工具输入（当type为tool\_use时）。                              | JSON对象。                                                                                                                                                                                 |
| content.tool\_name                   | 字符串               | 服务器工具名称（当type为server\_tool\_use时）。                |                                                                                                                                                                                            |
| content.tool\_id                     | 字符串               | 服务器工具ID（当type为server\_tool\_use时）。                  |                                                                                                                                                                                            |
| content.output                       | 对象                 | 工具输出（当type为web\_search\_tool\_result等时）。            |                                                                                                                                                                                            |
| model                                | 字符串               | 处理请求的模型。                                               |                                                                                                                                                                                            |
| stop\_reason                         | 枚举\<字符串\>或null | 模型停止的原因。                                               | end\_turn, max\_tokens, stop\_sequence, tool\_use, pause\_turn, refusal。                                                                                                                  |
| stop\_sequence                       | 字符串或null         | 匹配到的自定义停止序列。                                       |                                                                                                                                                                                            |
| usage                                | 对象                 | 计费和速率限制使用情况。                                       |                                                                                                                                                                                            |
| usage.input\_tokens                  | 整数                 | 输入令牌数。                                                   |                                                                                                                                                                                            |
| usage.output\_tokens                 | 整数                 | 输出令牌数。                                                   |                                                                                                                                                                                            |
| usage.cache\_creation\_input\_tokens | 整数或null           | 用于创建缓存条目的输入令牌数。                                 |                                                                                                                                                                                            |
| usage.cache\_read\_input\_tokens     | 整数或null           | 从缓存读取的输入令牌数。                                       |                                                                                                                                                                                            |
| usage.cache\_creation                | 对象或null           | 按TTL划分的缓存令牌细分。                                      | 包含ephemeral\_1h\_input\_tokens, ephemeral\_5m\_input\_tokens。                                                                                                                           |
| usage.server\_tool\_use              | 对象或null           | 服务器工具请求的数量。                                         | 包含web\_search\_requests。                                                                                                                                                                |
| usage.service\_tier                  | 枚举\<字符串\>或null | 使用的层级。                                                   | priority\_only, standard\_only, batch。                                                                                                                                                    |
| container                            | 对象或null           | 容器信息。                                                     | 包含expires\_at, id。                                                                                                                                                                      |

### **3.3. 流式响应格式 (Server-Sent Events)**

当设置stream: true时，Anthropic的API使用服务器发送事件（SSE）增量传递响应 3。每个事件都包含一个命名的事件

type和相关的JSON数据 17。

SSE事件流 17：

1. **message\_start**: 初始事件。包含一个Message对象，其content为空。stop\_reason为null 3。
2. **内容块**: 针对每个内容块的一系列事件：
   * **content\_block\_start**: 标记内容块的开始。包含块的index和type 17。
   * **一个或多个content\_block\_delta事件**: 提供内容块的增量更新。每个增量都有一个index和一个delta对象 17。
     * **text\_delta**: 用于文本内容。包含text（字符串片段） 17。
     * **input\_json\_delta**: 用于tool\_use内容块。包含partial\_json（字符串片段）。这些是部分JSON字符串，需要在content\_block\_stop时累积并解析 17。
     * **thinking\_delta**: 启用扩展思考时。包含thinking（字符串片段） 17。
     * **signature\_delta**: 用于thinking内容的特殊事件，在content\_block\_stop之前发送，用于验证思考块的完整性 17。
   * **content\_block\_stop**: 标记内容块的结束。包含index 17。
3. **一个或多个message\_delta事件**: 表示最终Message对象的顶层变化。
   * message\_delta中的usage字段是**累积的** 17。
   * stop\_reason在模型停止时，在最终的message\_delta事件中变为非null 3。
4. **message\_stop**: 流中的最终事件，表示完成 17。

其他事件类型 17：

* **ping事件**: 可能会分散在整个响应中，用于保持连接 17。
* **error事件**: 可能会偶尔发送，用于表示overloaded\_error等问题 17。代码应优雅地处理未知事件类型。

**与同步响应的区别：**

累积的流式响应的整体结构与同步Message对象匹配。主要区别在于content和usage字段的增量性质，通过\_delta事件传递。stop\_reason在message\_start中为null，然后在模型停止时在message\_delta中填充 3。

**表7：Anthropic Claude消息API \- 流式事件类型与增量**

| 事件类型                     | 描述                                  | 增量中的关键字段                                                                                         | 示例增量结构                                                                                                                                                                                                                                                          |
| :--------------------------- | :------------------------------------ | :------------------------------------------------------------------------------------------------------- | :-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| event: message\_start        | 流开始，包含一个空内容的Message对象。 | type, message (id, type, role, model, stop\_reason (null), stop\_sequence (null), usage (empty content)) | event: message\_start\\ndata: {"type": "message\_start", "message": {"id": "msg\_...", "type": "message", "role": "assistant", "model": "claude-3-opus-20240229", "stop\_reason": null, "stop\_sequence": null, "usage": {"input\_tokens": 10, "output\_tokens": 0}}} |
| event: content\_block\_start | 内容块开始。                          | type, index, content\_block (type, text (empty))                                                         | event: content\_block\_start\\ndata: {"type": "content\_block\_start", "index": 0, "content\_block": {"type": "text", "text": ""}}}                                                                                                                                   |
| event: content\_block\_delta | 文本内容增量。                        | type, index, delta (type, text)                                                                          | event: content\_block\_delta\\ndata: {"type": "content\_block\_delta", "index": 0, "delta": {"type": "text\_delta", "text": "Hello"}}                                                                                                                                 |
| event: content\_block\_delta | 工具使用JSON输入增量。                | type, index, delta (type, partial\_json)                                                                 | event: content\_block\_delta\\ndata: {"type": "content\_block\_delta", "index": 1, "delta": {"type": "input\_json\_delta", "partial\_json": "{\\"location\\": \\"New York\\"}}"                                                                                       |
| event: content\_block\_delta | 思考内容增量。                        | type, index, delta (type, thinking)                                                                      | event: content\_block\_delta\\ndata: {"type": "content\_block\_delta", "index": 0, "delta": {"type": "thinking\_delta", "thinking": "Let me break this down..."}}                                                                                                     |
| event: content\_block\_delta | 思考内容签名增量。                    | type, index, delta (type, signature)                                                                     | event: content\_block\_delta\\ndata: {"type": "content\_block\_delta", "index": 0, "delta": {"type": "signature\_delta", "signature": "EqQBCgIYAhIM..."}}                                                                                                             |
| event: content\_block\_stop  | 内容块结束。                          | type, index                                                                                              | event: content\_block\_stop\\ndata: {"type": "content\_block\_stop", "index": 0}                                                                                                                                                                                      |
| event: message\_delta        | 顶层消息变化和累积使用量。            | type, delta (usage), usage (cumulative)                                                                  | event: message\_delta\\ndata: {"type": "message\_delta", "delta": {"stop\_reason": "end\_turn"}, "usage": {"output\_tokens": 50}}}                                                                                                                                    |
| event: message\_stop         | 流结束。                              | type                                                                                                     | event: message\_stop\\ndata: {"type": "message\_stop"}                                                                                                                                                                                                                |
| event: ping                  | 保持连接。                            | type                                                                                                     | event: ping\\ndata: {"type": "ping"}                                                                                                                                                                                                                                  |
| event: error                 | 流中发生的错误。                      | type, error (type, message)                                                                              | event: error\\ndata: {"type": "error", "error": {"type": "overloaded\_error", "message": "The server is currently overloaded."}}                                                                                                                                      |

Anthropic的API提供了对模型内部过程的精细可见性，例如请求中的thinking参数 3 以及响应中相应的

thinking内容块 3。此外，

usage字段中详细的cache\_creation和cache\_read\_input\_tokens 3 提供了深入的缓存使用指标。这种详细程度超越了典型的大语言模型API。这种设计提供了一个前所未有的窗口，让开发者能够观察模型的内部推理过程。对于调试、审计或开发更复杂的AI系统（例如，自我纠正智能体）来说，这具有极高的价值。详细的缓存指标（

cache\_creation\_input\_tokens、cache\_read\_input\_tokens、ephemeral\_1h\_input\_tokens、ephemeral\_5m\_input\_tokens）提供了对令牌使用和与提示缓存相关的潜在成本优化的细粒度洞察。这表明Anthropic旨在为开发者提供对模型行为和资源消耗更深入的控制和理解，从而促进更高效和可解释的AI应用程序的开发。

## **4\. Google Gemini API (models.generateContent)**

本节将详细介绍Google Gemini API的generateContent方法的请求和响应结构。值得注意的是，尽管最初的片段显示信息“不可用”，但通过对ai.google.dev/api/generate-content和cloud.google.com/vertex-ai/generative-ai/docs/reference/rest/v1/GenerateContentResponse等文档的深入研究，获得了关于Gemini API参数和响应结构的详细信息 4。

### **4.1. 请求参数：完整规范**

Gemini API使用models.generateContent方法进行聊天完成 4。

* **端点：** POST https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent 4。Vertex AI可能使用不同的端点 8。
* **认证：** Gemini Developer API使用x-goog-api-key请求头 4，Vertex AI使用Google Cloud认证 5。

**核心对话字段：**

* contents (对象数组 \- Content，**必需**): 与模型的当前对话内容 7。
  * **约束：** 对于单轮查询，是一个单一实例。对于多轮聊天，是一个重复字段，包含对话历史和最新请求。
  * **每个Content对象：**
    * role (字符串，必需): 此内容的作者角色。通常为user或model（用于助手响应）。
    * parts (对象数组 \- Part，必需): 内容部分的列表。
      * **type: "text"：** text (字符串，必需) \- 纯文本 4。
      * **type: "inline\_data"：** mime\_type (字符串，必需)、data (字符串，必需，Base64编码) \- 用于图像、音频等 19。
      * **type: "function\_call"：** name (字符串，必需)、args (对象，必需) \- 模型生成的函数调用 20。
      * **type: "function\_response"：** name (字符串，必需)、response (对象，必需) \- 函数调用的输出。
      * **type: "file\_data"：** file\_uri (字符串，必需)、mime\_type (字符串，必需) \- 对已上传文件的引用 20。
      * **type: "video\_metadata"：** video\_uri (字符串，必需)、start\_offset\_millis (整数，可选)、end\_offset\_millis (整数，可选) \- 视频内容。
      * **type: "uri"：** uri (字符串，必需)、mime\_type (字符串，必需) \- 通用URI内容 20。
* model (字符串，隐式在端点路径中，**必需**): 要使用的模型ID（例如，gemini-2.5-flash、gemini-1.5-pro） 4。

**配置参数 (generationConfig)：** 6

* temperature (数字，可选): 控制随机性。
  * **默认值：** 0.9。
  * **约束：** 范围0.0到1.0 6。
* top\_p (数字，可选): 核采样。
  * **约束：** 范围0.0到1.0。
* top\_k (整数，可选): 仅从前K个选项中采样。
* stop\_sequences (字符串数组，可选): 停止生成的自定义序列 6。
* response\_mime\_type (字符串，可选): 期望的响应MIME类型。
  * **约束：** application/json用于JSON模式，text/x-enum用于枚举输出 21。
  * **注意：** 还需要在提示中指示模型生成JSON 21。
* response\_schema (对象，可选): JSON schema，用于强制结构化输出 21。
  * **支持字段：** enum、items、maxItems、nullable、properties、required 21。
* thinkingConfig (对象，可选): 启用Claude扩展思考的配置。
  * **thinkingBudget** (整数，可选): 确定Claude可用于内部推理过程的令牌数量。0禁用思考 6。

**安全和上下文参数：**

* safetySettings (对象数组 \- SafetySetting，可选): 用于阻止不安全内容的唯一SafetySetting实例列表 7。
  * **约束：** 在请求contents和响应candidates上强制执行。每个SafetyCategory最多一个设置。覆盖默认值。
  * **category**：(枚举\<字符串\>，必需) 例如，HARM\_CATEGORY\_HATE\_SPEECH、HARM\_CATEGORY\_SEXUALLY\_EXPLICIT 7。
  * **threshold**：(枚举\<字符串\>，必需) 例如，BLOCK\_NONE、BLOCK\_LOW\_AND\_ABOVE。
* systemInstruction (对象 \- Content，可选): 允许开发者设置系统指令，目前仅限于文本 6。
  * **约束：** 必须是包含text的parts的Content对象。

**高级功能：**

* tools (对象数组 \- Tool，可选): 模型可能使用的Tool列表。
  * **支持类型：** Function、codeExecution 7。
* toolConfig (对象 \- ToolConfig，可选): 为指定工具提供的工具配置 7。
* cachedContent (字符串，可选): 用于预测的上下文缓存内容名称。格式：cachedContents/{cachedContent} 7。

**表8：Google Gemini API \- 请求参数**

| 参数名称                              | 类型                     | 必需/可选 | 描述                              | 约束/默认值                                                                                                                           | 示例值                                                                      |
| :------------------------------------ | :----------------------- | :-------- | :-------------------------------- | :------------------------------------------------------------------------------------------------------------------------------------ | :-------------------------------------------------------------------------- |
| contents                              | 对象数组 (Content)       | 必需      | 与模型的当前对话内容。            | 每个Content对象包含role和parts。parts可包含text, inline\_data, function\_call, function\_response, file\_data, video\_metadata, uri。 | \[{"role": "user", "parts": \[{"text": "Hello"}\]}\]                        |
| model                                 | 字符串                   | 必需      | 要使用的模型ID（在URL路径中）。   |                                                                                                                                       | "gemini-1.5-pro"                                                            |
| generationConfig                      | 对象 (GenerationConfig)  | 可选      | 模型生成和输出的配置选项。        |                                                                                                                                       | {"temperature": 0.5}                                                        |
| generationConfig.temperature          | 数字                     | 可选      | 控制随机性。                      | 范围: 0.0到1.0。默认值: 0.9。                                                                                                         | 0.7                                                                         |
| generationConfig.top\_p               | 数字                     | 可选      | 核采样。                          | 范围: 0.0到1.0。                                                                                                                      | 0.9                                                                         |
| generationConfig.top\_k               | 整数                     | 可选      | 仅从前K个选项中采样。             |                                                                                                                                       | 40                                                                          |
| generationConfig.stop\_sequences      | 字符串数组               | 可选      | 停止生成的自定义序列。            |                                                                                                                                       | \["\\nUser:", "\#\#\#"\]                                                    |
| generationConfig.response\_mime\_type | 字符串                   | 可选      | 期望的响应MIME类型。              | application/json或text/x-enum。                                                                                                       | "application/json"                                                          |
| generationConfig.response\_schema     | 对象                     | 可选      | JSON schema，用于强制结构化输出。 | 支持enum, items, maxItems, nullable, properties, required。                                                                           | {"type": "object", "properties": {"name": {"type": "string"}}}              |
| generationConfig.thinkingConfig       | 对象                     | 可选      | 启用扩展思考的配置。              | 包含thinkingBudget (整数)。0禁用。                                                                                                    | {"thinkingBudget": 1024}                                                    |
| safetySettings                        | 对象数组 (SafetySetting) | 可选      | 用于阻止不安全内容的设置列表。    | 每个设置包含category和threshold。                                                                                                     | \`\`                                                                        |
| systemInstruction                     | 对象 (Content)           | 可选      | 系统指令。                        | 必须是包含text的parts的Content对象。                                                                                                  | {"parts":}                                                                  |
| tools                                 | 对象数组 (Tool)          | 可选      | 模型可能使用的工具列表。          | 支持Function和codeExecution类型。                                                                                                     | \[{"function\_declarations": \[{"name": "get\_time", "parameters": {}}\]}\] |
| toolConfig                            | 对象 (ToolConfig)        | 可选      | 为指定工具提供的工具配置。        |                                                                                                                                       |                                                                             |
| cachedContent                         | 字符串                   | 可选      | 用于预测的上下文缓存内容名称。    | 格式: cachedContents/{cachedContent}。                                                                                                | "cachedContents/my-cache"                                                   |

Gemini的contents参数是一个Content对象数组，其中每个Content对象可以包含多种Part类型（文本、图像/音频的inline\_data、上传文件的file\_data、video\_metadata、uri） 7。这种高度灵活且集成的多模态输入方法，表明多模态推理是Gemini架构的基础，而非附加功能。它简化了创建在对话中融合不同模态的应用程序的开发者体验，因为所有输入都在一个统一的

contents结构中处理。这与那些可能需要单独端点或更复杂编码来处理多模态输入的API形成对比，突显了Gemini在原生处理各种数据类型方面的优势。

此外，Gemini对结构化输出和缓存的重视也值得注意。它通过generationConfig中的response\_mime\_type和response\_schema明确支持结构化输出 21。

cachedContent参数 7 以及

UsageMetadata中相关的cachedContentTokenCount 22 则指向其高级缓存能力。结构化输出功能（通常称为“JSON模式”或“受控生成”）对于将LLM集成到自动化工作流和期望可预测数据格式的下游系统至关重要。这减少了对后处理和解析的需求，提高了可靠性和效率。缓存机制，尽管其完整范围未详细说明，但表明其对重复或相似提示进行了优化，可能降低了某些用例的延迟和成本。这表明Google专注于实现健壮、可扩展且经济高效的企业解决方案。

### **4.2. 同步响应格式 (GenerateContentResponse)**

同步响应是一个GenerateContentResponse对象 7。

根对象结构 22：

* candidates (对象数组 \- Candidate，只读，必需): 生成的候选响应 22。
* modelVersion (字符串，只读，必需): 用于生成响应的模型版本 22。
* createTime (字符串，时间戳格式，只读，必需): 请求发送到服务器的时间戳 22。
* responseId (字符串，只读，必需): 用于标识每个响应的ID 22。
* promptFeedback (对象 \- PromptFeedback，只读，必需): 提示的内容过滤结果。仅在第一个流块中发送，并且仅在由于内容违规而未生成任何候选时出现 22。
* usageMetadata (对象 \- UsageMetadata，必需): 关于响应的使用元数据 22。

详细嵌套对象 22：

* **Candidate**: 模型生成的响应候选。
  * index (整数，只读): 候选的索引。
  * content (对象 \- Content，只读): 候选的内容部分。
  * avgLogprobs (数字，只读): 候选的平均对数概率得分。
  * logprobsResult (对象 \- LogprobsResult，只读): 响应令牌的对数似然得分。
  * finishReason (枚举 \- FinishReason，只读): 模型停止生成令牌的原因。如果为空，则模型仍在生成。
  * safetyRatings (对象数组 \- SafetyRating，只读): 响应候选安全性的评级列表。每个类别最多一个评级。
  * citationMetadata (对象 \- CitationMetadata，只读): 生成内容的来源归因。
  * groundingMetadata (对象 \- GroundingMetadata，只读): 指定用于内容基础的来源元数据。
  * urlContextMetadata (对象 \- UrlContextMetadata，只读): 与URL上下文检索工具相关的元数据。
  * finishMessage (字符串，只读): 停止原因的更详细描述（仅当finishReason设置时填充）。
* **LogprobsResult**: 对数概率结果。
  * topCandidates (对象数组 \- TopCandidates): 每个解码步骤中具有最高对数概率的候选。
  * chosenCandidates (对象数组 \- Candidate): 选定的候选。
* **Candidate (嵌套在LogprobsResult和TopCandidates中)**: 对数概率令牌和分数的候选。
  * token (字符串)、tokenId (整数)、logProbability (数字)。
* **FinishReason (枚举)**: FINISH\_REASON\_UNSPECIFIED、STOP、MAX\_TOKENS、SAFETY、RECITATION、OTHER、BLOCKLIST、PROHIBITED\_CONTENT、SPII、MALFORMED\_FUNCTION\_CALL、IMAGE\_SAFETY、IMAGE\_PROHIBITED\_CONTENT、IMAGE\_RECITATION、IMAGE\_OTHER、UNEXPECTED\_TOOL\_CALL。
* **SafetyRating**: 与生成内容对应的安全评级。
  * category (枚举 \- HarmCategory)、probability (枚举 \- HarmProbability)、probabilityScore (数字)、severity (枚举 \- HarmSeverity)、severityScore (数字)、blocked (布尔值)、overwrittenThreshold (枚举 \- HarmBlockThreshold)。
* **HarmProbability (枚举)**: HARM\_PROBABILITY\_UNSPECIFIED、NEGLIGIBLE、LOW、MEDIUM、HIGH。
* **HarmSeverity (枚举)**: HARM\_SEVERITY\_UNSPECIFIED、HARM\_SEVERITY\_NEGLIGIBLE、HARM\_SEVERITY\_LOW、HARM\_SEVERITY\_MEDIUM、HARM\_SEVERITY\_HIGH。
* **CitationMetadata**: 内容来源归因的集合。
  * citations (对象数组 \- Citation)。
* **Citation**: 内容的来源归因。
  * startIndex (整数)、endIndex (整数)、uri (字符串)、title (字符串)、license (字符串)、publicationDate (对象 \- Date)。
* **GroundingMetadata**: 启用基础功能时返回给客户端的元数据。
  * webSearchQueries (字符串)、groundingChunks (对象 \- GroundingChunk)、groundingSupports (对象 \- GroundingSupport)、searchEntryPoint (对象 \- SearchEntryPoint)、retrievalMetadata (对象 \- RetrievalMetadata)、googleMapsWidgetContextToken (字符串)。
* **SearchEntryPoint**: Google搜索入口点。
  * renderedContent (字符串)、sdkBlob (字符串，字节格式)。
* **GroundingChunk**: 基础块类型（联合类型：web、retrievedContext、maps）。
* **Web**: 来自网络的块（uri、title、domain）。
* **RetrievedContext**: 来自检索工具检索到的上下文块（context\_details (联合类型：ragChunk)、uri、title、text）。
* **Maps**: 来自Google地图的块（uri、title、text、placeId）。
* **GroundingSupport**: 基础支持。
  * groundingChunkIndices (整数)、confidenceScores (数字)、segment (对象 \- Segment)。
* **Segment**: 内容的片段（partIndex、startIndex、endIndex、text）。
* **RetrievalMetadata**: 与基础流程中检索相关的元数据（googleSearchDynamicRetrievalScore）。
* **UrlContextMetadata**: 与URL上下文检索工具相关的元数据。
  * urlMetadata (对象 \- UrlMetadata)。
* **UrlMetadata**: 单个URL检索的上下文。
  * retrievedUrl (字符串)、urlRetrievalStatus (枚举 \- UrlRetrievalStatus)。
* **UrlRetrievalStatus (枚举)**: URL\_RETRIEVAL\_STATUS\_UNSPECIFIED、URL\_RETRIEVAL\_STATUS\_SUCCESS、URL\_RETRIEVAL\_STATUS\_ERROR。
* **PromptFeedback**: 提示的内容过滤结果。
  * blockReason (枚举 \- BlockedReason)、safetyRatings (对象 \- SafetyRating)、blockReasonMessage (字符串)。
* **BlockedReason (枚举)**: BLOCKED\_REASON\_UNSPECIFIED、SAFETY、OTHER、BLOCKLIST、PROHIBITED\_CONTENT、IMAGE\_SAFETY。
* **UsageMetadata**: 关于响应的使用元数据。
  * promptTokenCount (整数)、candidatesTokenCount (整数)、toolUsePromptTokenCount (整数)、thoughtsTokenCount (整数)、totalTokenCount (整数)、cachedContentTokenCount (整数)、promptTokensDetails (对象 \- ModalityTokenCount)、cacheTokensDetails (对象 \- ModalityTokenCount)、candidatesTokensDetails (对象 \- ModalityTokenCount)、toolUsePromptTokensDetails (对象 \- ModalityTokenCount)、trafficType (枚举 \- TrafficType)。
* **ModalityTokenCount**: 单一模态的令牌计数信息。
  * modality (枚举 \- Modality)、tokenCount (整数)。
* **Modality (枚举)**: MODALITY\_UNSPECIFIED、TEXT、IMAGE、VIDEO、AUDIO、DOCUMENT。
* **TrafficType (枚举)**: TRAFFIC\_TYPE\_UNSPECIFIED、ON\_DEMAND、PROVISIONED\_THROUGHPUT。

**表9：Google Gemini API \- 同步响应字段**

| 字段路径                           | 类型                      | 描述                              | 可能值/结构                              |
| :--------------------------------- | :------------------------ | :-------------------------------- | :--------------------------------------- |
| candidates                         | 对象数组 (Candidate)      | 生成的候选响应。                  |                                          |
| modelVersion                       | 字符串                    | 用于生成响应的模型版本。          |                                          |
| createTime                         | 字符串 (Timestamp)        | 请求发送到服务器的时间戳。        | RFC 3339格式。                           |
| responseId                         | 字符串                    | 标识每个响应的ID。                |                                          |
| promptFeedback                     | 对象 (PromptFeedback)     | 提示的内容过滤结果。              | 仅在第一个流块中发送，仅在无候选时出现。 |
| usageMetadata                      | 对象 (UsageMetadata)      | 关于响应的使用元数据。            |                                          |
| candidates.index                   | 整数                      | 候选的索引。                      |                                          |
| candidates.content                 | 对象 (Content)            | 候选的内容部分。                  |                                          |
| candidates.avgLogprobs             | 数字                      | 候选的平均对数概率得分。          |                                          |
| candidates.logprobsResult          | 对象 (LogprobsResult)     | 响应令牌的对数似然得分。          |                                          |
| candidates.finishReason            | 枚举 (FinishReason)       | 模型停止生成令牌的原因。          | STOP, MAX\_TOKENS, SAFETY等。            |
| candidates.safetyRatings           | 对象数组 (SafetyRating)   | 响应候选安全性的评级列表。        | 每个类别最多一个评级。                   |
| candidates.citationMetadata        | 对象 (CitationMetadata)   | 生成内容的来源归因。              |                                          |
| candidates.groundingMetadata       | 对象 (GroundingMetadata)  | 指定用于内容基础的来源元数据。    |                                          |
| candidates.urlContextMetadata      | 对象 (UrlContextMetadata) | 与URL上下文检索工具相关的元数据。 |                                          |
| candidates.finishMessage           | 字符串                    | 停止原因的更详细描述。            | 仅当finishReason设置时填充。             |
| usageMetadata.promptTokenCount     | 整数                      | 请求中的令牌数。                  |                                          |
| usageMetadata.candidatesTokenCount | 整数                      | 响应中的令牌数。                  |                                          |
| usageMetadata.totalTokenCount      | 整数                      | 总令牌数。                        |                                          |
| promptFeedback.blockReason         | 枚举 (BlockedReason)      | 阻止原因。                        | SAFETY, OTHER, BLOCKLIST等。             |
| promptFeedback.safetyRatings       | 对象数组 (SafetyRating)   | 安全评级。                        |                                          |

Google Gemini API在安全和基础机制方面表现出全面性。safetySettings 7 和

promptFeedback 22 字段，以及详细的

SafetyRating、HarmProbability和HarmSeverity枚举 22，都表明安全功能被深度集成。此外，包含

webSearchQueries、groundingChunks（网络、检索上下文、地图）和citationMetadata 22 的广泛

groundingMetadata提供了关于信息检索的丰富上下文。这种设计表明Google对负责任的AI开发和可解释性有着坚定的承诺。详细的安全反馈使开发者能够理解内容被阻止的*原因*，从而改进提示工程和用户体验。基础信息对于构建事实准确和可验证的AI应用程序至关重要，有助于解决幻觉问题。这种对安全性和事实准确性的透明度和控制是Gemini的一个显著优势，表明其专注于企业和生产级应用，在这些应用中，可靠性和信任至关重要。

### **4.3. 流式响应格式 (models.streamGenerateContent)**

Gemini API通过models.streamGenerateContent方法支持流式响应 5。与OpenAI和Anthropic明确的SSE事件类型不同，Gemini的流式传输似乎以增量

GenerateContentResponse对象的形式交付 5。

**方法概述和增量响应交付：**

streamGenerateContent方法在生成时产生数据块 5。每个数据块都是一个

GenerateContentResponse对象，但它只包含响应中*新的*或*更新的*部分。对于文本，这意味着content.parts.text将包含增量字符串片段 5。这些片段需要拼接以形成完整的文本。其他字段，如

safetyRatings、citationMetadata、usageMetadata，可能会出现在第一个数据块中，或者在后续数据块中更新，具体取决于该信息何时可用。累积的流式响应的整体结构将与同步GenerateContentResponse对象匹配。

**与同步响应的区别：**

核心对象（GenerateContentResponse）保持不变，但其字段在多个数据块中增量填充。像promptFeedback这样的字段明确声明“仅在第一个流块中发送” 22。

Candidate对象中的FinishReason将为空，直到模型停止为该候选生成令牌 22。

Candidate中的content字段将包含需要聚合的部分文本或工具输出。

**表10：Google Gemini API \- 流式响应字段（增量）**

| 字段路径                      | 类型                      | 描述                              | 在流中如何出现（例如，完整对象、部分更新、仅在第一个块中、累积）               |
| :---------------------------- | :------------------------ | :-------------------------------- | :----------------------------------------------------------------------------- |
| candidates                    | 对象数组 (Candidate)      | 生成的候选响应。                  | 增量更新，每个块可能包含新的Candidate或现有Candidate的部分更新。               |
| candidates.content.parts.text | 字符串                    | 候选的文本内容。                  | 文本片段，需要客户端拼接以形成完整文本。                                       |
| candidates.finishReason       | 枚举 (FinishReason)       | 模型停止生成令牌的原因。          | 在生成停止时，在相应Candidate的最终更新中填充。在此之前为null或未定义。        |
| promptFeedback                | 对象 (PromptFeedback)     | 提示的内容过滤结果。              | 仅在第一个流块中发送，并且仅在由于内容违规而未生成任何候选时出现。             |
| usageMetadata                 | 对象 (UsageMetadata)      | 关于响应的使用元数据。            | 可能会在第一个块中出现，或在后续块中更新（例如，candidatesTokenCount会累积）。 |
| modelVersion                  | 字符串                    | 用于生成响应的模型版本。          | 通常在第一个块中出现。                                                         |
| createTime                    | 字符串 (Timestamp)        | 请求发送到服务器的时间戳。        | 通常在第一个块中出现。                                                         |
| responseId                    | 字符串                    | 标识每个响应的ID。                | 通常在第一个块中出现。                                                         |
| candidates.safetyRatings      | 对象数组 (SafetyRating)   | 响应候选安全性的评级列表。        | 可能会在第一个块中出现，或在后续块中更新。                                     |
| candidates.citationMetadata   | 对象 (CitationMetadata)   | 生成内容的来源归因。              | 可能会在第一个块中出现，或在后续块中更新。                                     |
| candidates.groundingMetadata  | 对象 (GroundingMetadata)  | 指定用于内容基础的来源元数据。    | 可能会在第一个块中出现，或在后续块中更新。                                     |
| candidates.urlContextMetadata | 对象 (UrlContextMetadata) | 与URL上下文检索工具相关的元数据。 | 可能会在第一个块中出现，或在后续块中更新。                                     |

## **5\. 聊天完成API的比较分析**

本节将直接比较OpenAI、Anthropic和Google Gemini这三大API，基于前述的详细规范。

### **请求参数语义和命名约定的比较**

* **消息结构：** 所有API都使用messages或contents数组来表示对话历史。OpenAI和Anthropic使用role（system、user、assistant），而Gemini使用role（user、model）。OpenAI和Anthropic使用content字段表示消息文本，而Gemini则在content内使用parts数组。
* **系统提示：** Anthropic拥有独立的system参数。OpenAI将system角色消息集成到messages数组中。Gemini则使用systemInstruction作为一个顶级的Content对象。
* **温度/Top-P/Top-K：** 所有API都提供类似参数，但范围和默认值可能略有不同。
* **停止序列：** 所有API都支持自定义停止序列。
* **工具/函数调用：** 所有API都支持，但实现方式各异：
  * **OpenAI：** tools数组包含function类型，并有tool\_choice参数。响应中包含tool\_calls。
  * **Anthropic：** tools数组包含name、description、input\_schema。响应中包含tool\_use内容块。支持disable\_parallel\_tool\_use。
  * **Gemini：** tools数组包含Function或codeExecution，并有toolConfig。响应中包含function\_call部分。
* **多模态：**
  * **OpenAI：** gpt-4o模型在content数组中支持图像。
  * **Anthropic：** 在content数组中支持base64源类型的图像。
  * **Gemini：** 最全面，通过parts数组中的inline\_data、file\_data、video\_metadata、uri支持图像、音频、视频和文件。

### **同步响应结构和提供信息的差异**

* **根对象：** OpenAI使用chat.completion对象，Anthropic使用message对象，Gemini使用GenerateContentResponse。
* **内容表示：** OpenAI和Anthropic使用content字符串（或Anthropic的内容块数组）。Gemini使用candidates.content.parts表示生成的内容。
* **使用指标：** 所有API都提供令牌计数（prompt\_tokens、completion\_tokens/input\_tokens、output\_tokens）。Anthropic提供高度详细的缓存和服务器工具使用量。Gemini通过UsageMetadata提供promptTokenCount、candidatesTokenCount、totalTokenCount以及多模态令牌详情。
* **停止原因：** 所有API都提供finish\_reason或stop\_reason，值类似。
* **安全/内容审核：** Gemini和Anthropic在响应中明确提供safetyRatings / promptFeedback，详细说明了被阻止的内容和危害概率。OpenAI通过finish\_reason: content\_filter指示内容过滤。
* **基础/引用：** Gemini提供广泛的citationMetadata和groundingMetadata（网络搜索、检索上下文、地图），提供丰富的来源信息。Anthropic和OpenAI在标准聊天完成响应中对基础信息的明确程度较低。

### **流式传输协议（SSE与增量JSON）及其影响分析**

* **OpenAI和Anthropic：** 两者都使用服务器发送事件（SSE），并带有明确的事件类型（如message\_start、content\_block\_delta、message\_delta、message\_stop等）。这种显式的事件驱动方法允许清晰地解析响应生成过程的不同阶段。
* **Gemini：** 流式传输增量GenerateContentResponse对象。虽然在事件类型方面更简单，但它要求客户端通过累积部分JSON对象来重建完整响应，这可能比拼接简单的文本增量更复杂。
* **影响：** 带有明确事件类型（OpenAI、Anthropic）的SSE提供了更清晰的状态转换和对特定内容块（例如Anthropic的thinking\_delta）的更简单解析。Gemini的方法在高级层面可能更易于实现（只需累积JSON），但如果需要在流中对特定字段更新做出反应，则需要更仔细地处理部分对象。

**表11：跨API功能比较矩阵**

| 功能               | OpenAI                                                           | Claude                                                                                                                    | Gemini                                                                                           |
| :----------------- | :--------------------------------------------------------------- | :------------------------------------------------------------------------------------------------------------------------ | :----------------------------------------------------------------------------------------------- |
| **多模态支持**     | gpt-4o在messages.content中支持图像。                             | messages.content中支持base64图像。                                                                                        | contents.parts支持文本、图像、音频、视频、文件、URI，设计最全面。                                |
| **工具调用机制**   | tools数组（function类型），tool\_choice参数。响应中tool\_calls。 | tools数组（name, description, input\_schema），tool\_choice参数。响应中tool\_use内容块。                                  | tools数组（Function, codeExecution），toolConfig。响应中function\_call部分。                     |
| **系统提示处理**   | messages数组中的system角色消息。                                 | 独立的system参数。                                                                                                        | 独立的systemInstruction对象。                                                                    |
| **流式传输协议**   | SSE，具有明确的事件类型（data: {...}, data:）。                  | SSE，具有详细的事件类型（message\_start, content\_block\_delta等）。                                                      | 增量GenerateContentResponse对象。                                                                |
| **最大输入令牌**   | 取决于模型上下文长度。                                           | 取决于模型上下文长度。                                                                                                    | 取决于模型上下文长度。                                                                           |
| **温度范围**       | 0到2。                                                           | 0.0到1.0。                                                                                                                | 0.0到1.0。                                                                                       |
| **安全功能**       | finish\_reason: content\_filter。                                | 响应中明确的safetyRatings和promptFeedback。                                                                               | 响应中明确的safetySettings和promptFeedback，详细的SafetyRating、HarmProbability。                |
| **结构化输出支持** | response\_format参数（json\_object类型）。                       | 不明确，但可通过提示工程实现。                                                                                            | generationConfig.response\_mime\_type和response\_schema。                                        |
| **缓存功能**       | stream\_options.include\_usage用于最终使用统计。                 | usage中提供详细的cache\_creation和cache\_read\_input\_tokens。                                                            | cachedContent参数和usageMetadata.cachedContentTokenCount。                                       |
| **详细使用指标**   | usage包含prompt\_tokens, completion\_tokens, total\_tokens。     | usage包含input\_tokens, output\_tokens, cache\_creation\_input\_tokens, cache\_read\_input\_tokens, server\_tool\_use等。 | usageMetadata包含promptTokenCount, candidatesTokenCount, totalTokenCount, modalityTokenCount等。 |
| **基础/引用**      | 较少明确细节。                                                   | 较少明确细节。                                                                                                            | 广泛的citationMetadata和groundingMetadata（网络搜索、检索上下文、地图）。                        |

通过比较usage字段和流式传输事件，可以观察到API粒度和透明度方面的差异。OpenAI提供了stream\_options.include\_usage用于获取最终使用量统计信息 16。Anthropic提供了高度细粒度的

usage指标，包括缓存和服务器工具使用情况 3，并在流式传输期间提供详细的

thinking\_delta事件 17。Gemini则通过其

UsageMetadata和promptFeedback 22 提供了按模态划分的令牌计数和安全阻止的详细信息。这种差异表明了API透明度和开发者控制的不同理念。Anthropic似乎优先考虑对模型行为（例如“思考”过程）和成本归因（详细缓存）进行深入检查。Gemini则非常注重安全和基础，提供关于内容审核和事实来源的细粒度反馈。OpenAI在提供基本使用量的同时，更倾向于为一般用途提供更精简的输出。这意味着对于需要高可解释性、细粒度成本分析或强大安全/基础功能的应用程序，Anthropic和Gemini可能提供更多开箱即用的功能，而OpenAI则提供更通用、性能更强的接口。

多模态和工具范式的演变也值得关注。尽管所有三家都支持多模态和工具调用，但其API结构反映了不同的演进阶段或设计优先级。Gemini的contents与parts结构 7 感觉上是多模态最内在的，允许在单个消息中包含多样化的输入。Anthropic的

content数组与特定的image和tool\_use块 3 也集成良好。OpenAI对

gpt-4o的多模态支持较新，并集成到现有的content字段中。类似地，工具调用机制具有不同的输入/输出结构。这表明LLM能力正在迅速超越纯文本。构建多模态应用程序的开发者可能会发现Gemini统一的parts结构对于复杂的混合媒体输入更直观。工具调用机制的差异意味着，在这些API之间进行智能体工作流的抽象需要一个强大的中间层来规范工具定义和调用。趋势是朝着更强大、更具智能体的模型发展，但集成模式仍在趋同，这既带来了跨平台开发的机会，也带来了挑战。

最后，对“OpenAI兼容”生态系统的审视揭示了一些细微之处。用户查询中明确提到了“OpenAI兼容格式”。OpenVINO 16、Langdock 13 和OpenRouter 12 等API旨在实现OpenAI兼容性。然而，这些兼容API通常存在细微偏差（例如，Langdock不支持

n或stream\_options，OpenVINO的ignore\_eos或include\_stop\_str\_in\_output不在官方OpenAI规范中） 13。这意味着，虽然兼容性降低了熟悉OpenAI API的开发者的入门门槛，但理解“兼容”并不等同于“相同”至关重要。开发者必须仔细查阅

*每个*兼容提供商的具体文档，因为支持参数、默认值或响应结构中的细微差异可能导致意外行为或限制功能利用。这导致了一个碎片化的生态系统，其中真正的“通用”OpenAI兼容客户端可能需要条件逻辑或功能子集。

## **6\. 结论与建议**

本报告对OpenAI、Anthropic和Google Gemini的聊天完成API进行了深入分析，揭示了它们在请求输入、同步响应和流式响应格式方面的异同。每家提供商都根据其核心设计理念和优先事项，提供了独特的功能集和API范式。

**各API的优势与劣势总结：**

* **OpenAI：**
  * **优势：** 模型选择广泛，社区支持强大，工具调用功能成熟，对于基本文本流式传输相对简单。
  * **劣势：** 与Gemini/Claude相比，显式安全反馈较少。
* **Anthropic (Claude)：**
  * **优势：** 显式对话结构，高级安全功能，模型思考过程的详细可见性，以及全面的令牌使用和缓存指标。
  * **劣势：** 对messages角色有更严格的强制要求。
* **Google Gemini：**
  * **优势：** 原生多模态输入处理能力强，提供广泛的基础/引用功能，细粒度安全控制，以及结构化输出能力。
  * **劣势：** 流式传输事件类型不如OpenAI和Anthropic明确，可能需要更复杂的客户端解析逻辑。

**根据具体集成需求选择API的建议：**

* 对于**通用聊天应用程序**，如果优先考虑易于集成和广泛的模型选择：推荐选择**OpenAI**。
* 对于**需要高度可控性、可解释性（思考过程）和详细成本分析的应用程序**：推荐选择**Anthropic**。
* 对于**多模态应用程序，或需要强大事实基础和结构化输出以进行下游处理的应用程序**：推荐选择**Google Gemini**。
* 对于**智能体应用程序**，应根据每个API的具体工具实现细节及其与编排层的契合度进行考量。
* 对于**流式传输实现**，请注意其不同的范式：OpenAI和Anthropic采用显式SSE事件类型，而Gemini则采用增量对象重建。

最终选择应基于项目的具体需求、团队的熟悉程度以及对特定功能（如多模态、工具调用、安全控制或成本透明度）的重视程度。

#### **引用的著作**

1. Azure OpenAI in Azure AI Foundry Models REST API reference \- Learn Microsoft, 访问时间为 七月 27, 2025， [https://learn.microsoft.com/en-us/azure/ai-foundry/openai/reference](https://learn.microsoft.com/en-us/azure/ai-foundry/openai/reference)
2. Claude API | Documentation | Postman API Network, 访问时间为 七月 27, 2025， [https://www.postman.com/postman/anthropic-apis/documentation/dhus72s/claude-api](https://www.postman.com/postman/anthropic-apis/documentation/dhus72s/claude-api)
3. Messages \- Anthropic, 访问时间为 七月 27, 2025， [https://docs.anthropic.com/en/api/messages](https://docs.anthropic.com/en/api/messages)
4. Gemini API | Google AI for Developers, 访问时间为 七月 27, 2025， [https://ai.google.dev/gemini-api/docs](https://ai.google.dev/gemini-api/docs)
5. @google/genai \- The GitHub pages site for the googleapis organization., 访问时间为 七月 27, 2025， [https://googleapis.github.io/js-genai/](https://googleapis.github.io/js-genai/)
6. Text generation | Gemini API | Google AI for Developers, 访问时间为 七月 27, 2025， [https://ai.google.dev/gemini-api/docs/text-generation](https://ai.google.dev/gemini-api/docs/text-generation)
7. Generating content | Gemini API | Google AI for Developers, 访问时间为 七月 27, 2025， [https://ai.google.dev/api/generate-content](https://ai.google.dev/api/generate-content)
8. Vertex AI GenAI API | Generative AI on Vertex AI \- Google Cloud, 访问时间为 七月 27, 2025， [https://cloud.google.com/vertex-ai/generative-ai/docs/reference/rest](https://cloud.google.com/vertex-ai/generative-ai/docs/reference/rest)
9. All methods | Gemini API | Google AI for Developers, 访问时间为 七月 27, 2025， [https://ai.google.dev/api/all-methods](https://ai.google.dev/api/all-methods)
10. Openai /v1/completions vs. /v1/chat/completions end points \- Stack Overflow, 访问时间为 七月 27, 2025， [https://stackoverflow.com/questions/76192496/openai-v1-completions-vs-v1-chat-completions-end-points](https://stackoverflow.com/questions/76192496/openai-v1-completions-vs-v1-chat-completions-end-points)
11. How to call the chat completion api from node? \- OpenAI \- Reddit, 访问时间为 七月 27, 2025， [https://www.reddit.com/r/OpenAI/comments/11fxfpz/how\_to\_call\_the\_chat\_completion\_api\_from\_node/](https://www.reddit.com/r/OpenAI/comments/11fxfpz/how_to_call_the_chat_completion_api_from_node/)
12. Chat completion | OpenRouter | Documentation, 访问时间为 七月 27, 2025， [https://openrouter.ai/docs/api-reference/chat-completion](https://openrouter.ai/docs/api-reference/chat-completion)
13. OpenAI Chat completion \- Documentation, 访问时间为 七月 27, 2025， [https://docs.langdock.com/api-endpoints/completion/openai](https://docs.langdock.com/api-endpoints/completion/openai)
14. REST API Reference \- xAI Docs, 访问时间为 七月 27, 2025， [https://docs.x.ai/docs/api-reference](https://docs.x.ai/docs/api-reference)
15. Model \- OpenAI API, 访问时间为 七月 27, 2025， [https://platform.openai.com/docs/models/gpt-4o](https://platform.openai.com/docs/models/gpt-4o)
16. OpenAI API chat/completions endpoint \- OpenVINO™ documentation, 访问时间为 七月 27, 2025， [https://docs.openvino.ai/2025/model-server/ovms\_docs\_rest\_api\_chat.html](https://docs.openvino.ai/2025/model-server/ovms_docs_rest_api_chat.html)
17. Streaming Messages \- Anthropic, 访问时间为 七月 27, 2025， [https://docs.anthropic.com/en/api/messages-streaming](https://docs.anthropic.com/en/api/messages-streaming)
18. Gemini API reference | Google AI for Developers, 访问时间为 七月 27, 2025， [https://ai.google.dev/api](https://ai.google.dev/api)
19. deprecated-generative-ai-python/docs/api/google/generativeai ..., 访问时间为 七月 27, 2025， [https://github.com/google-gemini/generative-ai-python/blob/main/docs/api/google/generativeai/protos.md](https://github.com/google-gemini/generative-ai-python/blob/main/docs/api/google/generativeai/protos.md)
20. googleapis/python-genai: Google Gen AI Python SDK provides an interface for developers to integrate Google's generative models into their Python applications. \- GitHub, 访问时间为 七月 27, 2025， [https://github.com/googleapis/python-genai](https://github.com/googleapis/python-genai)
21. Generate structured output (like JSON and enums) using the Gemini API | Firebase AI Logic, 访问时间为 七月 27, 2025， [https://firebase.google.com/docs/ai-logic/generate-structured-output](https://firebase.google.com/docs/ai-logic/generate-structured-output)
22. GenerateContentResponse | Generative AI on Vertex AI | Google ..., 访问时间为 七月 27, 2025， [https://cloud.google.com/vertex-ai/generative-ai/docs/reference/rest/v1/GenerateContentResponse](https://cloud.google.com/vertex-ai/generative-ai/docs/reference/rest/v1/GenerateContentResponse)
23. Generate streaming text content with Generative Model | Generative AI on Vertex AI | Google Cloud, 访问时间为 七月 27, 2025， [https://cloud.google.com/vertex-ai/generative-ai/docs/samples/googlegenaisdk-textgen-with-txt-stream](https://cloud.google.com/vertex-ai/generative-ai/docs/samples/googlegenaisdk-textgen-with-txt-stream)