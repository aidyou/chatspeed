## 思考模式

DeepSeek 模型支持思考模式：在输出最终回答之前，模型会先输出一段思维链内容，以提升最终答案的准确性。

## 思考模式开关与思考强度控制[​](#思考模式开关与思考强度控制 "思考模式开关与思考强度控制的直接链接")

控制参数（OpenAI 格式）

控制参数（Anthropic 格式）

控制参数（Responses API 格式）

思考模式开关(1)

`{"thinking": {"type": "enabled/disabled"}}`

`{"reasoning": {"effort": "none/low/high/max"}}`  
(none 表示关闭思考模式)

思考强度控制(2)

`{"reasoning_effort": "low/high/max"}`

`{"output_config": {"effort": "low/high/max"}}`

(1) 思考模式默认打开，且 effort 默认为 `high`  
(2) 用户设置的 effort 与模型推理 effort 映射表如下（`deepseek-v4-flash` 与 `deepseek-v4-pro` 一致）：

请求传入 effort

实际映射 effort

low

low

medium

high

high

high

xhigh

high

max

max

您在 OpenAI SDK 中使用 Chat Completion 设置 `thinking` 参数时，需要将 `thinking` 参数传入 `extra_body` 中：

```
response = client.chat.completions.create(  model="deepseek-v4-pro",  # ...  reasoning_effort="high",  extra_body={"thinking": {"type": "enabled"}})
```

## 输入输出参数[​](#输入输出参数 "输入输出参数的直接链接")

思考模式不支持 `temperature`、`top_p`、`presence_penalty`、`frequency_penalty` 参数。请注意，为了兼容已有软件，设置参数不会报错，但也不会生效。

在思考模式下，思维链内容通过 `reasoning_content` 参数返回，与 `content` 同级。在后续轮次的请求中，`reasoning_content` 是否需要回传、是否会被拼接进上下文，取决于请求是否携带 `tools` 参数：

