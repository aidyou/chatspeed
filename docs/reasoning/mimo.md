深度思考
深度思考让模型在生成最终回答前进行深度推理，通过内部思维链（Chain of Thought）逐步分析问题，显著提升复杂任务的准确性。适用于复杂推理、代码生成、数学计算、多步骤分析等需要深度分析的场景。

核心能力

深度推理：将复杂问题拆解为多个步骤逐步分析，提升推理准确性

思考过程透明：返回完整思考过程，增强可解释性

灵活控制：支持开启/关闭，可根据任务复杂度按需使用

支持的模型
当前支持 mimo-v2.5-pro、mimo-v2.5 模型。

请求参数
在请求中设置 thinking.type 参数控制深度思考开关：enabled 开启思考或 disabled 关闭思考。

默认状态：

默认开启：mimo-v2.5-pro、mimo-v2.5
注意事项
参数限制
在深度思考下，mimo-v2.5-pro、mimo-v2.5 模型不支持自定义 temperature 和 top_p 参数。即使传入该参数，实际生效值也会被模型强制采用其推荐默认值 1.0 和 0.95。

多轮对话回传要求
在 Agent 类产品的多轮会话中开启深度思考，且历史会话中存在工具调用时，后续所有 user 交互轮次中回传的 assistant 如果包含了工具调用，必须完整回传 reasoning_content 字段，否则 API 将返回 400 错误。正确回传方式请参考调用示例的“深度思考下的多轮工具调用”。

历史 reasoning_content 一旦缺失，模型上下文将不完整，可能表现出指令遵循下降，幻觉增多等现象。

受影响的 Agent 产品：

协议	受影响的 Agent 产品
OpenAI 兼容协议	TRAE、Cursor、Roo Code、Codex、GitHub Copilot CLI、Zed、AutoGen、Goose
Anthropic 兼容协议	TRAE、GitHub Copilot CLI、AutoGen、Goose、OpenClaw、OpenCode、Kilo Code
其他说明
输出长度限制：max_completion_tokens 限制的是思考内容与最终回答的总长度。若思考过程较长，留给最终回答的 token 空间会相应减少。建议设置足够的 max_completion_tokens 以避免回答被截断。

响应时间：开启深度思考会增加响应延迟，复杂任务尤为明显。建议结合 stream: true 实时查看思考过程。

调用示例
thinking 字段并非 OpenAI 标准参数。通过 OpenAI Python SDK 传入思考相关参数时，需将其置于 extra_body 中传递。

开启思考
Python SDK
Curl
import os
from openai import OpenAI

client = OpenAI(
    api_key=os.environ.get("MIMO_API_KEY"),
    base_url="https://api.xiaomimimo.com/v1"
)

completion = client.chat.completions.create(
    model="mimo-v2.5-pro",
    messages=[
        {
            "role": "system",
            "content": "You are MiMo, an AI assistant developed by Xiaomi. Today is date: Tuesday, December 16, 2025. Your knowledge cutoff date is December 2024."
        },
        {
            "role": "user",
            "content": "Introduce machine learning in three sentences."
        }
    ],
    max_completion_tokens=1024,
    extra_body={
        "thinking": {"type": "enabled"}
    }
)

print(completion.model_dump_json())


响应示例

{
    "id": "2b92b0964c9b4335bffad7c2f75cfe9e",
    "choices": [
        {
            "finish_reason": "stop",
            "index": 0,
            "message": {
                "content": "Machine learning is a branch of artificial intelligence that enables systems to automatically learn and improve from experience without being explicitly programmed. It works by identifying patterns in data to make predictions or decisions. This technology powers a wide range of applications, from recommendation systems and speech recognition to autonomous vehicles and medical diagnosis.",
                "role": "assistant",
                "tool_calls": null,
                "reasoning_content": "Hmm, the user wants a concise three-sentence introduction to machine learning. This seems like a straightforward request for a clear, high-level explanation. \n\nI should focus on the core idea without technical jargon, mention its practical use, and end with its significance. The first sentence can define it simply, the second can give an example, and the third can highlight its impact. \n\nKeeping it neutral and informative fits the user's likely need for a quick overview. No need for extra details or fluff since they specifically asked for brevity."
            }
        }
    ],
    "created": 1781233054,
    "model": "mimo-v2.5-pro",
    "object": "chat.completion",
    "usage": {
        "completion_tokens": 171,
        "prompt_tokens": 60,
        "total_tokens": 231,
        "completion_tokens_details": {
            "reasoning_tokens": 110
        },
        "prompt_tokens_details": {}
    }
}

