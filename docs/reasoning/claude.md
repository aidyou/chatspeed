---
title: 思考
url: https://platform.claude.com/docs/zh-CN/build-with-claude/thinking
description: 了解 Claude 的思考功能如何运作：如何启用、读取思考输出、通过 effort 调节思考深度，以及如何将思考与工具、缓存和流式传输结合使用。
---

<Note>
  关于"zero data retention"（零数据保留），即 ZDR 如何适用于此功能，请参阅 [API 与数据保留](https://platform.claude.com/docs/zh-CN/manage-claude/api-and-data-retention)。
</Note>

一个单次生成答案的模型必须在第一次尝试时就把所有事情做对：没有草稿、没有检查、也无法中途改变方向。对于数学证明、棘手的 bug 或长时间运行的智能体任务，第一种方法往往不是最佳方法。

思考功能消除了这一限制。当思考功能处于活动状态时，Claude 会在回答之前用自己的语言梳理问题：它会重述所问的内容、尝试不同的方法、检查中间结果，并放弃那些站不住脚的路径。这些推理内容会以 `thinking` 内容块的形式出现在响应之前，Claude 会基于这些推理来生成最终答案。这就是为什么思考功能能够提升复杂任务（如数学、编程、分析和长时间运行的智能体工作）的表现——在这些任务中，答案的质量取决于中间工作，而这些中间工作原本会被压缩到响应本身中或被跳过。

思考是有成本的：Claude 用于推理的令牌会按输出令牌计费，即使思考文本没有返回给您也是如此，并且它们与响应文本一起计入 `max_tokens`。本页介绍思考功能在 API 层面的行为：如何启用、如何读取其输出，以及如何管理它与工具、流式传输、缓存和上下文窗口之间的交互。

## 思考的工作原理

![思考工作原理示意图：Claude 评估请求并决定是否思考；在使用工具时，思考可以在工具调用之间反复出现；一个响应先返回 thinking 块，然后返回 text 块](https://platform.claude.com/docs/images/how-thinking-works.svg)

Claude 是否对给定请求进行思考以及思考的深度，取决于您的思考配置和请求的复杂程度。

以下是思考在响应中的呈现方式：一个或多个 `thinking` 内容块会出现在 `text` 块之前。思考块仍然是生成的内容，就像随后的 `text` 块一样，但它与规范响应是分开的。每个思考块还带有一个 `signature` 字段，这是完整推理的加密副本，您需要在多轮对话和工具使用对话中原样传回（参见[思考加密](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#thinking-encryption)）：

```json
{
  "content": [
    {
      "type": "thinking",
      "thinking": "Let me break this down. The question has two parts, so I'll start with the simpler one and use its result to constrain the second...",
      "signature": "WaUjzkypQ2mUEVM36O2Txu...."
    },
    {
      "type": "text",
      "text": "Based on my analysis..."
    }
  ]
}
```

您并不总能看到这些文本，而且您看到的永远不是原始的思维链：思考块中的文本是 [Claude 推理的摘要](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#summarized-thinking)。思考配置中的 `display` 字段控制是否返回该摘要：`"summarized"` 会返回摘要，而 `"omitted"`（最新模型的默认值）会返回 `thinking` 字段为空的思考块。无论哪种方式，该块的计费方式相同，在多轮对话中的传回方式也相同。有关各模型的默认值和详细信息，请参见[控制思考显示](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#controlling-thinking-display)。

如果 Claude 使用工具，思考也可能出现在工具调用之间。参见[思考与工具使用](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#thinking-with-tool-use)。有关完整的响应格式，请参见 [Messages API 参考](https://platform.claude.com/docs/zh-CN/api/messages/create)。

## 配置思考

在当前模型上，思考功能默认开启或只需一个参数即可开启。每个模型接受的配置及其默认值列在故障排除页面的[各模型配置表](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking-troubleshooting#supported-models)中。

在 Claude Opus 5、Claude Sonnet 5、Claude Fable 5、Claude Mythos 5 和 Claude Mythos Preview 上，思考功能已默认开启：无需配置。大多数开发者在这些模型上首先需要的是查看思考文本，因为这些模型的 `display` 默认为 `"omitted"`。通过 `thinking: {"type": "adaptive", "display": "summarized"}` 选择加入，这正是以下请求（只需替换[模型字符串](https://platform.claude.com/docs/zh-CN/about-claude/models/overview)）。

在 Claude Opus 4.8、Claude Opus 4.7、Claude Opus 4.6 和 Claude Sonnet 4.6 上，思考功能默认关闭，直到您设置 `thinking: {type: "adaptive"}`，这会让 Claude 根据请求决定何时思考以及思考的深度。以下示例执行此操作，设置 `display: "summarized"` 以使思考文本可见，并使用较大的 `max_tokens`：

<CodeGroup>
  ```bash cURL
  curl https://api.anthropic.com/v1/messages \
    -H "x-api-key: $ANTHROPIC_API_KEY" \
    -H "anthropic-version: 2023-06-01" \
    -H "content-type: application/json" \
    -d '{
      "model": "claude-opus-4-8",
      "max_tokens": 16000,
      "thinking": {
        "type": "adaptive",
        "display": "summarized"
      },
      "messages": [
        {
          "role": "user",
          "content": "What is the greatest common divisor of 1071 and 462?"
        }
      ]
    }'
  ```

  ```bash CLI
  ant messages create \
    --model claude-opus-4-8 \
    --max-tokens 16000 \
    --thinking '{type: adaptive, display: summarized}' \
    --message '{role: user, content: "What is the greatest common divisor of 1071 and 462?"}' \
    --transform content \
    --format yaml
  ```

  ```python Python
  client = anthropic.Anthropic()

  response = client.messages.create(
      model="claude-opus-4-8",
      max_tokens=16000,
      thinking={"type": "adaptive", "display": "summarized"},
      messages=[
          {
              "role": "user",
              "content": "What is the greatest common divisor of 1071 and 462?",
          }
      ],
  )

  for block in response.content:
      if block.type == "thinking":
          print(f"\nThinking: {block.thinking}")
      elif block.type == "text":
          print(f"\nResponse: {block.text}")
  ```

  ```typescript TypeScript
  const client = new Anthropic();

  const response = await client.messages.create({
    model: "claude-opus-4-8",
    max_tokens: 16000,
    thinking: {
      type: "adaptive",
      display: "summarized"
    },
    messages: [
      {
        role: "user",
        content: "What is the greatest common divisor of 1071 and 462?"
      }
    ]
  });

  for (const block of response.content) {
    if (block.type === "thinking") {
      console.log(`\nThinking: ${block.thinking}`);
    } else if (block.type === "text") {
      console.log(`\nResponse: ${block.text}`);
    }
  }
  ```

  ```csharp C#
  AnthropicClient client = new();

  var parameters = new MessageCreateParams
  {
      Model = Model.ClaudeOpus4_8,
      MaxTokens = 16000,
      Thinking = new ThinkingConfigAdaptive { Display = Display.Summarized },
      Messages = [
          new() {
              Role = Role.User,
              Content = "What is the greatest common divisor of 1071 and 462?"
          }
      ]
  };

  var message = await client.Messages.Create(parameters);

  foreach (var block in message.Content)
  {
      if (block.TryPickThinking(out ThinkingBlock? thinking))
      {
          Console.WriteLine($"\nThinking: {thinking.Thinking}");
      }
      else if (block.TryPickText(out TextBlock? text))
      {
          Console.WriteLine($"\nResponse: {text.Text}");
      }
  }
  ```

  ```go Go
  client := anthropic.NewClient()

  response, err := client.Messages.New(context.TODO(), anthropic.MessageNewParams{
  	Model:     anthropic.ModelClaudeOpus4_8,
  	MaxTokens: 16000,
  	Thinking: anthropic.ThinkingConfigParamUnion{
  		OfAdaptive: &anthropic.ThinkingConfigAdaptiveParam{
  			Display: anthropic.ThinkingConfigAdaptiveDisplaySummarized,
  		},
  	},
  	Messages: []anthropic.MessageParam{
  		anthropic.NewUserMessage(anthropic.NewTextBlock("What is the greatest common divisor of 1071 and 462?")),
  	},
  })
  if err != nil {
  	log.Fatal(err)
  }

  for _, block := range response.Content {
  	switch v := block.AsAny().(type) {
  	case anthropic.ThinkingBlock:
  		fmt.Printf("\nThinking: %s", v.Thinking)
  	case anthropic.TextBlock:
  		fmt.Printf("\nResponse: %s", v.Text)
  	}
  }
  ```

  ```java Java
  import com.anthropic.models.messages.ThinkingConfigAdaptive;

  void main() {
      AnthropicClient client = AnthropicOkHttpClient.fromEnv();

      MessageCreateParams params = MessageCreateParams.builder()
          .model(Model.CLAUDE_OPUS_4_8)
          .maxTokens(16000L)
          .thinking(ThinkingConfigAdaptive.builder()
              .display(ThinkingConfigAdaptive.Display.SUMMARIZED)
              .build())
          .addUserMessage("What is the greatest common divisor of 1071 and 462?")
          .build();

      Message response = client.messages().create(params);

      response.content().forEach(block -> {
          block.thinking().ifPresent(thinkingBlock ->
              IO.println("\nThinking: " + thinkingBlock.thinking())
          );
          block.text().ifPresent(textBlock ->
              IO.println("\nResponse: " + textBlock.text())
          );
      });
  }
  ```

  ```php PHP
  $client = new Client();

  $message = $client->messages->create(
      maxTokens: 16000,
      messages: [
          [
              'role' => 'user',
              'content' => 'What is the greatest common divisor of 1071 and 462?'
          ]
      ],
      model: 'claude-opus-4-8',
      thinking: ['type' => 'adaptive', 'display' => 'summarized'],
  );

  foreach ($message->content as $block) {
      if ($block->type === 'thinking') {
          echo "\nThinking: " . $block->thinking;
      } elseif ($block->type === 'text') {
          echo "\nResponse: " . $block->text;
      }
  }
  ```

  ```ruby Ruby
  client = Anthropic::Client.new

  message = client.messages.create(
    model: "claude-opus-4-8",
    max_tokens: 16000,
    thinking: {
      type: "adaptive",
      display: "summarized"
    },
    messages: [
      {
        role: "user",
        content: "What is the greatest common divisor of 1071 and 462?"
      }
    ]
  )

  message.content.each do |block|
    case block.type
    when :thinking
      puts "\nThinking: #{block.thinking}"
    when :text
      puts "\nResponse: #{block.text}"
    end
  end
  ```
</CodeGroup>

运行该示例会先打印摘要化的思考内容，然后打印答案：

```text Output wrap
Thinking: Use Euclidean algorithm.
1071 = 2*462 + 147
462 = 3*147 + 21
147 = 7*21 + 0
GCD = 21

Response: ## Finding GCD of 1071 and 462

I'll use the **Euclidean algorithm**, repeatedly dividing and taking remainders...
```

思考令牌计入 `max_tokens`，因此请将其设置得足够高，以便为思考和响应文本都留出空间。请参见调节页面上的[成本控制](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking-steering-and-cost#cost-control)以及[思考与上下文窗口](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#thinking-and-the-context-window)。

### 关闭思考

在 Claude Sonnet 5 上，思考功能默认开启，您可以将其关闭：

<CodeGroup>
  ```bash cURL
  curl https://api.anthropic.com/v1/messages \
    -H "x-api-key: $ANTHROPIC_API_KEY" \
    -H "anthropic-version: 2023-06-01" \
    -H "content-type: application/json" \
    -d '{
      "model": "claude-sonnet-5",
      "max_tokens": 4096,
      "thinking": {"type": "disabled"},
      "messages": [
        {
          "role": "user",
          "content": "Summarize this article in one sentence."
        }
      ]
    }'
  ```

  ```bash CLI
  ant messages create \
    --model claude-sonnet-5 \
    --max-tokens 4096 \
    --thinking '{type: disabled}' \
    --message '{role: user, content: "Summarize this article in one sentence."}'
  ```

  ```python Python
  client = anthropic.Anthropic()

  response = client.messages.create(
      model="claude-sonnet-5",
      max_tokens=4096,
      thinking={"type": "disabled"},
      messages=[{"role": "user", "content": "Summarize this article in one sentence."}],
  )
  ```

  ```typescript TypeScript
  const client = new Anthropic();

  const response = await client.messages.create({
    model: "claude-sonnet-5",
    max_tokens: 4096,
    thinking: { type: "disabled" },
    messages: [{ role: "user", content: "Summarize this article in one sentence." }]
  });
  ```

  ```csharp C#
  AnthropicClient client = new();

  var parameters = new MessageCreateParams
  {
      Model = Model.ClaudeSonnet5,
      MaxTokens = 4096,
      Thinking = new ThinkingConfigDisabled(),
      Messages = [
          new() {
              Role = Role.User,
              Content = "Summarize this article in one sentence."
          }
      ]
  };

  var message = await client.Messages.Create(parameters);
  ```

  ```go Go
  client := anthropic.NewClient()

  response, err := client.Messages.New(context.TODO(), anthropic.MessageNewParams{
  	Model:     anthropic.ModelClaudeSonnet5,
  	MaxTokens: 4096,
  	Thinking: anthropic.ThinkingConfigParamUnion{
  		OfDisabled: &anthropic.ThinkingConfigDisabledParam{},
  	},
  	Messages: []anthropic.MessageParam{
  		anthropic.NewUserMessage(anthropic.NewTextBlock("Summarize this article in one sentence.")),
  	},
  })
  if err != nil {
  	log.Fatal(err)
  }
  ```

  ```java Java
  import com.anthropic.models.messages.ThinkingConfigDisabled;

  void main() {
      AnthropicClient client = AnthropicOkHttpClient.fromEnv();

      MessageCreateParams params = MessageCreateParams.builder()
          .model(Model.CLAUDE_SONNET_5)
          .maxTokens(4096L)
          .thinking(ThinkingConfigDisabled.builder().build())
          .addUserMessage("Summarize this article in one sentence.")
          .build();

      Message response = client.messages().create(params);
  }
  ```

  ```php PHP
  $client = new Client();

  $message = $client->messages->create(
      maxTokens: 4096,
      messages: [
          [
              'role' => 'user',
              'content' => 'Summarize this article in one sentence.'
          ]
      ],
      model: 'claude-sonnet-5',
      thinking: ['type' => 'disabled'],
  );
  ```

  ```ruby Ruby
  client = Anthropic::Client.new

  message = client.messages.create(
    model: "claude-sonnet-5",
    max_tokens: 4096,
    thinking: { type: "disabled" },
    messages: [
      {
        role: "user",
        content: "Summarize this article in one sentence."
      }
    ]
  )
  ```
</CodeGroup>

Claude Opus 5 也默认开启思考功能，并在 [effort](https://platform.claude.com/docs/zh-CN/build-with-claude/effort) 为 `high` 或更低时接受 `thinking: {type: "disabled"}`。在 `xhigh` 或 `max` effort 级别下，思考功能无法关闭：将 `thinking: {type: "disabled"}` 与这些 effort 级别组合的请求会返回 400 错误。此限制适用于 Claude Opus 5 及更高版本的模型，并在每个请求上强制执行。禁用思考后，Claude Opus 5 偶尔会将工具调用作为纯文本输出，或在其可见输出中包含内部 XML 标签。有关提示缓解措施，请参见[在禁用思考的情况下运行](https://platform.claude.com/docs/zh-CN/build-with-claude/prompt-engineering/prompting-claude-opus-5#running-with-thinking-disabled)。

Claude Fable 5、Claude Mythos 5 和 Claude Mythos Preview 会拒绝 `thinking: {type: "disabled"}`：这些模型无法关闭思考功能。

如果您的模型仅支持扩展思考（参见[各模型配置表](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking-troubleshooting#supported-models)），请改用 `type: "enabled"` 和 `budget_tokens` 值进行配置。[扩展思考](https://platform.claude.com/docs/zh-CN/build-with-claude/extended-thinking)页面介绍了该配置。如果任何思考配置返回 400 错误，[思考故障排除](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking-troubleshooting)会将每条错误消息与其修复方法对应起来。

## 读取思考输出

### 控制思考显示

思考配置中的 `display` 字段控制思考内容在 API 响应中的返回方式。`display` 在两种模式下都有效：可与 `type: "adaptive"` 或 `type: "enabled"` 一起设置。它接受两个值：

* `"summarized"`：思考块包含[摘要化思考](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#summarized-thinking)文本，即 Claude 推理的可读摘要。这是 Claude Opus 4.6、Claude Sonnet 4.6 及更早模型的默认值。
* `"omitted"`：返回的思考块中 `thinking` 字段为空。`signature` 字段仍携带加密的完整思考内容，以保持多轮对话的连续性（参见[思考加密](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#thinking-encryption)）。这是 Claude Fable 5、Claude Mythos 5、Claude Opus 5、Claude Sonnet 5、Claude Opus 4.8、Claude Opus 4.7 和 [Claude Mythos Preview](https://anthropic.com/glasswing) 的默认值。

当您的应用程序不向用户展示思考内容时，请设置 `display: "omitted"`。主要好处是在流式传输时更快地获得首个文本令牌：服务器完全跳过思考令牌的流式传输，只传递签名，因此最终文本响应会更早开始流式传输。

使用 `display: "omitted"` 时，响应包含 `thinking` 字段为空的 `thinking` 块：

```json Output
{
  "content": [
    {
      "type": "thinking",
      "thinking": "",
      "signature": "EosnCkYICxIMMb3LzNrMu..."
    },
    {
      "type": "text",
      "text": "The answer is 12,231."
    }
  ]
}
```

使用省略思考时，请注意以下几点：

* 您仍需为完整的思考令牌付费。省略可以降低延迟，但不会降低成本。
* 如果您在多轮对话中传回思考块，请原样传回。服务器会解密 `signature` 以重建原始思考内容用于构建提示（参见[保留思考块](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#preserving-thinking-blocks)）。您在往返传递的省略块的 `thinking` 字段中放置的任何文本都会被忽略。
* `display` 与 `thinking.type: "disabled"` 一起使用无效（因为没有内容可显示）。
* 当使用 `thinking.type: "adaptive"` 且模型对简单请求跳过思考时，无论 `display` 如何设置，都不会生成思考块。
* 使用 `display: "omitted"` 进行流式传输时，不会发出 `thinking_delta` 事件。有关事件序列，请参见[流式传输思考](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#streaming-thinking)。

<Note>
  无论 `display` 是 `"summarized"` 还是 `"omitted"`，`signature` 字段都是相同的。支持在对话的不同轮次之间切换 `display` 值。
</Note>

在 Ruby SDK 中，普通哈希使用 `display:`，如示例所示。类型化的 `ThinkingConfigAdaptive` 类将该参数命名为 `display_`（带尾部下划线，以避免遮蔽 Ruby 的 `Kernel#display`）。无论哪种方式，传输字段仍然是 `display`。

### 摘要化思考

当 `display` 为 `"summarized"` 时，您收到的思考文本是 Claude 完整思考过程的摘要，而不是原始的思维链。摘要化思考提供了思考的全部智能优势，同时防止滥用。没有任何 `display` 设置会返回原始思维链。

使用摘要化思考时，请注意以下几点：

* 您需要为原始请求生成的完整思考令牌付费，而不是摘要令牌。计费的输出令牌数与您在响应中看到的令牌数不匹配。
* 在 Claude Opus 4.6、Claude Sonnet 4.6 及更早的模型上，思考输出的前几行更为详细，提供了对提示工程特别有帮助的详细推理。[Claude Mythos Preview](https://anthropic.com/glasswing) 从第一个令牌开始就进行摘要，因此其思考块不会显示这种详细的前导内容。
* 摘要化以最小的额外延迟保留了 Claude 思考过程的关键思路，因此摘要可以在到达时进行流式传输。
* 摘要化由与您在请求中指定的模型不同的模型处理。思考模型不会看到摘要化的输出。
* 随着 Anthropic 不断改进思考功能，摘要化行为可能会发生变化。

<Note>
  在极少数需要访问完整思考输出的情况下，请[联系 Anthropic 销售团队](mailto:sales@anthropic.com)。
</Note>

### 流式传输思考

思考功能可与[流式传输](https://platform.claude.com/docs/zh-CN/build-with-claude/streaming)配合使用。思考块以 `content_block_delta` 事件内的 `thinking_delta` 事件形式进行流式传输，随后在该块的 `content_block_stop` 之前有一个单独的 `signature_delta` 事件。之后文本块照常进行流式传输。

![带思考的流式传输事件序列示意图：thinking 块打开，仅当 display 为 summarized 时流式传输 thinking delta，单个 signature delta 关闭该块，然后流式传输 text delta](https://platform.claude.com/docs/images/how-thinking-streams.svg)

以下示例使用自适应思考流式传输响应，在思考和文本增量到达时打印它们：

<CodeGroup>
  ```bash cURL
  curl https://api.anthropic.com/v1/messages \
    -H "x-api-key: $ANTHROPIC_API_KEY" \
    -H "anthropic-version: 2023-06-01" \
    -H "content-type: application/json" \
    -d '{
      "model": "claude-opus-4-8",
      "max_tokens": 16000,
      "stream": true,
      "thinking": {
        "type": "adaptive",
        "display": "summarized"
      },
      "messages": [
        {
          "role": "user",
          "content": "What is the greatest common divisor of 1071 and 462?"
        }
      ]
    }'
  ```

  ```bash CLI
  ant messages create \
    --model claude-opus-4-8 \
    --max-tokens 16000 \
    --thinking '{type: adaptive, display: summarized}' \
    --message '{role: user, content: "What is the greatest common divisor of 1071 and 462?"}' \
    --stream \
    --format jsonl
  ```

  ```python Python
  client = anthropic.Anthropic()

  with client.messages.stream(
      model="claude-opus-4-8",
      max_tokens=16000,
      thinking={"type": "adaptive", "display": "summarized"},
      messages=[
          {
              "role": "user",
              "content": "What is the greatest common divisor of 1071 and 462?",
          }
      ],
  ) as stream:
      for event in stream:
          if event.type == "content_block_start":
              print(f"\nStarting {event.content_block.type} block...")
          elif event.type == "content_block_delta":
              if event.delta.type == "thinking_delta":
                  print(event.delta.thinking, end="", flush=True)
              elif event.delta.type == "text_delta":
                  print(event.delta.text, end="", flush=True)
  ```

  ```typescript TypeScript
  const client = new Anthropic();

  const stream = client.messages.stream({
    model: "claude-opus-4-8",
    max_tokens: 16000,
    thinking: { type: "adaptive", display: "summarized" },
    messages: [{ role: "user", content: "What is the greatest common divisor of 1071 and 462?" }]
  });

  for await (const event of stream) {
    if (event.type === "content_block_start") {
      console.log(`\nStarting ${event.content_block.type} block...`);
    } else if (event.type === "content_block_delta") {
      if (event.delta.type === "thinking_delta") {
        process.stdout.write(event.delta.thinking);
      } else if (event.delta.type === "text_delta") {
        process.stdout.write(event.delta.text);
      }
    }
  }
  ```

  ```csharp C#
  AnthropicClient client = new();

  var parameters = new MessageCreateParams
  {
      Model = Model.ClaudeOpus4_8,
      MaxTokens = 16000,
      Thinking = new ThinkingConfigAdaptive { Display = Display.Summarized },
      Messages = [new() { Role = Role.User, Content = "What is the greatest common divisor of 1071 and 462?" }]
  };

  await foreach (var rawEvent in client.Messages.CreateStreaming(parameters))
  {
      if (rawEvent.TryPickContentBlockStart(out var start))
      {
          Console.WriteLine($"\nStarting {start.ContentBlock.Type} block...");
      }
      else if (rawEvent.TryPickContentBlockDelta(out var delta))
      {
          if (delta.Delta.TryPickThinking(out var thinkingDelta))
          {
              Console.Write(thinkingDelta.Thinking);
          }
          else if (delta.Delta.TryPickText(out var textDelta))
          {
              Console.Write(textDelta.Text);
          }
      }
  }
  ```

  ```go Go
  client := anthropic.NewClient()

  stream := client.Messages.NewStreaming(context.TODO(), anthropic.MessageNewParams{
  	Model:     anthropic.ModelClaudeOpus4_8,
  	MaxTokens: 16000,
  	Thinking: anthropic.ThinkingConfigParamUnion{
  		OfAdaptive: &anthropic.ThinkingConfigAdaptiveParam{
  			Display: anthropic.ThinkingConfigAdaptiveDisplaySummarized,
  		},
  	},
  	Messages: []anthropic.MessageParam{
  		anthropic.NewUserMessage(anthropic.NewTextBlock("What is the greatest common divisor of 1071 and 462?")),
  	},
  })

  for stream.Next() {
  	event := stream.Current()
  	switch eventVariant := event.AsAny().(type) {
  	case anthropic.ContentBlockStartEvent:
  		fmt.Printf("\nStarting %s block...\n", eventVariant.ContentBlock.Type)
  	case anthropic.ContentBlockDeltaEvent:
  		switch deltaVariant := eventVariant.Delta.AsAny().(type) {
  		case anthropic.ThinkingDelta:
  			fmt.Print(deltaVariant.Thinking)
  		case anthropic.TextDelta:
  			fmt.Print(deltaVariant.Text)
  		}
  	}
  }
  if err := stream.Err(); err != nil {
  	log.Fatal(err)
  }
  ```

  ```java Java
  import com.anthropic.models.messages.ThinkingConfigAdaptive;

  void main() {
      AnthropicClient client = AnthropicOkHttpClient.fromEnv();

      MessageCreateParams params = MessageCreateParams.builder()
          .model(Model.CLAUDE_OPUS_4_8)
          .maxTokens(16000L)
          .thinking(ThinkingConfigAdaptive.builder()
              .display(ThinkingConfigAdaptive.Display.SUMMARIZED)
              .build())
          .addUserMessage("What is the greatest common divisor of 1071 and 462?")
          .build();

      try (var streamResponse = client.messages().createStreaming(params)) {
          streamResponse.stream().forEach(event -> {
              if (event.contentBlockStart().isPresent()) {
                  var startEvent = event.contentBlockStart().get();
                  var block = startEvent.contentBlock();
                  if (block.isThinking()) {
                      IO.println("\nStarting thinking block...");
                  } else if (block.isText()) {
                      IO.println("\nStarting text block...");
                  }
              } else if (event.contentBlockDelta().isPresent()) {
                  var deltaEvent = event.contentBlockDelta().get();
                  deltaEvent.delta().thinking().ifPresent(td ->
                      IO.print(td.thinking())
                  );
                  deltaEvent.delta().text().ifPresent(td ->
                      IO.print(td.text())
                  );
              }
          });
      }
  }
  ```

  ```php PHP
  $client = new Client();

  $stream = $client->messages->createStream(
      maxTokens: 16000,
      messages: [
          ['role' => 'user', 'content' => 'What is the greatest common divisor of 1071 and 462?']
      ],
      model: 'claude-opus-4-8',
      thinking: ['type' => 'adaptive', 'display' => 'summarized'],
  );

  foreach ($stream as $event) {
      if ($event->type === 'content_block_start') {
          echo "\nStarting {$event->contentBlock->type} block...\n";
      } elseif ($event->type === 'content_block_delta') {
          if ($event->delta->type === 'thinking_delta') {
              echo $event->delta->thinking;
          } elseif ($event->delta->type === 'text_delta') {
              echo $event->delta->text;
          }
      }
  }
  ```

  ```ruby Ruby
  client = Anthropic::Client.new

  stream = client.messages.stream(
    model: "claude-opus-4-8",
    max_tokens: 16000,
    thinking: { type: "adaptive", display: "summarized" },
    messages: [
      { role: "user", content: "What is the greatest common divisor of 1071 and 462?" }
    ]
  )

  stream.each do |event|
    case event
    when Anthropic::Streaming::ThinkingEvent
      print event.thinking
    when Anthropic::Streaming::TextEvent
      print event.text
    end
  end
  ```
</CodeGroup>

要在流式传输后重新组装带有签名的完整思考块，请使用您的 SDK 的消息累积辅助工具（如果存在，例如 Python 中的 `stream.get_final_message()` 或 TypeScript 中的 `stream.finalMessage()`），而不是自己拼接增量。

<Accordion title="完整的流式传输事件跟踪">
  ```sse Output
  event: message_start
  data: {"type": "message_start", "message": {"id": "msg_01...", "type": "message", "role": "assistant", "content": [], "model": "claude-opus-4-8", "stop_reason": null, "stop_sequence": null}}

  event: content_block_start
  data: {"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": "", "signature": ""}}

  event: content_block_delta
  data: {"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "I need to find the GCD of 1071 and 462 using the Euclidean algorithm.\n\n1071 = 2 × 462 + 147"}}

  event: content_block_delta
  data: {"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "\n462 = 3 × 147 + 21\n147 = 7 × 21 + 0\n\nSo GCD(1071, 462) = 21"}}

  // Additional thinking deltas...

  event: content_block_delta
  data: {"type": "content_block_delta", "index": 0, "delta": {"type": "signature_delta", "signature": "EqQBCgIYAhIM1gbcDa9GJwZA2b..."}}

  event: content_block_stop
  data: {"type": "content_block_stop", "index": 0}

  event: content_block_start
  data: {"type": "content_block_start", "index": 1, "content_block": {"type": "text", "text": ""}}

  event: content_block_delta
  data: {"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "The greatest common divisor of 1071 and 462 is **21**."}}

  // Additional text deltas...

  event: content_block_stop
  data: {"type": "content_block_stop", "index": 1}

  event: message_delta
  data: {"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null}}

  event: message_stop
  data: {"type": "message_stop"}
  ```
</Accordion>

当设置 `display: "omitted"` 时，思考块打开，单个 `signature_delta` 到达，然后该块关闭，没有任何 `thinking_delta` 事件。文本流式传输随即开始：

```sse Output
event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EosnCkYICxIMMb3LzNrMu..."}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: content_block_start
data: {"type":"content_block_start","index":1,"content_block":{"type":"text","text":""}}
```

<Note>
  在启用思考的情况下使用流式传输时，您可能会注意到文本有时以较大的块到达，与较小的逐令牌传递交替出现。这是预期行为，尤其是对于思考内容。

  流式传输系统批量处理内容，这可能会延迟并将流式传输事件分组为这种"分块"传递模式。
</Note>

有关通用流式传输机制，请参见[流式传输消息](https://platform.claude.com/docs/zh-CN/build-with-claude/streaming)。

## 思考与 effort

`thinking` 参数控制 Claude 是否在回答前在[思考块](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking)中进行思考；`effort` 参数控制 Claude 在整个响应中投入的工作量，在自适应模式下，这包括思考的频率和深度。不要将 `adaptive` 作为 `effort` 的值传递：`adaptive` 是一种思考模式，而不是一个努力程度级别。

有关每个 effort 级别对思考行为的影响，请参见[调节思考](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking-steering-and-cost)页面上的[各级别思考行为表](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking-steering-and-cost#effort-levels)。[Effort](https://platform.claude.com/docs/zh-CN/build-with-claude/effort) 页面记录了该参数本身，包括每个模型支持的级别。在 Claude Opus 4.5（唯一支持 effort 的仅扩展思考模型）上，effort 与 `budget_tokens` 组合使用。参见[预算规则与调优](https://platform.claude.com/docs/zh-CN/build-with-claude/extended-thinking#budget-rules-and-tuning)。

由于这两个控制项是分开的，请选择与您的目标相匹配的那个：

* **在启用思考的工作负载上降低成本或延迟：**&#x9996;先降低 `effort`。它会按比例缩减整个响应，包括思考部分。
* **Claude 思考得太少或太浅：**&#x63D0;高 `effort`，或参见调节页面上的[调节 Claude 思考的频率](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking-steering-and-cost#tuning-thinking-behavior)。
* **您需要完全关闭思考：**&#x5728;允许的模型上使用 `thinking: {type: "disabled"}`（参见[各模型配置表](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking-troubleshooting#supported-models)）。
* **您需要对支出设置硬性上限：**&#x4F7F;用 `max_tokens`。Effort 是软性指导，`max_tokens` 是严格限制。

## 思考与工具使用

思考功能可与[工具使用](https://platform.claude.com/docs/zh-CN/agents-and-tools/tool-use/overview)配合使用，让 Claude 能够推理工具选择并处理工具结果。有两个约束条件：

1. **工具选择限制（手动模式）：**&#x4F7F;用手动扩展思考（`thinking: {type: "enabled"}`）的工具使用仅支持 `tool_choice: {"type": "auto"}`（默认值）或 `tool_choice: {"type": "none"}`。使用 `tool_choice: {"type": "any"}` 或 `tool_choice: {"type": "tool", "name": "..."}` 会导致错误，因为这些选项强制使用工具，这与手动扩展思考不兼容。自适应思考（包括在默认开启思考的模型上）支持强制工具使用。
2. **保留思考块：**&#x5F53;您返回工具结果时，必须将助手消息中的思考块完整且未经修改地传回 API。参见[保留思考块](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#preserving-thinking-blocks)。

**一个工具使用循环是一个助手轮次。**&#x4ECE;模型的角度来看，助手轮次在 Claude 完成其完整响应之前不会结束，这可能包括多个工具调用和结果。整个序列是单个助手轮次：

```text wrap
User: "What's the weather in Paris?"
Assistant: [thinking] + [tool_use: get_weather]
User: [tool_result: "20°C, sunny"]
Assistant: [text: "The weather in Paris is 20°C and sunny"]
```

整个轮次在单一思考模式下运行：您不能在轮次中途切换思考，包括在工具使用循环期间。在扩展（手动）模式下，API 还强制要求启用思考的请求的最后一个助手轮次以思考块开始。自适应模式放宽了这一要求：没有任何助手轮次需要以思考块开始。

**轮次中途的冲突会优雅降级。**&#x5982;果您在轮次中途切换思考（例如，在发送工具调用和返回其结果之间），API 不会报错。相反，它会静默地为该请求禁用思考。为了保持模型质量，API 可能会剥离会创建无效轮次结构的思考块，或在对话历史与启用思考不兼容时禁用思考。要确认思考是否处于活动状态，请检查响应中是否存在 `thinking` 块。

**在轮次之间切换，而不是在轮次内切换。**&#x5728;每个轮次开始时规划您的思考策略。完成助手轮次，然后为下一个轮次更改思考配置：

```text wrap
User: "What's the weather?"
Assistant: [tool_use] (thinking disabled)
User: [tool_result]
Assistant: [text: "It's sunny"]
User: "What about tomorrow?"
Assistant: [thinking] + [text: "..."] (thinking enabled - new turn)
```

切换思考模式也会使提示缓存失效。参见[思考与提示缓存](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#thinking-and-prompt-caching)。

### 保留思考块

当 Claude 调用工具时，它会暂停构建其响应以等待外部信息。当您返回工具结果时，Claude 会继续构建同一个响应，因此其早期的推理必须仍然存在。请将每个 `thinking` 块完整且未经修改地传回 API，连同它所伴随的 `tool_use` 块一起。这很重要，原因有二：

1. **推理连续性：**&#x601D;考块捕获了导致工具请求的逐步推理。包含它们可以让 Claude 从中断的地方继续推理。
2. **上下文维护：**&#x5DE5;具结果在 API 结构中显示为用户消息，但它们是一个连续推理流的一部分。保留思考块可以在 API 调用之间维护该流程。

简而言之：

* **必需：**&#x5728;工具使用轮次内，传回思考块。
* **推荐：**&#x8DE8;轮次时，传回所有内容。
* **允许：**&#x5728;工具使用之外，省略先前轮次的思考。

您不需要自己修剪旧的思考内容。在多轮对话中传回所有思考块，API 会自动过滤它们，保留维护模型推理所需的块，并且仅对实际展示给 Claude 的块计费输入令牌。保留哪些先前轮次的块因模型而异。参见[各模型的思考块保留](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#thinking-block-preservation-by-model)。要覆盖默认行为，请使用 [`clear_thinking_20251015` 上下文编辑策略](https://platform.claude.com/docs/zh-CN/build-with-claude/context-editing#thinking-block-clearing)。

在最新的助手消息中，连续 `thinking` 块的序列必须与模型在原始请求中生成的内容匹配：您不能重新排列、编辑或部分删除它们。这包括 [`redacted_thinking` 块](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#redacted-thinking-blocks)。

<Note>
  修改过的思考块会被拒绝并返回 400 错误。有关确切的错误消息、常见原因和修复方法，请参见 [400 错误提示思考块不能被修改](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking-troubleshooting#error-thinking-blocks-modified)。唯一的例外：放置在[省略](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#controlling-thinking-display)块的空 `thinking` 字段中的文本会被忽略而不是被拒绝。
</Note>

有关包含每个 SDK 代码的完整两轮演练，请参见[工具和多轮工作流中的思考](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking-tool-workflows#two-turn-tool-use-round-trip)。它定义了一个工具，接收思考加工具使用的响应，并将助手轮次连同工具结果一起回传。

### 交错思考

交错思考让 Claude 能够在工具调用之间进行思考，在对每个工具结果采取行动之前对其进行推理。通过交错思考，Claude 可以：

* 在决定下一步做什么之前对工具调用的结果进行推理
* 在多个工具调用之间穿插推理步骤
* 基于中间结果做出更细致的决策

<Note>
  连续的工具调用不需要交错思考。无论是否有交错思考，Claude 都可以链式调用工具。交错改变的是思考块在工具调用之间出现的位置，而不是工具调用是否可以链式进行。
</Note>

使用自适应思考时，交错思考在每个支持自适应思考的模型上都是自动的。不需要 beta 标头。在 Claude Fable 5、Claude Mythos 5、Claude Mythos Preview、Claude Opus 5、Claude Opus 4.8 和 Claude Opus 4.7 上，工具调用之间的推理始终出现在思考块中。Claude Haiku 4.5 不支持交错思考。在使用手动扩展思考的模型上，交错需要 beta 标头，并且会改变思考预算的计算方式。[手动模式下的交错思考](https://platform.claude.com/docs/zh-CN/build-with-claude/extended-thinking#interleaved-thinking)介绍了各模型的规则和特定平台的标头行为。

使用交错思考时，思考分配可以跨越整个助手轮次，而不是单个响应。交错思考仅支持[通过 Messages API 使用的工具](https://platform.claude.com/docs/zh-CN/agents-and-tools/tool-use/overview)。

有关展示交错思考在双工具工作流中改变了什么的实际对比，请参见[交错思考如何改变流程](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking-tool-workflows#how-interleaved-thinking-changes-the-flow)。

### 各模型的思考块保留

先前助手轮次的思考块是否默认保留在上下文中取决于模型：

* **保留所有先前轮次：**&#x43;laude Opus 4.5 及更高版本的 Opus 模型、Claude Sonnet 4.6 及更高版本的 Sonnet 模型、Claude Fable 5、Claude Mythos 5 和 Claude Mythos Preview。
* **仅保留最后一个轮次：**&#x66F4;早的 Opus 和 Sonnet 模型，以及截至 Claude Haiku 4.5 的所有 Haiku 模型。当您传回较旧的思考块时，API 会自动剥离它们。您不需要自己删除它们。

保留带来两个好处：

* **缓存优化：**&#x4FDD;留的思考块在工具使用期间可以实现缓存命中，因为它们与工具结果一起传回并在助手轮次中增量缓存，从而在多步骤工作流中节省令牌。
* **无智能影响：**&#x4FDD;留思考块对模型性能没有负面影响。

权衡之处在于上下文使用：在保留所有轮次的模型上，长对话会消耗更多的上下文空间，因为保留的思考块像任何其他对话历史一样计为输入（参见[思考与上下文窗口](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#thinking-and-the-context-window)）。在两种机制下，该行为都是自动的。不需要代码更改或 beta 标头，您应该继续按照[保留思考块](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#preserving-thinking-blocks)中的描述传回完整、未经修改的思考块。要在任一方向上覆盖默认行为，请使用[思考块清除](https://platform.claude.com/docs/zh-CN/build-with-claude/context-editing#thinking-block-clearing)。

**在对话中途切换模型。**&#x5F53;您在任意两个模型之间切换时（例如在[分类器拒绝回退](https://platform.claude.com/docs/zh-CN/build-with-claude/refusals-and-fallback)之后），请从先前的助手轮次中剥离 `thinking` 和 `redacted_thinking` 块。思考块与生成它们的模型绑定。其他模型会静默忽略它们而不是拒绝请求，但被忽略的块仍会增加输入令牌。

## 思考与提示缓存

[提示缓存](https://platform.claude.com/docs/zh-CN/build-with-claude/prompt-caching)与思考功能在几个特定方面存在交互。以下规则在两种思考模式下都适用。

**配置更改会使缓存失效。**&#x601D;考配置和解析后的 [`effort`](https://platform.claude.com/docs/zh-CN/build-with-claude/effort) 级别会被渲染到提示本身中，因此更改其中任何一项都会启动新的缓存前缀。在 `adaptive`、`enabled` 和 `disabled` 之间切换、更改 `budget_tokens` 以及更改 effort 值都会使缓存断点失效：消息级断点总是未命中，工具和系统提示断点也可能未命中，具体取决于模型在何处渲染配置。请将任何思考或 effort 更改视为重新开始缓存。保持相同配置的连续请求会保留缓存，并且将参数显式设置为其默认值等同于省略它。[调节思考](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking-steering-and-cost#prompt-caching)页面上有带使用量输出的实际演示。

**思考块与工具结果一起缓存。**&#x5728;工具使用循环期间，当您发出包含工具结果的后续请求时会发生缓存。此时，先前的对话历史（包括其思考块）可以被缓存，并且当从缓存中读取时，这些缓存的思考块在您的使用量指标中计为输入令牌。这会自动发生，即使没有显式的 `cache_control` 标记，并且对于常规思考和交错思考的行为相同。权衡之处：您在响应中再也看不到的思考块在从缓存读取时仍会计入输入令牌使用量。

**先前的块是否在上下文中取决于模型。**[保留默认值](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#thinking-block-preservation-by-model)决定了这一点。在保留所有轮次的模型上，先前轮次的思考块保持缓存并在上下文中。在仅保留最后一个轮次的模型上，一旦您发送不是工具结果的用户消息，所有先前的思考块都会从上下文中剥离。在这些模型上，像这样的对话：

```text wrap
User: ["What's the weather in Paris?"],
Assistant: [thinking_block_1] + [tool_use block 1],
User: [tool_result_1, cache=True],
Assistant: [thinking_block_2] + [text block 2],
User: [Text response, cache=True]
```

会被处理为好像思考块从未存在过：

```text wrap
User: ["What's the weather in Paris?"],
Assistant: [tool_use block 1],
User: [tool_result_1, cache=True],
Assistant: [text block 2],
User: [Text response, cache=True]
```

在保留所有轮次的模型上，相同的请求会将 `thinking_block_1` 和 `thinking_block_2` 保留在上下文和缓存中。

**降级会从可缓存的历史中剥离思考。**&#x5982;果思考在轮次中途被禁用，并且您在当前工具使用轮次中传递了思考内容，则思考内容会被剥离，并且该请求的思考保持禁用状态（参见[优雅降级](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#thinking-with-tool-use)）。[交错思考](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#interleaved-thinking)会放大缓存失效效应，因为思考块可能出现在多个工具调用之间。

<Tip>
  思考密集型任务通常需要超过默认 5 分钟缓存生命周期的时间才能完成。考虑使用 [1 小时缓存持续时间](https://platform.claude.com/docs/zh-CN/build-with-claude/prompt-caching#1-hour-cache-duration)，以在较长的思考会话和多步骤工作流中保持缓存命中。
</Tip>

## 思考与上下文窗口

`max_tokens`（包括 Claude 在当前轮次中生成的所有思考内容）作为严格限制强制执行。在 Claude 4.5 及更新的模型上，如果输入令牌加上 `max_tokens` 超过上下文窗口大小，API 会接受该请求。如果生成随后达到上下文窗口限制，它会以 `stop_reason: "model_context_window_exceeded"` 停止，而不是返回错误。在更早的模型上，API 会返回验证错误。参见[处理停止原因](https://platform.claude.com/docs/zh-CN/build-with-claude/handling-stop-reasons)。

思考如何计入窗口取决于它何时生成：

* **当前轮次的思考**始终计入 `max_tokens`，按输出令牌计费，并在生成它的轮次中占用上下文窗口空间。
* **先前轮次的思考**取决于[保留默认值](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#thinking-block-preservation-by-model)。在[保留所有先前轮次的模型](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#thinking-block-preservation-by-model)上，先前的思考块保留在上下文中，计入窗口，并像对话历史的其余部分一样按输入令牌计费。在仅保留最后一个轮次的模型上，当您传回较旧的思考块时，API 会自动剥离它们，因此它们不会消耗窗口空间或输入令牌。

实际操作中：

* 在保留所有轮次的模型上，请将思考视为普通对话历史来规划您的上下文窗口预算，因为它确实如此。长时间的智能体会话会在上下文中累积思考内容。如果您需要回收空间，请使用[思考块清除](https://platform.claude.com/docs/zh-CN/build-with-claude/context-editing#thinking-block-clearing)。
* 在仅保留最后一个轮次的模型上，思考只是每轮次的成本：每个轮次的思考计入该轮次的 `max_tokens`，然后从窗口中移除。

以下示意图说明了仅保留最后一个轮次（剥离）机制。第一张图显示了多轮对话：每个轮次的思考块在输出中生成，但不会带入后续轮次的输入。

![在剥离先前思考块的模型上的思考示意图：每个轮次的 thinking 块在输出中生成，不会带入后续轮次的输入](https://platform.claude.com/docs/images/context-window-thinking.svg)

第二张图显示了相同机制下的工具使用：思考在助手轮次期间与其工具结果一起保留在上下文中，然后在下一个用户轮次时被移除。

![在剥离先前思考块的模型上带工具使用的思考示意图：thinking 与其工具结果一起保留，然后在下一个用户轮次时被移除](https://platform.claude.com/docs/images/context-window-thinking-tools.svg)

使用[令牌计数 API](https://platform.claude.com/docs/zh-CN/build-with-claude/token-counting) 为您的特定用例获取准确的计数，尤其是对于包含思考的多轮对话。

## 思考加密

完整的思考内容经过加密并在每个思考块的 `signature` 字段中返回。当您传回思考块时，API 使用签名来验证思考块是由 Claude 生成的。

使用签名时，请注意以下几点：

* 只有在[将工具与思考结合使用](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#thinking-with-tool-use)时才严格需要传回思考块。否则，您可以省略先前轮次的思考块。如果您确实传回它们，API 是保留还是剥离它们取决于模型（参见[各模型的思考块保留](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#thinking-block-preservation-by-model)）。使用[上下文编辑](https://platform.claude.com/docs/zh-CN/build-with-claude/context-editing)来配置此行为。
* 传回思考块时，请完全按照您收到的内容传回所有内容，以保持一致性并避免潜在问题。
* 在[流式传输响应](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#streaming-thinking)时，签名作为 `content_block_delta` 事件内的 `signature_delta` 到达，就在 `content_block_stop` 事件之前。
* 在 Claude 4 及更高版本的模型中，`signature` 值比之前的模型长得多。
* `signature` 字段是不透明的：不要解释或解析它。
* `signature` 值跨平台兼容（Claude API、[Amazon Bedrock](https://platform.claude.com/docs/zh-CN/build-with-claude/claude-in-amazon-bedrock) 和 [Google Cloud](https://platform.claude.com/docs/zh-CN/build-with-claude/claude-on-vertex-ai)）。在一个平台上生成的值可以在另一个平台上使用。

## 已编辑的思考块

除了常规的 `thinking` 块之外，当 Claude 推理的某些部分因安全原因被编辑时，API 可能会返回 `redacted_thinking` 块。`redacted_thinking` 块在 `data` 字段中包含加密的思考内容，没有可读文本：

```json
{
  "type": "redacted_thinking",
  "data": "..."
}
```

`data` 字段是不透明且加密的。与常规思考块上的 `signature` 字段一样，在使用[工具](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#thinking-with-tool-use)继续多轮对话时，请将 `redacted_thinking` 块原样传回 API。

<Tip>
  如果您的代码在往返传递带工具使用的响应时按类型过滤内容块（例如 `block.type == "thinking"`），请同时包含 `redacted_thinking` 块。仅按 `block.type == "thinking"` 过滤会静默丢弃 `redacted_thinking` 块，并破坏[保留思考块](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#preserving-thinking-blocks)中描述的多轮协议。
</Tip>

<Note>
  `redacted_thinking` 块是当思考因安全原因被编辑时返回的一种独立的内容块类型。这与 [`display: "omitted"`](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#controlling-thinking-display) 选项不同，后者返回 `thinking` 字段为空的常规 `thinking` 块。
</Note>

## Claude Fable 5 和 Claude Mythos 5 上的思考输出

在 Claude Fable 5 和 Claude Mythos 5 上，原始思维链永远不会返回。您收到的块是常规的 `thinking` 块，而不是 `redacted_thinking`，并且 [`display` 设置](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#controlling-thinking-display)的工作方式与其他模型相同（[摘要化](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#summarized-thinking)文本，或在省略时为空的 `thinking` 字段，这是此处的默认值）。有关思考块的响应形态，请参见 [Messages API 参考](https://platform.claude.com/docs/zh-CN/api/messages/create)。

在同一模型上继续对话时，请将每个思考块完全按照收到的内容传回 API，包括 `thinking` 字段为空的块。不要编辑或重建它们。读取摘要文本用于显示是可以的：API 拒绝的是返回内容已被修改的块，而不是您已读取的块。放置在空的省略 `thinking` 字段中的文本会[被忽略而不是被拒绝](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#controlling-thinking-display)。

有关在对话中途切换模型时如何处理思考块，请参见[各模型的思考块保留](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#thinking-block-preservation-by-model)。

有两个例外，在[回退额度](https://platform.claude.com/docs/zh-CN/build-with-claude/fallback-credit)中有介绍：

* 回退额度重试必须原样回传被拒绝的请求体。
* 来自输出中途回退的 `fallback` 块保留在它们出现的位置。

要了解模型的推理过程，请读取本页描述的 `thinking` 块，而不是在响应文本中提示推理。在 Claude Fable 5 上，试图将模型的内部推理作为响应文本的一部分引出的请求可能会被拒绝，并返回 `stop_details.category: "reasoning_extraction"`。有关字段参考和处理指南，请参见[拒绝类别](https://platform.claude.com/docs/zh-CN/build-with-claude/refusals-and-fallback#refusal-response)。

## 限制与功能兼容性

**采样参数。** 在 Claude Fable 5、Claude Mythos 5、Claude Mythos Preview、Claude Opus 5、Claude Opus 4.8、Claude Opus 4.7 和 Claude Sonnet 5 上，无论是否使用思考功能，非默认的 `temperature`、`top_p` 或 `top_k` 值都会在每个请求中返回 400 错误。在较旧的模型上，该限制仅在思考功能开启时适用：`temperature` 和 `top_k` 与思考功能不兼容，而 `top_p` 允许使用 0.95 到 1 之间的值。

**响应预填充和强制工具使用。** 当思考功能开启时，您无法预填充助手响应。强制工具使用（`tool_choice: {"type": "any"}` 或 `{"type": "tool", ...}`）与手动扩展思考不兼容，但可与自适应思考配合使用。请参阅[思考与工具使用](https://platform.claude.com/docs/zh-CN/build-with-claude/thinking#thinking-with-tool-use)。

**输出限制。** Claude Fable 5、Claude Mythos 5、Claude Mythos Preview、Claude Opus 5、Claude Opus 4.8、Claude Opus 4.7、Claude Sonnet 5、Claude Opus 4.6 和 Claude Sonnet 4.6 支持每个请求最多 128k 输出令牌。Claude Haiku 4.5、Claude Sonnet 4.5 和 Claude Opus 4.5 支持最多 64k。在[消息批处理 API](https://platform.claude.com/docs/zh-CN/build-with-claude/batch-processing#extended-output-beta) 上，`output-300k-2026-03-24` [beta 标头](https://platform.claude.com/docs/zh-CN/api/beta-headers)可将 Claude Opus 5、Claude Opus 4.8、Claude Opus 4.7、Claude Sonnet 5、Claude Opus 4.6 和 Claude Sonnet 4.6 的限制提高到 300k。有关旧版模型的限制，请参阅[模型概述](https://platform.claude.com/docs/zh-CN/about-claude/models/overview)。

**长请求。** 当 `max_tokens` 大于 21,333 时，SDK 要求使用流式传输，以避免长时间运行的请求出现 HTTP 超时。这是客户端验证，而非 API 限制。如果您不需要增量处理事件，请使用 `.stream()` 配合 `.get_final_message()`（Python）或 `.finalMessage()`（TypeScript）来获取完整的 `Message` 对象，而无需处理单个事件。请参阅[流式传输消息](https://platform.claude.com/docs/zh-CN/build-with-claude/streaming#get-the-final-message-without-handling-events)。当思考功能处于活动状态时，预计响应时间会更长，因为生成思考块会增加处理时间。对于每个请求的思考令牌数大约超过 32k 的工作负载，请使用[批处理](https://platform.claude.com/docs/zh-CN/build-with-claude/batch-processing)以避免网络问题：此类请求的运行时间可能足够长，以至于触发系统超时和开放连接数限制。

## 后续步骤

<CardGroup cols={2}>
  <Card title="引导思考" icon="compass" href="https://platform.claude.com/docs/zh-CN/build-with-claude/thinking-steering-and-cost">
    通过努力级别、系统提示指导和逐条消息引导来控制 Claude 思考的频率和深度，并了解思考的成本和定价。
  </Card>

  <Card title="工具和多轮工作流中的思考" icon="wrench" href="https://platform.claude.com/docs/zh-CN/build-with-claude/thinking-tool-workflows">
    逐步完成一个完整的两轮工具使用往返流程，正确保留思考块，并了解交错思考如何改变流程。
  </Card>

  <Card title="思考功能故障排除" icon="hammer" href="https://platform.claude.com/docs/zh-CN/build-with-claude/thinking-troubleshooting">
    诊断并修复最常见的思考功能故障：配置 400 错误、空白或缺失的思考块、max\_tokens 停止以及缓存未命中。
  </Card>

  <Card title="努力参数" icon="sliders" href="https://platform.claude.com/docs/zh-CN/build-with-claude/effort">
    使用 effort 参数控制 Claude 在响应时使用的令牌数量，在响应完整性和令牌效率之间进行权衡。
  </Card>
</CardGroup>