*   若请求**携带 `tools` 参数**：历史轮次的 `reasoning_content` 均应回传给 API，并会被拼接进上下文。详见[工具调用](#tool-calls)。
*   若请求**未携带 `tools` 参数**：`reasoning_content` 无需回传；即使传入 API，也会被忽略，不会拼接进上下文。详见[多轮对话拼接](#%E5%A4%9A%E8%BD%AE%E5%AF%B9%E8%AF%9D%E6%8B%BC%E6%8E%A5)。

## 多轮对话拼接[​](#多轮对话拼接 "多轮对话拼接的直接链接")

在每一轮对话过程中，模型会输出思维链内容（`reasoning_content`）和最终回答（`content`）。如果请求未携带 `tools` 参数，则在下一轮对话中，之前轮输出的思维链内容不会被拼接到上下文中，如下图所示：

### 样例代码[​](#样例代码 "样例代码的直接链接")

下面的代码以 Python 语言为例，展示了如何访问思维链和最终回答，以及如何在多轮对话中进行上下文拼接。

*   非流式
*   流式

```
from openai import OpenAIclient = OpenAI(api_key="<DeepSeek API Key>", base_url="https://api.deepseek.com")# Turn 1messages = [{"role": "user", "content": "9.11 and 9.8, which is greater?"}]response = client.chat.completions.create(    model="deepseek-v4-pro",    messages=messages,    reasoning_effort="high"    extra_body={"thinking": {"type": "enabled"}},)reasoning_content = response.choices[0].message.reasoning_contentcontent = response.choices[0].message.content# Turn 2# The reasoning_content will be ignored by the APImessages.append(response.choices[0].message)messages.append({'role': 'user', 'content': "How many Rs are there in the word 'strawberry'?"})response = client.chat.completions.create(    model="deepseek-v4-pro",    messages=messages,    reasoning_effort="high"    extra_body={"thinking": {"type": "enabled"}},)# ...
```

## 工具调用[​](#tool-calls "工具调用的直接链接")

DeepSeek 模型的思考模式支持工具调用功能。模型在输出最终答案之前，可以进行多轮的思考与工具调用，以提升答案的质量。其调用模式如下图所示：

请注意，携带了 `tools` 参数的请求，在后续所有请求中，必须完整回传 `reasoning_content` 给 API——即使该轮模型未实际进行工具调用。若您的代码中未正确回传 `reasoning_content`，API 会返回 400 报错。正确回传方法请您参考下面的样例代码。

### 样例代码[​](#样例代码-1 "样例代码的直接链接")

下面是一个简单的在思考模式下进行工具调用的样例代码：

```
import osimport jsonfrom openai import OpenAIfrom datetime import datetime# The definition of the toolstools = [    {        "type": "function",        "function": {            "name": "get_date",            "description": "Get the current date",            "parameters": { "type": "object", "properties": {} },        }    },    {        "type": "function",        "function": {            "name": "get_weather",            "description": "Get weather of a location, the user should supply the location and date.",            "parameters": {                "type": "object",                "properties": {                    "location": { "type": "string", "description": "The city name" },                    "date": { "type": "string", "description": "The date in format YYYY-mm-dd" },                },                "required": ["location", "date"]            },        }    },]# The mocked version of the tool callsdef get_date_mock():    return datetime.now().strftime("%Y-%m-%d")def get_weather_mock(location, date):    return "Cloudy 7~13°C"TOOL_CALL_MAP = {    "get_date": get_date_mock,    "get_weather": get_weather_mock}def run_turn(turn, messages):    sub_turn = 1    while True:        response = client.chat.completions.create(            model='deepseek-v4-pro',            messages=messages,            tools=tools,            reasoning_effort="high",            extra_body={ "thinking": { "type": "enabled" } },        )        messages.append(response.choices[0].message)        reasoning_content = response.choices[0].message.reasoning_content        content = response.choices[0].message.content        tool_calls = response.choices[0].message.tool_calls        print(f"Turn {turn}.{sub_turn}\n{reasoning_content=}\n{content=}\n{tool_calls=}")        # If there is no tool calls, then the model should get a final answer and we need to stop the loop        if tool_calls is None:            break        for tool in tool_calls:            tool_function = TOOL_CALL_MAP[tool.function.name]            tool_result = tool_function(**json.loads(tool.function.arguments))            print(f"tool result for {tool.function.name}: {tool_result}\n")            messages.append({                "role": "tool",                "tool_call_id": tool.id,                "content": tool_result,            })        sub_turn += 1    print()client = OpenAI(    api_key=os.environ.get('DEEPSEEK_API_KEY'),    base_url=os.environ.get('DEEPSEEK_BASE_URL'),)# The user starts a questionturn = 1messages = [{    "role": "user",    "content": "How's the weather in Hangzhou Tomorrow"}]run_turn(turn, messages)# The user starts a new questionturn = 2messages.append({    "role": "user",    "content": "How's the weather in Guangzhou Tomorrow"})run_turn(turn, messages)
```

在 Turn 1 的每个子请求中，都携带了该 Turn 下产生的 `reasoning_content` 给 API，从而让模型继续之前的思考。`response.choices[0].message` 携带了 `assistant` 消息的所有必要字段，包括 `content`、`reasoning_content`、`tool_calls`。简单起见，可以直接用如下代码将消息 append 到 messages 结尾：

```
messages.append(response.choices[0].message)
```

这行代码等价于：

```
messages.append({    'role': 'assistant',    'content': response.choices[0].message.content,    'reasoning_content': response.choices[0].message.reasoning_content,    'tool_calls': response.choices[0].message.tool_calls,})
```

且在 Turn 2 的请求中，我们仍然携带着 Turn1 所产生的 `reasoning_content` 给 API。

该代码的样例输出如下：

```
Turn 1.1reasoning_content="The user is asking about the weather in Hangzhou tomorrow. I need to get tomorrow's date first, then call the weather function."content="Let me check tomorrow's weather in Hangzhou for you. First, let me get tomorrow's date."tool_calls=[ChatCompletionMessageFunctionToolCall(id='call_00_kw66qNnNto11bSfJVIdlV5Oo', function=Function(arguments='{}', name='get_date'), type='function', index=0)]tool result for get_date: 2026-04-19Turn 1.2reasoning_content="Today is 2026-04-19, so tomorrow is 2026-04-20. Now I'll call the weather function for Hangzhou."content=''tool_calls=[ChatCompletionMessageFunctionToolCall(id='call_00_H2SCW6136vWJGq9SQlBuhVt4', function=Function(arguments='{"location": "Hangzhou", "date": "2026-04-20"}', name='get_weather'), type='function', index=0)]tool result for get_weather: Cloudy 7~13°CTurn 1.3reasoning_content='The weather result is in. Let me share this with the user.'content="Here's the weather forecast for **Hangzhou tomorrow (April 20, 2026)**:\n\n- 🌤 **Condition:** Cloudy  \n- 🌡 **Temperature:** 7°C ~ 13°C (45°F ~ 55°F)\n\nIt'll be on the cooler side, so you might want to bring a light jacket if you're heading out! Let me know if you need anything else."tool_calls=NoneTurn 2.1reasoning_content='The user is asking about the weather in Guangzhou tomorrow. Today is 2026-04-19, so tomorrow is 2026-04-20. I can directly call the weather function.'content=''tool_calls=[ChatCompletionMessageFunctionToolCall(id='call_00_8URkLt5NjmNkVKhDmMcNq9Mo', function=Function(arguments='{"location": "Guangzhou", "date": "2026-04-20"}', name='get_weather'), type='function', index=0)]tool result for get_weather: Cloudy 7~13°CTurn 2.2reasoning_content='The weather result for Guangzhou is the same as Hangzhou. Let me share this with the user.'content="Here's the weather forecast for **Guangzhou tomorrow (April 20, 2026)**:\n\n- 🌤 **Condition:** Cloudy  \n- 🌡 **Temperature:** 7°C ~ 13°C (45°F ~ 55°F)\n\nIt'll be cool and cloudy, so a light jacket would be a good idea if you're going out. Let me know if there's anything else you'd like to know!"tool_calls=None
```