关闭思考
Python SDK
Curl
import os
from openai import OpenAI

client = OpenAI(
    api_key=os.environ.get("MIMO_API_KEY"),
    base_url="https://api.xiaomimimo.com/v1"
)

completion = client.chat.completions.create(
    model="mimo-v2.5-pro",
    messages=[
        {
            "role": "system",
            "content": "You are MiMo, an AI assistant developed by Xiaomi. Today is date: Tuesday, December 16, 2025. Your knowledge cutoff date is December 2024."
        },
        {
            "role": "user",
            "content": "Write a short paragraph about the beauty of nature."
        }
    ],
    max_completion_tokens=1024,
    extra_body={
        "thinking": {"type": "disabled"}
    }
)

print(completion.model_dump_json())


响应示例

{
    "id": "f914c393444e4a35a4f7b1e337e032cb",
    "choices": [
        {
            "finish_reason": "stop",
            "index": 0,
            "message": {
                "content": "From the gentle rustle of leaves in an ancient forest to the fiery spectacle of a sunset painting the sky, nature’s beauty is a symphony for the senses. It is found in the delicate symmetry of a snowflake, the vibrant hues of a wildflower meadow, and the silent majesty of a mountain range draped in morning mist. This ever-changing tapestry offers a profound sense of peace and wonder, reminding us of a world that exists beyond our own making. Whether in a vast, untouched wilderness or a single dewdrop clinging to a spider's web, nature’s artistry is a constant, humbling source of inspiration and renewal.",
                "role": "assistant",
                "tool_calls": null
            }
        }
    ],
    "created": 1781233927,
    "model": "mimo-v2.5-pro",
    "object": "chat.completion",
    "usage": {
        "completion_tokens": 131,
        "prompt_tokens": 64,
        "total_tokens": 195,
        "completion_tokens_details": {
            "reasoning_tokens": 0
        },
        "prompt_tokens_details": {}
    }
}


流式响应（开启思考）
流式响应时，思考内容与回答内容依次输出：首先通过 reasoning_content 逐步返回思考过程，思考完成后，再通过 content 逐步输出最终回答。

Python SDK
Curl
import os
from openai import OpenAI

client = OpenAI(
    api_key=os.environ.get("MIMO_API_KEY"),
    base_url="https://api.xiaomimimo.com/v1"
)

completion = client.chat.completions.create(
    model="mimo-v2.5-pro",
    messages=[
        {
            "role": "system",
            "content": "You are MiMo, an AI assistant developed by Xiaomi. Today is date: Tuesday, December 16, 2025. Your knowledge cutoff date is December 2024."
        },
        {
            "role": "user",
            "content": "Give me some tips for improving work efficiency."
        }
    ],
    max_completion_tokens=1024,
    stream=True,
    extra_body={
        "thinking": {"type": "enabled"}
    }
)

for chunk in completion:
    print(chunk.model_dump_json())


响应示例

data: {"id":"4e57d676fe464c09aa2f27fa652abc40","choices":[{"delta":{"content":"","role":"assistant","tool_calls":null,"reasoning_content":null},"finish_reason":null,"index":0}],"created":1781234029,"model":"mimo-v2.5-pro","object":"chat.completion.chunk"}

data: {"id":"4e57d676fe464c09aa2f27fa652abc40","choices":[{"delta":{"content":null,"role":null,"tool_calls":null,"reasoning_content":"The user is asking"},"finish_reason":null,"index":0}],"created":1781234029,"model":"mimo-v2.5-pro","object":"chat.completion.chunk"}

data: {"id":"4e57d676fe464c09aa2f27fa652abc40","choices":[{"delta":{"content":null,"role":null,"tool_calls":null,"reasoning_content":" for tips on"},"finish_reason":null,"index":0}],"created":1781234029,"model":"mimo-v2.5-pro","object":"chat.completion.chunk"}

data: {"id":"4e57d676fe464c09aa2f27fa652abc40","choices":[{"delta":{"content":null,"role":null,"tool_calls":null,"reasoning_content":" improving work efficiency."},"finish_reason":null,"index":0}],"created":1781234029,"model":"mimo-v2.5-pro","object":"chat.completion.chunk"}

...

data: {"id":"4e57d676fe464c09aa2f27fa652abc40","choices":[{"delta":{"content":null,"role":null,"tool_calls":null,"reasoning_content":", etc"},"finish_reason":null,"index":0}],"created":1781234030,"model":"mimo-v2.5-pro","object":"chat.completion.chunk"}

