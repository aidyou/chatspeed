> ## Documentation Index
> Fetch the complete documentation index at: https://platform.stepfun.ai/docs/llms.txt
> Use this file to discover all available pages before exploring further.

# Reasoning model best practices

StepFun offers a range of reasoning models covering text reasoning, multimodal understanding, agent, and code analysis scenarios. We recommend using `step-3.7-flash` as the preferred option; for pure text reasoning, you can also use `step-3.5-flash`.

`step-3.7-flash` is StepFun's flagship multimodal reasoning model. Building on the high-speed reasoning and tool-calling capabilities of `step-3.5-flash`, it adds native image and video understanding and supports three levels of reasoning effort, making it well-suited for agent, code, multimodal analysis, and complex planning tasks.

`step-3.5-flash` is a high-speed reasoning model built for extreme efficiency. Based on a sparse Mixture-of-Experts (MoE) architecture, it carries 196B parameters but selectively activates only \~11B per token, pairing the logical depth of much larger models with low-latency inference. With a 256K context window plus solid tool-calling and multi-step agent capabilities, it is well-suited to pure-text reasoning, engineering, and automation workloads.

## Reasoning effort control

Models that support three levels of reasoning effort let you tune thinking depth via a parameter. The Chat Completion API uses `reasoning_effort`; the Messages API uses `output_config.effort`.

| Reasoning effort | Use cases                                                                   |
| :--------------- | :-------------------------------------------------------------------------- |
| `low`            | Simple Q\&A, summarization, rewriting, information extraction               |
| `medium`         | Default recommendation, suitable for general reasoning and multi-step tasks |
| `high`           | Complex reasoning, math, planning, code analysis                            |

<Info>
  For complete call examples, see [Step 3.7 Flash quickstart](/docs/en/guides/models/step-3.7-flash-quickstart#control-reasoning-effort). For parameter field details, see [Chat Completion API](/docs/en/api-reference/chat/chat-completion-create#request-parameters) and [Messages API](/docs/en/api-reference/chat/messages-create#request-parameters).
</Info>

## Chat Completion Example

Calling reasoning models for text chat works the same way across models. The following uses `step-3.5-flash`, a common choice for pure-text scenarios, to build a streaming conversation.

```python copy theme={null}
import time
from openai import OpenAI

# Set your API Key and Base URL
BASE_URL = "https://api.stepfun.ai/v1"
STEP_API_KEY = "YOUR_STEPFUN_API_KEY"

# Select Model
COMPLETION_MODEL = "step-3.5-flash"

# User Prompt
user_prompt = "How many 'r's are in the word strawberry?"

client = OpenAI(api_key=STEP_API_KEY, base_url=BASE_URL)

time_start = time.time()

try:
    response = client.chat.completions.create(
        model=COMPLETION_MODEL,
        messages=[
            {"role": "user", "content": user_prompt}
        ],
        stream=True
    )
except Exception as e:
    print("Exception occurred when requesting API:", e)
    exit(1)

print("Reasoning Process:")
try:
    for chunk in response:
        # Check for reasoning content
        if hasattr(chunk.choices[0].delta, 'reasoning') and chunk.choices[0].delta.reasoning:
             print(chunk.choices[0].delta.reasoning, end='', flush=True)
        # Check for standard content
        elif chunk.choices[0].delta.content:
             print(chunk.choices[0].delta.content, end='', flush=True)

except Exception as e:
    print("\\nError occurred while processing streaming results:", e)

time_end = time.time()
print(f"\\n\\nTotal generation time: {time_end - time_start:.2f} seconds")
```

For input parameter details, please refer to the [Chat Completion Documentation](/docs/en/api-reference/chat/chat-completion-create#request-parameters)

## Obtaining Reasoning Content

When StepFun's reasoning models handle complex problems, they include a `reasoning` field in the output to display the model's thinking process. Developers can check for the existence of this field to obtain the model's thinking information.

```python theme={null}
if chunk.choices[0].delta.reasoning:
    reasoning = chunk.choices[0].delta.reasoning
    print("Model thinking process:", reasoning)
```

For non-streaming scenarios, you can directly extract the `reasoning` field to get the model's thinking process.

```python theme={null}
msg  = completion.choices[0].message.content
reasoning = completion.choices[0].message.reasoning
```

## Notes

* **Error Handling and Logging**: A Trace ID is added to model outputs. Please include this ID when reporting any issues with reasoning behavior.
