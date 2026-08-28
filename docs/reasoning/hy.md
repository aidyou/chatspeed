## 功能说明

深度思考模型支持在生成最终答案前先进行推理，提升复杂任务的准确性和可解释性。

## 适用场景
- 复杂代码生成、代码修复、代码重构。

- 数学推导、逻辑分析、多步骤决策。

- 复杂信息抽取后再综合归纳。

- 需要更强稳定性和更少推理失误的任务。

## 开启/关闭深度思考

通过 `thinking` 参数控制是否开启思考模式。
- 开启深度思考：`"thinking":{"type":"enabled"}`

- 关闭深度思考：`"thinking":{"type":"disabled"}`

### 支持模型

|**模型名称**|**MODEL 参数值**|**默认值及说明**|
|---------|---------|---------|
|Hy4 preview|`hy4-preview`|enabled|
|Hy3|`hy3`|enabled|
|Hy3 preview|`hy3-preview`|disabled|
|DeepSeek-V4-Flash|`deepseek-v4-flash`|enabled|
|DeepSeek-V4-Flash（原厂直连）|`deepseek-v4-flash-202605`|enabled|
|DeepSeek-V4-Pro|`deepseek-v4-pro`|enabled|
|DeepSeek-V4-Pro（原厂直连）|`deepseek-v4-pro-202606`|enabled|
|GLM-5.3|`glm-5.3`|enabled（不支持关闭）|
|GLM-5.3-Flash|`glm-5.3-flash`|enabled（不支持关闭）|
|GLM-5.2|`glm-5.2`|enabled|
|GLM-5.1|`glm-5.1`|enabled|
|GLM-5|`glm-5`|enabled|
|GLM-5-Turbo|`glm-5-turbo`|enabled|
|GLM-5V-Turbo|`glm-5v-turbo`|enabled|
|Kimi-K2.7-Code|`kimi-k2.7-code`|enabled（不支持关闭）|
|Kimi-K2.7-Code-HighSpeed|`kimi-k2.7-code-highspeed`|enabled（不支持关闭）|
|Kimi-K2.6|`kimi-k2.6`|enabled|
|Kimi-K2.5|`kimi-k2.5`|enabled|
|MiniMax-M3|`minimax-m3`|adaptive|
|MiniMax-M2.7|`minimax-m2.7`|enabled（不支持关闭）|
|MiniMax-M2.5|`minimax-m2.5`|enabled（不支持关闭）|
|Qwen3.5-Plus|`qwen3.5-plus`|enabled|
|Qwen3.5-Flash|`qwen3.5-flash`|enabled|

### 调用示例：开启深度思考