data: {"id":"4e57d676fe464c09aa2f27fa652abc40","choices":[{"delta":{"content":null,"role":null,"tool_calls":null,"reasoning_content":"."},"finish_reason":null,"index":0}],"created":1781234030,"model":"mimo-v2.5-pro","object":"chat.completion.chunk"}

data: {"id":"4e57d676fe464c09aa2f27fa652abc40","choices":[{"delta":{"content":"# Tips","role":null,"tool_calls":null,"reasoning_content":null},"finish_reason":null,"index":0}],"created":1781234030,"model":"mimo-v2.5-pro","object":"chat.completion.chunk"}

data: {"id":"4e57d676fe464c09aa2f27fa652abc40","choices":[{"delta":{"content":" for Improving Work","role":null,"tool_calls":null,"reasoning_content":null},"finish_reason":null,"index":0}],"created":1781234030,"model":"mimo-v2.5-pro","object":"chat.completion.chunk"}

...

data: {"id":"4e57d676fe464c09aa2f27fa652abc40","choices":[{"delta":{"content":" on any specific","role":null,"tool_calls":null,"reasoning_content":null},"finish_reason":null,"index":0}],"created":1781234037,"model":"mimo-v2.5-pro","object":"chat.completion.chunk"}

data: {"id":"4e57d676fe464c09aa2f27fa652abc40","choices":[{"delta":{"content":" area?","role":null,"tool_calls":null,"reasoning_content":null},"finish_reason":null,"index":0}],"created":1781234037,"model":"mimo-v2.5-pro","object":"chat.completion.chunk"}

data: {"id":"4e57d676fe464c09aa2f27fa652abc40","choices":[{"delta":{"content":null,"role":null,"tool_calls":null,"reasoning_content":null},"finish_reason":"stop","index":0}],"created":1781234037,"model":"mimo-v2.5-pro","object":"chat.completion.chunk","usage":null}

data: {"id":"4e57d676fe464c09aa2f27fa652abc40","choices":[],"created":1781234037,"model":"mimo-v2.5-pro","object":"chat.completion.chunk","usage":{"completion_tokens":339,"prompt_tokens":61,"total_tokens":400,"completion_tokens_details":{"reasoning_tokens":41},"prompt_tokens_details":{}}}

data: [DONE]


思考模式下的多轮工具调用
在深度思考的多轮对话中，若涉及工具调用，回传 reasoning_content 可确保思考连续性，提升模型输出质量。

import os
import json
from openai import OpenAI

# Initialize client
client = OpenAI(
    api_key=os.environ.get("MIMO_API_KEY"),
    base_url="https://api.xiaomimimo.com/v1"
)

# Define tools
tools = [
    {
        "type": "function",
        "function": {
            "name": "get_current_weather",
            "description": "Get the current weather for a given city",
            "parameters": {
                "type": "object",
                "properties": {
                    "location": {"type": "string", "description": "City name, e.g. Beijing"},
                    "unit": {"type": "string", "enum": ["celsius", "fahrenheit"]}
                },
                "required": ["location"]
            }
        }
    },
    {
        "type": "function",
        "function": {
            "name": "get_time",
            "description": "Get the current time in a given timezone",
            "parameters": {
                "type": "object",
                "properties": {
                    "timezone": {"type": "string", "description": "Timezone, e.g. Asia/Shanghai"}
                },
                "required": ["timezone"]
            }
        }
    }
]

# Tool execution functions (replace with real API calls in production)
def get_current_weather(location: str, unit: str = "celsius") -> str:
    weather_data = {"Beijing": "Sunny 25°C", "Shanghai": "Cloudy 22°C", "Shenzhen": "Rainy 28°C"}
    return weather_data.get(location, f"Weather unknown for {location}")

def get_time(timezone: str) -> str:
    from datetime import datetime
    return datetime.now().strftime(f"%Y-%m-%d %H:%M:%S ({timezone})")

TOOL_MAP = {
    "get_current_weather": lambda **kw: get_current_weather(**kw),
    "get_time": lambda **kw: get_time(**kw)
}

def run_turn(messages, turn_num):
    """Execute a single user turn: call model, run tools in a loop until final answer."""
    request_num = 0
    while True:
        request_num += 1
        print(f"\nRequest {turn_num}-{request_num}:")

        response = client.chat.completions.create(
            model="mimo-v2.5-pro",
            messages=messages,
            tools=tools,
            extra_body={"thinking": {"type": "enabled"}}
        )

        assistant_message = response.choices[0].message
        messages.append(assistant_message)

        # Print full model response
        print(f"reasoning_content: {assistant_message.reasoning_content}")
        print(f"content: \"{assistant_message.content}\"")
        print(f"tool_calls: {assistant_message.tool_calls}")

        # If no tool calls, we have the final answer
        if not assistant_message.tool_calls:
            break

        # Execute each tool call and append results
        for tool_call in assistant_message.tool_calls:
            func_name = tool_call.function.name
            func_args = json.loads(tool_call.function.arguments)
            result = TOOL_MAP[func_name](**func_args)

            print(f"-> Tool result [{func_name}]: {result}")
            messages.append({
                "role": "tool",
                "tool_call_id": tool_call.id,
                "content": result
            })

# --- Multi-turn conversation ---
messages = []

# Turn 1
print("=== Turn 1 ===")
messages.append({"role": "user", "content": "How is the weather in Beijing today? What time is it now?"})
run_turn(messages, turn_num=1)

# Turn 2: reasoning_content from Turn 1 is already in messages via assistant_message
print("\n=== Turn 2 ===")
messages.append({"role": "user", "content": "How about Shanghai? And is it hotter or colder than Beijing?"})
run_turn(messages, turn_num=2)


示例输出

第一轮： 用户询问北京天气和当前时间。模型收到用户消息后进行思考，决定同时调用 get_current_weather 和 get_time 两个工具（Request 1-1）。客户端执行工具并将结果以 role: "tool" 消息追加到 messages 中，再次请求模型。模型结合工具结果生成最终回答（Request 1-2）。

=== Turn 1 ===

Request 1-1:
reasoning_content: The user wants to know two things: 1. The current weather in Beijing 2. The current time in Beijing I can call both functions at the same time since they are independent of each other.
content: ""
tool_calls: [ChatCompletionMessageFunctionToolCall(id='call_dd34ce1810be4afbaaa11c9a', function=Function(arguments='{"location": "Beijing"}', name='get_current_weather'), type='function'), ChatCompletionMessageFunctionToolCall(id='call_cf4c667abd094ce090b40f00', function=Function(arguments='{"timezone": "Asia/Shanghai"}', name='get_time'), type='function')]
-> Tool result [get_current_weather]: Sunny 25°C
-> Tool result [get_time]: 2026-05-12 16:37:26 (Asia/Shanghai)

Request 1-2:
reasoning_content: I got the results for both calls. Let me present this information in a friendly way.
content: "Here's the information for Beijing: ☀️ **Weather:** Sunny, 25°C — a lovely day! 🕒 **Current Time:** 2026年5月12日 16:37 (北京时间) Looks like a beautiful afternoon in Beijing! Perfect weather for being outdoors. Is there anything else you'd like to know? 😊"
tool_calls: None


第二轮： 用户追问上海天气并与北京对比。由于第一轮的 assistant 消息（包含 reasoning_content、content、tool_calls）已通过 messages.append() 累积到对话历史中，模型能直接从上下文获取北京 25°C 的信息，只需再调用 get_current_weather 查询上海（Request 2-1），然后基于两个城市的天气数据进行对比回答（Request 2-2）。

=== Turn 2 ===

Request 2-1:
reasoning_content: The user wants to know the weather in Shanghai and compare it with Beijing. I already know Beijing is 25°C, so I just need to get Shanghai's weather.
content: ""
tool_calls: [ChatCompletionMessageFunctionToolCall(id='call_f4fc7fdbfbd14cb497026d21', function=Function(arguments='{"location": "Shanghai"}', name='get_current_weather'), type='function')]
-> Tool result [get_current_weather]: Cloudy 22°C

Request 2-2:
reasoning_content: Shanghai is 22°C and cloudy, while Beijing is 25°C and sunny. So Shanghai is 3°C cooler than Beijing.
content: "Here's the weather for Shanghai: ☁️ **Weather:** Cloudy, 22°C And comparing the two cities:
| City | Temperature | Conditions |
|------|------------|------------|
| Beijing | 25°C | ☀️ Sunny |
| Shanghai | 22°C | ☁️ Cloudy |

**Beijing is 3°C warmer** than Shanghai right now! Beijing also has clearer skies, while Shanghai is a bit cloudier. Both are pleasant temperatures though — great weather in both cities! 😊 Is there anything else you'd like to check?"
tool_calls: None


更新时间 2026 年 07 月 15 日