> **说明：**
> 
> 请将 `YOUR_API_KEY` 替换为您创建的 [API Key](https://console.cloud.tencent.com/tokenhub/apikey)。
> 

【cURL】
``` bash
curl -X POST 'https://tokenhub.tencentmaas.com/v1/chat/completions' \
  -H 'Content-Type: application/json' \
  -H 'Authorization: Bearer YOUR_API_KEY' \
  -d '{
    "model": "MODEL 参数值",
    "messages": [
      {"role": "user", "content": "小明有5个苹果，给了小红2个，又买了3个，最后还剩几个？"}
    ],
    "thinking": {"type": "enabled"},
    "stream": false
  }'
```

【Python】
``` python
from openai import OpenAI

client = OpenAI(
    api_key="YOUR_API_KEY",
    base_url="https://tokenhub.tencentmaas.com/v1",
)

response = client.chat.completions.create(
    model="MODEL 参数值",
    messages=[
        {"role": "user", "content": "小明有5个苹果，给了小红2个，又买了3个，最后还剩几个？"},
    ],
    extra_body={"thinking": {"type": "enabled"}},
)

# OpenAI SDK 不直接声明 reasoning_content 字段，需用 getattr 访问
msg = response.choices[0].message
if hasattr(msg, "reasoning_content"):
    print("思考过程:", getattr(msg, "reasoning_content"))
print("最终回答:", msg.content)
```

【Node.js】
``` javascript
import OpenAI from 'openai';

const client = new OpenAI({
  apiKey: 'YOUR_API_KEY',
  baseURL: 'https://tokenhub.tencentmaas.com/v1',
});

// Node.js SDK：thinking 字段直接展开到顶层
const response = await client.chat.completions.create({
  model: 'MODEL 参数值',
  messages: [
    { role: 'user', content: '小明有5个苹果，给了小红2个，又买了3个，最后还剩几个？' },
  ],
  thinking: { type: 'enabled' },
} as any);

const msg: any = response.choices[0].message;
if (msg.reasoning_content) console.log('思考过程:', msg.reasoning_content);
console.log('最终回答:', msg.content);
```

【Java】
``` java
import okhttp3.*;
import com.google.gson.Gson;
import java.util.*;

public class ThinkingChat {
    public static void main(String[] args) throws Exception {
        Map<String, Object> body = new HashMap<>();
        body.put("model", "MODEL 参数值");
        body.put("messages", List.of(
            Map.of("role", "user", "content", "小明有5个苹果，给了小红2个，又买了3个，最后还剩几个？")
        ));
        body.put("thinking", Map.of("type", "enabled"));

        Request request = new Request.Builder()
            .url("https://tokenhub.tencentmaas.com/v1/chat/completions")
            .header("Authorization", "Bearer YOUR_API_KEY")
            .post(RequestBody.create(new Gson().toJson(body), MediaType.parse("application/json")))
            .build();

        try (Response response = new OkHttpClient().newCall(request).execute()) {
            // 响应体中 message.reasoning_content 为思考过程，message.content 为最终回答
            System.out.println(response.body().string());
        }
    }
}
```

【Go】
``` go
package main

import (
    "bytes"
    "encoding/json"
    "fmt"
    "io"
    "net/http"
)

func main() {
    body, _ := json.Marshal(map[string]interface{}{
        "model": "MODEL 参数值",
        "messages": []map[string]string{
            {"role": "user", "content": "小明有5个苹果，给了小红2个，又买了3个，最后还剩几个？"},
        },
        "thinking": map[string]string{"type": "enabled"},
    })

    req, _ := http.NewRequest("POST",
        "https://tokenhub.tencentmaas.com/v1/chat/completions",
        bytes.NewBuffer(body))
    req.Header.Set("Authorization", "Bearer YOUR_API_KEY")
    req.Header.Set("Content-Type", "application/json")

    resp, _ := http.DefaultClient.Do(req)
    defer resp.Body.Close()

    data, _ := io.ReadAll(resp.Body)
    // 响应体中 message.reasoning_content 为思考过程，message.content 为最终回答
    fmt.Println(string(data))
}
```

### 调用示例：关闭深度思考

【cURL】
``` bash
curl -X POST 'https://tokenhub.tencentmaas.com/v1/chat/completions' \
  -H 'Content-Type: application/json' \
  -H 'Authorization: Bearer YOUR_API_KEY' \
  -d '{
    "model": "MODEL 参数值",
    "messages": [
      {"role": "user", "content": "小明有5个苹果，给了小红2个，又买了3个，最后还剩几个？"}
    ],
    "thinking": {"type": "disabled"},
    "stream": false
  }'
```

【Python】
``` python
from openai import OpenAI

client = OpenAI(
    api_key="YOUR_API_KEY",
    base_url="https://tokenhub.tencentmaas.com/v1",
)

response = client.chat.completions.create(
    model="MODEL 参数值",
    messages=[
        {"role": "user", "content": "小明有5个苹果，给了小红2个，又买了3个，最后还剩几个？"},
    ],
    extra_body={"thinking": {"type": "disabled"}},
)
print(response.choices[0].message.content)
```

【Node.js】
``` javascript
import OpenAI from 'openai';

const client = new OpenAI({
  apiKey: 'YOUR_API_KEY',
  baseURL: 'https://tokenhub.tencentmaas.com/v1',
});

const response = await client.chat.completions.create({
  model: 'MODEL 参数值',
  messages: [
    { role: 'user', content: '小明有5个苹果，给了小红2个，又买了3个，最后还剩几个？' },
  ],
  thinking: { type: 'disabled' },
} as any);
console.log(response.choices[0].message.content);
```

【Java】
``` java
import okhttp3.*;
import com.google.gson.Gson;
import java.util.*;

public class DisableThinking {
    public static void main(String[] args) throws Exception {
        Map<String, Object> body = new HashMap<>();
        body.put("model", "MODEL 参数值");
        body.put("messages", List.of(
            Map.of("role", "user", "content", "小明有5个苹果，给了小红2个，又买了3个，最后还剩几个？")
        ));
        body.put("thinking", Map.of("type", "disabled"));

        Request request = new Request.Builder()
            .url("https://tokenhub.tencentmaas.com/v1/chat/completions")
            .header("Authorization", "Bearer YOUR_API_KEY")
            .post(RequestBody.create(new Gson().toJson(body), MediaType.parse("application/json")))
            .build();

        try (Response response = new OkHttpClient().newCall(request).execute()) {
            System.out.println(response.body().string());
        }
    }
}
```

【Go】
``` go
package main

import (
    "bytes"
    "encoding/json"
    "fmt"
    "io"
    "net/http"
)

func main() {
    body, _ := json.Marshal(map[string]interface{}{
        "model": "MODEL 参数值",
        "messages": []map[string]string{
            {"role": "user", "content": "小明有5个苹果，给了小红2个，又买了3个，最后还剩几个？"},
        },
        "thinking": map[string]string{"type": "disabled"},
    })

    req, _ := http.NewRequest("POST",
        "https://tokenhub.tencentmaas.com/v1/chat/completions",
        bytes.NewBuffer(body))
    req.Header.Set("Authorization", "Bearer YOUR_API_KEY")
    req.Header.Set("Content-Type", "application/json")

    resp, _ := http.DefaultClient.Do(req)
    defer resp.Body.Close()

    data, _ := io.ReadAll(resp.Body)
    fmt.Println(string(data))
}
```

## 推理深度配置

通过 `reasoning_effort` 参数控制推理深度。该参数用于约束模型投入多少推理强度；推理强度越高，通常回答会更充分，但延迟和 token 消耗也会更高。

|`reasoning_effort` **的值**|**说明**|
|---------|---------|
|`low`|轻量推理，推理步数少，速度快，适合简单任务。|
|`medium`|平衡模式，适合大多数日常、逻辑适中的复杂任务。|
|`high`|深度推理，推理时间最长，思考最深入，适合高难度数学、编程或复杂逻辑推理任务，但延迟和成本最高。|

### 支持模型

|**模型名称**|**MODEL 参数值**|**说明**|
|---------|---------|---------|
|Hy4 preview|`hy4-preview`|默认 `high`，支持`none`、`high`|
|Hy3|`hy3`|默认 `high`|
|Hy3 preview|`hy3-preview`|默认 `low`|
|DeepSeek-V4-Flash|`deepseek-v4-flash`|默认 `high`|
|DeepSeek-V4-Pro|`deepseek-v4-pro`|默认 `high`|
|Deepseek-v3.2|`deepseek-v3.2`|默认 `high`|
|GLM-5.2|`glm-5.2`|默认 `max`，支持 `max`、`xhigh`、`high`、`medium`、`low`、`minimal`、`none`|
|GLM-5.3|`glm-5.3`|默认 `max`，支持 `low`、`high`、`max`|
|GLM-5.3-Flash|`glm-5.3-flash`|默认 `max`，支持 `low`、`high`、`max`|

### 调用示例：推理深度配置

【cURL】
``` bash
curl -X POST 'https://tokenhub.tencentmaas.com/v1/chat/completions' \
  -H 'Content-Type: application/json' \
  -H 'Authorization: Bearer YOUR_API_KEY' \
  -d '{
    "model": "MODEL 参数值",
    "messages": [
      {"role": "user", "content": "小明有5个苹果，给了小红2个，又买了3个，最后还剩几个？"}
    ],
    "stream": false,
    "temperature": 0.9,
    "reasoning_effort": "high"
  }'
```

【Python】
``` python
from openai import OpenAI

client = OpenAI(
    api_key="YOUR_API_KEY",
    base_url="https://tokenhub.tencentmaas.com/v1",
)

response = client.chat.completions.create(
    model="MODEL 参数值",
    messages=[
        {"role": "user", "content": "小明有5个苹果，给了小红2个，又买了3个，最后还剩几个？"},
    ],
    temperature=0.9,
    extra_body={"reasoning_effort": "high"},
)

msg = response.choices[0].message
if hasattr(msg, "reasoning_content"):
    print("思考过程:", getattr(msg, "reasoning_content"))
print("最终回答:", msg.content)
```

【Node.js】
``` javascript
import OpenAI from 'openai';

const client = new OpenAI({
  apiKey: 'YOUR_API_KEY',
  baseURL: 'https://tokenhub.tencentmaas.com/v1',
});

const response = await client.chat.completions.create({
  model: 'MODEL 参数值',
  messages: [
    { role: 'user', content: '小明有5个苹果，给了小红2个，又买了3个，最后还剩几个？' },
  ],
  temperature: 0.9,
  reasoning_effort: 'high',
} as any);

const msg: any = response.choices[0].message;
if (msg.reasoning_content) console.log('思考过程:', msg.reasoning_content);
console.log('最终回答:', msg.content);
```

【Java】
``` java
import okhttp3.*;
import com.google.gson.Gson;
import java.util.*;

public class ReasoningEffortChat {
    public static void main(String[] args) throws Exception {
        Map<String, Object> body = new HashMap<>();
        body.put("model", "MODEL 参数值");
        body.put("messages", List.of(
            Map.of("role", "user", "content", "小明有5个苹果，给了小红2个，又买了3个，最后还剩几个？")
        ));
        body.put("temperature", 0.9);
        body.put("reasoning_effort", "high");

        Request request = new Request.Builder()
            .url("https://tokenhub.tencentmaas.com/v1/chat/completions")
            .header("Authorization", "Bearer YOUR_API_KEY")
            .post(RequestBody.create(new Gson().toJson(body), MediaType.parse("application/json")))
            .build();

        try (Response response = new OkHttpClient().newCall(request).execute()) {
            System.out.println(response.body().string());
        }
    }
}
```

【Go】
``` go
package main

import (
    "bytes"
    "encoding/json"
    "fmt"
    "io"
    "net/http"
)

func main() {
    body, _ := json.Marshal(map[string]interface{}{
        "model": "MODEL 参数值",
        "messages": []map[string]string{
            {"role": "user", "content": "小明有5个苹果，给了小红2个，又买了3个，最后还剩几个？"},
        },
        "temperature":      0.9,
        "reasoning_effort": "high",
    })

    req, _ := http.NewRequest("POST",
        "https://tokenhub.tencentmaas.com/v1/chat/completions",
        bytes.NewBuffer(body))
    req.Header.Set("Authorization", "Bearer YOUR_API_KEY")
    req.Header.Set("Content-Type", "application/json")

    resp, _ := http.DefaultClient.Do(req)
    defer resp.Body.Close()

    data, _ := io.ReadAll(resp.Body)
    fmt.Println(string(data))
}
```

### 响应示例

启用思考后，响应中会附带 `reasoning_content` 思考过程字段：
``` json
{
  "id": "c95dc87ecce440678c3bb08f5868fee6",
  "object": "chat.completion",
  "created": 1775146546,
  "model": "MODEL 参数值",
  "choices": [
    {
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "答案是 6 个。",
        "reasoning_content": "用户现在需要解决的是小明苹果数量变化的问题，首先得理清楚每一步的变化。首先小明一开始有5个苹果，给了小红2个，那这时候应该减去2，对吧？然后又买了3个，这时候要加上3。所以计算的话就是5减2再加3。先算5-2=3，然后3+3=6。"
      },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 22,
    "completion_tokens": 264,
    "total_tokens": 286
  }
}
```

思考模式下的工具调用，需在每一轮请求都回填历史 `reasoning_content`，以获取最佳效果，详情请参见 [保留式思考模式（Preserved Thinking）](https://cloud.tencent.com/document/product/1823/133534)。

