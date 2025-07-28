# **Comparative Analysis Report of Mainstream Large Language Model Chat Completion APIs**

Source: Gemini Deep Research - [https://g.co/gemini/share/eb34cd4f188d](https://g.co/gemini/share/eb34cd4f188d)


## **Executive Summary**

This report aims to provide a comprehensive comparison of the chat completion APIs from three leading providers: OpenAI, Anthropic, and Google.

These APIs are the core of building conversational artificial intelligence applications, enabling models to understand and generate human language for multi-turn conversations, content creation, and automated tasks.

OpenAI's Chat Completions API (/v1/chat/completions) is known for its extensive model support and relatively intuitive request structure, with streaming responses using standard Server-Sent Events (SSE) format. Anthropic's Claude Messages API (/v1/messages) demonstrates strict design in conversation structure and safety, providing visibility into the model's internal thinking process through detailed event types in streaming responses. Google's Gemini API (generateContent method) has significant advantages in multimodal input processing, with streaming responses presented as incremental JSON objects rather than explicit SSE event types, while providing comprehensive safety and attribution information.

Overall, while these APIs all aim to achieve chat completion functionality, they differ significantly in message structure, parameter naming, streaming mechanisms, and support for multimodal and tool usage. These differences determine their applicability in different application scenarios and directly impact developers' integration strategies.

## **1. Introduction to Large Language Model Chat APIs**

Chat completion APIs are key components of modern artificial intelligence applications, enabling dynamic, context-aware conversations with large language models (LLMs). These APIs are widely used to build intelligent customer service systems, virtual assistants, interactive content generation tools, educational platforms, and any application requiring human-like conversation simulation. Their core value lies in maintaining conversational context and generating coherent, relevant responses based on user input, greatly enhancing user experience and automation levels.

OpenAI, Anthropic, and Google, as leaders in the generative AI field, each provide powerful chat completion APIs. OpenAI, with its extensive model portfolio and easy integration features, has driven the popularization of LLM technology. Anthropic is renowned for its emphasis on model safety and controllability, committed to building safer, more interpretable AI systems. Google, leveraging its deep accumulation in AI research, demonstrates strong capabilities in multimodal understanding and generation through its Gemini models.

The following table summarizes the basic access information for these three major APIs:

**Table 1: API Endpoints and Authentication Summary**

| Provider      | API Name                            | Endpoint                                                                                                                                                       | Authentication Method                                          | API Versioning Method                           |
| :------------ | :---------------------------------- | :------------------------------------------------------------------------------------------------------------------------------------------------------------- | :------------------------------------------------------------- | :---------------------------------------------- |
| OpenAI        | OpenAI Chat Completions API         | https://api.openai.com/v1/chat/completions                                                                                                                     | Bearer Token (API Key)                                         | Azure OpenAI uses api-version query parameter ¹ |
| Anthropic     | Anthropic Claude Messages API       | {{baseUrl}}/{{version}}/messages (e.g., https://api.anthropic.com/v1/messages)                                                                                 | Bearer Token (x-api-key header)                                | Uses anthropic-version header ²                 |
| Google Gemini | Google Gemini API (generateContent) | https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent or https://generativelanguage.googleapis.com/v1/models/{model}:generateContent | API Key (x-goog-api-key header) or Google Cloud authentication | Set via apiVersion parameter in SDK ⁴           |

This table provides developers with a quick reference, summarizing the basic information needed to integrate these APIs, including endpoint URLs, authentication mechanisms, and versioning methods. It reveals the universality of API key authentication while also pointing out provider-specific header requirements or more complex authentication processes (such as Google Cloud Vertex AI).

## **2. OpenAI Chat Completions API (/v1/chat/completions)**

OpenAI's Chat Completions API is its core conversational AI interface, designed specifically for handling multi-turn conversations, with request input requiring adherence to a specific message history format ¹⁰. This differs from the earlier /v1/completions endpoint used for single string prompts ¹.

### **2.1. Request Parameters: Complete Specification**

The API uses POST method with endpoint https://api.openai.com/v1/chat/completions ¹⁰. Authentication is achieved by including Authorization: Bearer YOUR_API_KEY in the request header ¹².

**Core Conversation Fields:**

* messages (array of objects, **required**): A list of messages comprising the conversation. The model is trained for specific roles.
  * **Constraints:** Each object must contain role and content.
  * **Roles:** system, user, assistant, tool ¹¹.
    * system role: Provides initial instructions or context ¹³.
    * user role: Represents user input.
    * assistant role: Represents model responses.
    * tool role: Used for tool outputs.
  * content (string or array of content parts, required): The text content of the message. For multimodal models (like gpt-4o), can be a string or array of content parts (supporting images).
  * name (string, optional): Optional name of the participant, used to distinguish different participants of the same role ¹³.
  * tool_calls (array of objects, optional): Tool calls included in assistant messages, containing id and function (name, parameters).
  * tool_call_id (string, optional): For tool messages, the ID of the tool call this message responds to.
* model (string, **required**): The model ID to use (e.g., gpt-3.5-turbo, gpt-4o, gpt-4o-mini) ¹¹.
  * **Constraints:** Supports specific models; consult documentation for latest list ¹³.

**Generation Control Parameters:**

* max_tokens (integer, optional): The maximum number of tokens that can be generated in the completion.
  * **Constraints:** Prompt tokens + max_tokens cannot exceed the model's context length ¹.
  * **Default:** 16 ¹.
* temperature (number, optional): Sampling temperature. Higher values (e.g., 0.8) make output more random, lower values (e.g., 0.2) make it more focused and deterministic.
  * **Constraints:** Range 0 to 2 ¹².
  * **Default:** 1.0.
* top_p (number, optional): Nucleus sampling. The model considers tokens with cumulative probability exceeding top_p.
  * **Constraints:** Range 0 to 1 ¹².
  * **Default:** 1.0.
* n (integer, optional): How many chat completion choices to generate for each input message.
  * **Constraints:** Can quickly consume token quota ¹.
  * **Default:** 1 ¹.
  * **Note:** Some compatible APIs (like Langdock) don't support this parameter ¹³.
* stream (boolean, optional): If true, partial message deltas will be sent as Server-Sent Events (SSE).
  * **Default:** false ¹².
* stop (string or array of strings, optional): Up to 4 sequences where the API will stop generating further tokens. The returned text will not contain the stop sequence ¹.
* seed (integer, optional): If specified, the system will make a best effort at deterministic sampling, but cannot guarantee the same result for each request ¹.

**Advanced Features:**

* presence_penalty (number, optional): Penalizes new tokens based on whether they appear in the text so far. Positive values increase the model's likelihood to talk about new topics.
  * **Constraints:** Range -2.0 to 2.0 ¹.
  * **Default:** 0.0 ¹.
* frequency_penalty (number, optional): Penalizes new tokens based on their existing frequency in the text so far. Positive values decrease the model's likelihood to repeat the same line verbatim.
  * **Constraints:** Range -2.0 to 2.0 ¹.
  * **Default:** 0.0 ¹.
* logit_bias (object, optional): Modify the likelihood of specified tokens appearing by mapping token IDs to bias values (-100 to 100).
  * **Constraints:** Bias values range -100 to 100 ¹.
* user (string, optional): A unique identifier representing the end-user, which can help monitor and detect abuse ¹.
* tools (array of objects, optional): A list of tools the model may call. Currently, only function type tools are supported. Each tool contains name, description (recommended), and parameters (JSON schema of tool input) ¹⁶.
* tool_choice (string or object, optional): Controls which (if any) tool is called by the model.
  * **Values:** none (model won't call tools and generates a message), auto (model decides), required (model must call one or more tools). Can also specify a particular tool via {"type": "function", "function": {"name": "my_function"}} ¹³.
  * **Default:** none if no tools present; auto if tools exist ¹³.
* response_format (object, optional): Specifies the format that the model must output.
  * **type:** (enum<string>, required) text or json_object.
  * **Constraints:** When using json_object, must explicitly instruct the model to generate JSON in the prompt to avoid infinite blank character generation ¹³.
* logprobs (boolean, optional): Whether to return log probabilities of the output tokens.
  * **Default:** false ¹⁶.
  * **Note:** In streaming mode, only returns information about selected tokens, not full log probabilities ¹⁶.
* stream_options (object, optional): Options for streaming responses, only set when stream: true.
  * **include_usage** (boolean, optional): If true, will stream an additional chunk before the data: [DONE] message with token usage statistics for the entire request ¹⁶.
  * **Note:** Some compatible APIs (like Langdock) don't support this parameter ¹³.
* service_tier (enum<string>, optional): Determines whether to use priority capacity (priority_only) or standard capacity (standard_only).
  * **Available options:** auto, standard_only.
  * **Note:** Some compatible APIs (like Langdock) don't support this parameter ¹³.**Ta
ble 2: OpenAI Chat Completions API - Request Parameters**

| Parameter Name    | Type                       | Required/Optional | Description                                                          | Constraints/Default                                                                                                                  | Example Value                                                                                          |
| :---------------- | :------------------------- | :---------------- | :------------------------------------------------------------------- | :----------------------------------------------------------------------------------------------------------------------------------- | :----------------------------------------------------------------------------------------------------- |
| messages          | array of objects           | Required          | List of messages comprising the conversation.                        | Each object contains role and content. Role can be system, user, assistant, tool. Content can be string or multimodal content array. | [{"role": "user", "content": "Hello"}]                                                                 |
| model             | string                     | Required          | The model ID to use.                                                 | Length 1-256 characters.                                                                                                             | "gpt-4o"                                                                                               |
| max_tokens        | integer                    | Optional          | Maximum number of tokens that can be generated in the completion.    | Prompt tokens + max_tokens <= model context length. Default: 16.                                                                     | 1024                                                                                                   |
| temperature       | number                     | Optional          | Sampling temperature.                                                | Range: 0 to 2. Default: 1.0.                                                                                                         | 0.7                                                                                                    |
| top_p             | number                     | Optional          | Nucleus sampling.                                                    | Range: 0 to 1. Default: 1.0.                                                                                                         | 0.9                                                                                                    |
| n                 | integer                    | Optional          | How many chat completion choices to generate for each input message. | Default: 1. Note: Some compatible APIs don't support.                                                                                | 1                                                                                                      |
| stream            | boolean                    | Optional          | Whether to stream response as SSE.                                   | Default: false.                                                                                                                      | true                                                                                                   |
| stop              | string or array of strings | Optional          | Sequences to stop token generation.                                  | Up to 4 sequences.                                                                                                                   | ["\nUser:", "###"]                                                                                     |
| seed              | integer                    | Optional          | Random seed for deterministic sampling.                              | No guarantee of complete determinism.                                                                                                | 1234                                                                                                   |
| presence_penalty  | number                     | Optional          | Penalizes new tokens based on whether they already appear.           | Range: -2.0 to 2.0. Default: 0.0.                                                                                                    | 0.5                                                                                                    |
| frequency_penalty | number                     | Optional          | Penalizes new tokens based on their existing frequency in text.      | Range: -2.0 to 2.0. Default: 0.0.                                                                                                    | 0.5                                                                                                    |
| logit_bias        | object                     | Optional          | Modify likelihood of specified tokens appearing.                     | Maps token IDs to bias values (-100 to 100).                                                                                         | {"1234": 50}                                                                                           |
| user              | string                     | Optional          | Unique identifier representing the end-user.                         |                                                                                                                                      | "user-123"                                                                                             |
| tools             | array of objects           | Optional          | List of tools the model may call.                                    | Only supports function type. Contains name, description, parameters.                                                                 | [{"type": "function", "function": {"name": "get_weather", "description": "...", "parameters": {...}}}] |
| tool_choice       | string or object           | Optional          | Controls which tool the model calls.                                 | none, auto, required or specify tool. Default: none or auto.                                                                         | "auto" or {"type": "function", "function": {"name": "get_weather"}}                                    |
| response_format   | object                     | Optional          | Specifies model output format.                                       | type can be text or json_object. json_object needs explicit instruction in prompt.                                                   | {"type": "json_object"}                                                                                |
| logprobs          | boolean                    | Optional          | Whether to return log probabilities of output tokens.                | Default: false. In streaming mode only returns selected token info.                                                                  | true                                                                                                   |
| stream_options    | object                     | Optional          | Streaming response options.                                          | Only set when stream: true. Contains include_usage (boolean).                                                                        | {"include_usage": true}                                                                                |
| service_tier      | enum<string>               | Optional          | Determines capacity usage type.                                      | auto, standard_only, priority_only.                                                                                                  | "standard_only"                                                                                        |

OpenAI's API design evolution reflects its adaptation and optimization for the conversational AI paradigm. From the early /completions endpoint that accepted single string prompts to the current /chat/completions endpoint requiring structured message history, this represents a fundamental shift in the model's underlying training and application scenarios ¹. This transformation not only affects request format but also reflects the model's enhanced ability to handle multi-turn conversations and maintain context. For developers, this means migrating from legacy integrations to the new interface requires significant adjustments to request and response handling logic. However, OpenAI maintains backward compatibility for non-chat use cases by preserving the old endpoint, while promoting a more powerful, conversation-appropriate new standard.

Additionally, OpenAI's maturity in tool integration is noteworthy. The request parameters include tools and tool_choice, with responses containing tool_calls ¹³, indicating its function calling capabilities are quite sophisticated. The required option in the tool_choice parameter ¹³ allows developers to force the model to call specific tools, which is powerful for building complex agent workflows. The standardized tool_calls structure in model responses provides a clear interface for applications to parse and execute external functions, enabling complex interactions with real-world data or services. Streaming support for tool_calls (implicitly implemented through delta.tool_calls) means applications can respond incrementally to tool calls, potentially accelerating the execution of complex agent loops.

### **2.2. Synchronous Response Format (JSON)**

For synchronous requests (stream: false), the API returns a JSON object representing a completed chat interaction.

**Root Object Structure:**

* id (string, required): Unique ID of the chat response ¹⁴.
* object (string, required): Object type, always "chat.completion" ¹⁴.
* created (integer, required): Unix timestamp of when the chat completion was created ¹⁴.
* model (string, required): The model ID used to create the chat completion ¹⁴.
* choices (array of objects, required): List of model response options. Length corresponds to the n parameter in the request body (default 1) ¹⁴.
  * index (integer, required): Index of the choice in the list ¹⁶.
  * message (object, required): Chat completion message generated by the model ¹⁶.
    * role (string, required): Role of the message author, typically "assistant" ¹⁶.
    * content (string, required): Content of the message ¹⁶. May be null if tool_calls are present.
    * tool_calls (array of objects, optional): Array of tool calls generated by the model ¹⁶.
      * id (string, required): ID of the tool call.
      * type (string, required): Type of tool call, always "function".
      * function (object, required): Function the model wants to call.
        * name (string, required): Name of the function to call.
        * arguments (string, required): Arguments to use when calling the function, represented as a JSON string.
  * finish_reason (string or null, required): Reason the model stopped generating tokens.
    * **Possible values:** stop (natural stop or stop sequence), length (reached max tokens), tool_calls (model requested tool call), content_filter (content violation), function_call (deprecated, replaced by tool_calls) ¹⁶.
  * logprobs (object or null, optional): Log probability information for the choice ¹⁶.
    * content (array of objects, optional): List of log probability data for each token in the completion.
      * token (string, required): The token.
      * logprob (number, required): Log probability of the token.
      * bytes (array of integers, optional): Byte representation of the token.
      * top_logprobs (array of objects, optional): List of most likely tokens and their log probabilities at this position.
* usage (object, required): Usage statistics for the completion request ¹⁴.
  * prompt_tokens (integer, required): Number of tokens in the prompt ¹⁴.
  * completion_tokens (integer, required): Number of tokens in the generated completion ¹⁴.
  * total_tokens (integer, required): Total tokens used (prompt + completion) ¹⁶.
* system_fingerprint (string, optional): Unique identifier of the backend configuration that generated the response. Can be used to monitor backend changes ¹.

**Table 3: OpenAI Chat Completions API - Synchronous Response Fields**

| Field Path                                    | Type             | Description                                                                 | Possible Values/Structure                                     |
| :-------------------------------------------- | :--------------- | :-------------------------------------------------------------------------- | :------------------------------------------------------------ |
| id                                            | string           | Unique ID of the chat response.                                             |                                                               |
| object                                        | string           | Object type.                                                                | "chat.completion"                                             |
| created                                       | integer          | Unix timestamp when chat completion was created.                            |                                                               |
| model                                         | string           | Model ID used to create the chat completion.                                |                                                               |
| choices                                       | array of objects | List of model response options.                                             | Length corresponds to n parameter.                            |
| choices.index                                 | integer          | Index of the choice in the list.                                            |                                                               |
| choices.message                               | object           | Chat completion message generated by the model.                             |                                                               |
| choices.message.role                          | string           | Role of the message author.                                                 | "assistant"                                                   |
| choices.message.content                       | string           | Content of the message.                                                     | May be null if tool_calls present.                            |
| choices.message.tool_calls                    | array of objects | Array of tool calls generated by the model.                                 |                                                               |
| choices.message.tool_calls.id                 | string           | ID of the tool call.                                                        |                                                               |
| choices.message.tool_calls.type               | string           | Type of tool call.                                                          | "function"                                                    |
| choices.message.tool_calls.function           | object           | Function the model wants to call.                                           |                                                               |
| choices.message.tool_calls.function.name      | string           | Name of the function to call.                                               |                                                               |
| choices.message.tool_calls.function.arguments | string           | Arguments for calling the function (JSON string).                           |                                                               |
| choices.finish_reason                         | string or null   | Reason the model stopped generating tokens.                                 | stop, length, tool_calls, content_filter, function_call.      |
| choices.logprobs                              | object or null   | Log probability information for the choice.                                 | content array containing token, logprob, bytes, top_logprobs. |
| usage                                         | object           | Usage statistics for the completion request.                                |                                                               |
| usage.prompt_tokens                           | integer          | Number of tokens in the prompt.                                             |                                                               |
| usage.completion_tokens                       | integer          | Number of tokens in the generated completion.                               |                                                               |
| usage.total_tokens                            | integer          | Total tokens used.                                                          |                                                               |
| system_fingerprint                            | string           | Unique identifier of the backend configuration that generated the response. |                                                               |

### **2.3. Streaming Response Format (Server-Sent Events)**

When stream: true is set in the request, OpenAI's API sends partial message deltas as Server-Sent Events (SSE). Each event is a JSON object prefixed with data:, and the stream terminates with a data: [DONE] message ¹⁶.

**SSE Event Structure:**

* Each event is a line starting with data: followed by a JSON object.
* The final event is data: [DONE].

**Difference between choices.delta object and synchronous message object:**

In streaming, the choices array contains delta objects instead of message objects ¹⁶. The delta object represents incremental changes to the message. It may contain role (usually only in the first chunk), content (partial string), or tool_calls (partial tool call objects). The content field in delta is a string fragment that needs to be concatenated to form the complete message content. tool_calls in delta can also be streamed incrementally and need to be accumulated. finish_reason will be null before generation stops and will be populated in the final delta chunk when stopping ¹⁶.

logprobs are also streamed incrementally ¹⁶.

**Incremental Updates and Final Usage Chunk:**

The usage field will be null for most streaming chunks. If stream_options.include_usage is set in the request, an additional chunk will be streamed before data: [DONE] containing total usage statistics for the entire request ¹⁶. This is a key feature for real-time token accounting.

**Table 4: OpenAI Chat Completions API - Streaming Event Types and Deltas**

| Event Type   | Description                                 | Key Fields in Delta                                                                   | Example Delta Structure                                                                                                                                                                                 |
| :----------- | :------------------------------------------ | :------------------------------------------------------------------------------------ | :------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| data: {... } | Initial or intermediate message delta.      | choices.index, choices.delta (role, content, tool_calls), model, created, id, object. | {"id": "chatcmpl-...", "object": "chat.completion.chunk", "created": 1234567890, "model": "gpt-4o", "choices": [{"index": 0, "delta": {"role": "assistant"}, "logprobs": null, "finish_reason": null}]} |
| data: {... } | Text content delta.                         | choices.delta.content                                                                 | {"choices": [{"index": 0, "delta": {"content": "Hello"}}], "id": "...", "object": "chat.completion.chunk",...}                                                                                          |
| data: {... } | Tool call delta.                            | choices.delta.tool_calls                                                              | {"choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "function": {"arguments": "{\\"location\\""}}]}}], "id": "...", "object": "chat.completion.chunk",...}                                  |
| data: {... } | Final message delta (contains stop reason). | choices.finish_reason                                                                 | {"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}], "id": "...", "object": "chat.completion.chunk",...}                                                                                   |
| data: {... } | Final usage statistics chunk.               | usage (prompt_tokens, completion_tokens, total_tokens)                                | {"id": "...", "object": "chat.completion.chunk", "created": 1234567890, "model": "gpt-4o", "choices": [], "usage": {"prompt_tokens": 10, "completion_tokens": 20, "total_tokens": 30}}                  |
| data: [DONE] | Stream end marker.                          | None                                                                                  | data: [DONE]                                                                                                                                                                                            |

OpenAI's streaming design provides a balance between fine-grained control and simplified implementation through the stream_options.include_usage parameter. This design avoids repeating usage data in every small incremental chunk, reducing payload size, while still providing total usage information without sending additional non-streaming requests ¹⁶. For applications requiring real-time token cost estimation, this feature is crucial as it enables accurate billing and quota management at the end of streaming conversations. This shows OpenAI made trade-offs in streaming efficiency while still providing important metadata.## **3. A
nthropic Claude Messages API (/v1/messages)**

Anthropic's Messages API is designed for powerful conversational AI, with both request and response structures reflecting strict control over conversation flow and safety.

### **3.1. Request Parameters: Complete Specification**

Anthropic Claude Messages API uses the /v1/messages endpoint ².

* **Endpoint:** POST {{baseUrl}}/{{version}}/messages (e.g., https://api.anthropic.com/v1/messages) ².
* **Authentication:** Include x-api-key (string, **required**) in request header ³.
* **Request Headers:**
  * anthropic-version (string, **required**): Specifies API version, e.g., 2023-06-01 ².
  * anthropic-beta (string, optional): Specifies beta version, comma-separated or multiple headers ³.
  * content-type: application/json ².

**Core Conversation Fields:**

* messages (array of objects, **required**): List of structured input messages ².
  * **Constraints:** The model is trained on alternating user and assistant turns. Consecutive turns of the same role in the request will be merged.
  * **Each message object:** Must contain role and content ³.
  * role: (user or assistant) ³.
  * content: (string or array of content blocks, required)
    * Can be a single string (shorthand for [{"type": "text", "text": "..."}]) ³.
    * Can be an array of content blocks, each with a type.
    * **type: "text":** text (string, required) - Plain text content ³.
    * **type: "image":** source (object, required) - Image content.
      * type (string, required): base64.
      * media_type (string, required): image/jpeg, image/png, image/gif, image/webp ³.
      * data (string, required): Base64-encoded image data ³.
    * **type: "tool_use":** Used for model-generated tool calls. Contains id, name, input (JSON object).
    * **type: "tool_result":** Used to return tool output to the model. Contains tool_use_id and content (array of text blocks or string).
  * **Constraints:** If the last message uses the assistant role, response content will continue immediately from that message's content ³.
  * **Limit:** Maximum 100,000 messages in a single request ³.
* model (string, **required**): The model that will complete the prompt (e.g., claude-3-opus-20240229, claude-3-5-sonnet-latest) ².
  * **Constraints:** Length 1-256 characters ³.
* max_tokens (integer, **required**): Maximum number of tokens to generate before stopping.
  * **Constraints:** x >= 1. Different models have different maximums ².

**Context and Control Parameters:**

* system (string or array of text content blocks, optional): System prompt providing context and instructions ³.
* temperature (number, optional): Amount of randomness in the response.
  * **Constraints:** Range 0.0 to 1.0. Close to 0.0 for analytical tasks, close to 1.0 for creative tasks ³.
  * **Default:** 1.0 ³.
* stop_sequences (array of strings, optional): Custom text sequences that cause the model to stop generating. If matched, stop_reason will be "stop_sequence" ³.
* top_k (integer, optional): Only sample from the top K options for each subsequent token.
  * **Constraints:** x >= 0. Only recommended for advanced use cases ³.
* top_p (number, optional): Nucleus sampling.
  * **Constraints:** Range 0 to 1. Only recommended for advanced use cases ³.

**Advanced Features:**

* stream (boolean, optional): Whether to incrementally stream the response using server-sent events.
  * **Default:** false.
* metadata (object, optional): Object describing metadata about the request.
  * **user_id** (string, optional): External identifier for the user (UUID, hash, or opaque identifier). Maximum length 256 ³.
* service_tier (enum<string>, optional): Determines whether to use priority capacity (priority_only) or standard capacity (standard_only).
  * **Available options:** auto, standard_only ³.
  * **Default:** auto ³.
* thinking (object, optional): Configuration for enabling Claude's extended thinking.
  * **Constraints:** Requires at least 1,024 tokens budget and counts toward max_tokens limit ³.
  * **type** (enum<string>, required): enabled ³.
  * **budget_tokens** (integer, required): Number of tokens Claude can use for internal reasoning. x >= 1024 and less than max_tokens ³.
* tool_choice (object, optional): How the model should use the provided tools.
  * **type** (enum<string>, required): auto ³.
  * **disable_parallel_tool_use** (boolean, optional): If true, the model will output at most one tool use. Default false ³.
* tools (array of objects, optional): Definitions of tools the model may use.
  * Each tool: name (string), description (string, strongly recommended), input_schema (JSON schema of tool input shape) ³.
  * **Supported tool types:** custom, Bash tool, Code execution tool, Computer use tool, Text editor tool, Web search tool ³.
* cache_control (object, optional): Create a cache control breakpoint at this content block.
  * **type** (enum<string>, required): ephemeral ³.
  * **ttl** (enum<string>, optional): Time-to-live for the breakpoint. Options: 5m, 1h. Default 5m ³.
* container (string or null, optional): Container identifier for reuse across requests ³.
* mcp_servers (array of objects, optional): MCP servers that will be used.
  * Each server: name (string, required), type (enum<string>, required, url), url (string, required), authorization_token (string or null, optional), tool_configuration (object or null, optional) ³.

**Table 5: Anthropic Claude Messages API - Request Parameters**

| Parameter Name | Type                                   | Required/Optional | Description                                                    | Constraints/Default                                                                                                                | Example Value                                                                                                                 |
| :------------- | :------------------------------------- | :---------------- | :------------------------------------------------------------- | :--------------------------------------------------------------------------------------------------------------------------------- | :---------------------------------------------------------------------------------------------------------------------------- |
| messages       | array of objects                       | Required          | List of input messages.                                        | Contains role (user/assistant) and content. Content can be string or array of content blocks (text, image, tool use, tool result). | [{"role": "user", "content": "Hello"}, {"role": "assistant", "content": "Hi!"}, {"role": "user", "content": "Explain LLMs."}] |
| model          | string                                 | Required          | The model that will complete the prompt.                       | Length 1-256 characters.                                                                                                           | "claude-3-opus-20240229"                                                                                                      |
| max_tokens     | integer                                | Required          | Maximum number of tokens to generate before stopping.          | x >= 1.                                                                                                                            | 1024                                                                                                                          |
| system         | string or array of text content blocks | Optional          | System prompt providing context and instructions.              |                                                                                                                                    | "You are a helpful assistant."                                                                                                |
| temperature    | number                                 | Optional          | Amount of randomness in the response.                          | Range: 0.0 to 1.0. Default: 1.0.                                                                                                   | 0.7                                                                                                                           |
| stop_sequences | array of strings                       | Optional          | Custom text sequences that cause the model to stop generating. |                                                                                                                                    | ["\nUser:", "###"]                                                                                                            |
| top_k          | integer                                | Optional          | Only sample from the top K options.                            | x >= 0. Only recommended for advanced use cases.                                                                                   | 50                                                                                                                            |
| top_p          | number                                 | Optional          | Nucleus sampling.                                              | Range: 0 to 1. Only recommended for advanced use cases.                                                                            | 0.9                                                                                                                           |
| stream         | boolean                                | Optional          | Whether to incrementally stream the response using SSE.        | Default: false.                                                                                                                    | true                                                                                                                          |
| metadata       | object                                 | Optional          | Metadata about the request.                                    | Contains user_id (string, max length 256).                                                                                         | {"user_id": "user-abc-123"}                                                                                                   |
| service_tier   | enum<string>                           | Optional          | Determines capacity usage type.                                | auto, standard_only, priority_only. Default: auto.                                                                                 | "priority_only"                                                                                                               |
| thinking       | object                                 | Optional          | Configuration for enabling extended thinking.                  | Contains type (enabled) and budget_tokens (integer, >=1024 and <max_tokens).                                                       | {"type": "enabled", "budget_tokens": 2048}                                                                                    |
| tool_choice    | object                                 | Optional          | How the model should use the provided tools.                   | Contains type (auto) and disable_parallel_tool_use (boolean).                                                                      | {"type": "auto", "disable_parallel_tool_use": true}                                                                           |
| tools          | array of objects                       | Optional          | Definitions of tools the model may use.                        | Contains name, description, input_schema.                                                                                          | [{"name": "get_weather", "description": "...", "input_schema": {...}}]                                                        |
| cache_control  | object                                 | Optional          | Create a cache control breakpoint.                             | Contains type (ephemeral) and ttl (5m, 1h).                                                                                        | {"type": "ephemeral", "ttl": "1h"}                                                                                            |
| container      | string or null                         | Optional          | Container identifier for reuse.                                |                                                                                                                                    | "my-container-id"                                                                                                             |
| mcp_servers    | array of objects                       | Optional          | MCP servers that will be used.                                 | Contains name, type (url), url, authorization_token, tool_configuration.                                                           | [{"name": "server1", "type": "url", "url": "https://..."}]                                                                    |

Anthropic demonstrates high regard for explicit conversation structure and safety in its API design. Its messages parameter strictly requires alternating user and assistant roles ³, which is more stringent than OpenAI. Additionally, the system prompt is a separate top-level parameter ³, distinct from the messages array. This design choice indicates Anthropic's commitment to controlling conversation flow and providing clear, consistent context to the model. The separate system prompt may help make model behavior more robust, reducing instruction dilution due to conversation turns. This may be one reason why Claude is more "alignable" or controllable in specific tasks, as well as its safety-first approach.

Anthropic's integration of multimodal and tools also reflects its design philosophy as core functionality. The content field in messages explicitly supports arrays of content blocks, including images of various media_types ³. Similarly, tool_use and tool_result are defined as first-class content block types ³. This indicates that multimodal and tool calling are deeply integrated into Claude's core API design, rather than being add-on features. The structured content blocks make it simple to combine different modalities and manage tool interactions within a single conversation turn. This design promotes a unified approach to building multimodal, agent applications, potentially simplifying development compared to APIs that might handle these features through separate endpoints or less integrated structures.

### **3.2. Synchronous Response Format (JSON)**

For synchronous requests with stream: false, the response is a single Message object ³.

**Root Object Structure:**

* id (string, required): Unique object identifier ³.
* type (enum<string>, required): Object type, always "message" ³.
* role (enum<string>, required): Conversational role that generated the message, always "assistant" ³.
* content (array of objects, required): Content generated by the model. This is an array of content blocks, each with a type ³.
  * **type: "text":** text (string, required) - e.g., [{"type": "text", "text": "Hi, I'm Claude."}] ³.
  * **type: "thinking":** thinking (string, required) - Claude's internal reasoning process ³.
  * **type: "redacted_thinking":** thinking (string, required) - Redacted thinking content.
  * **type: "tool_use":** id (string, required), name (string, required), input (object, required) - Model's use of a tool ³.
  * **type: "server_tool_use":** tool_name (string, required), tool_id (string, required), input (object, required) - Server-side tool use.
  * **type: "web_search_tool_result":** tool_name (string, required), tool_id (string, required), output (object, required) - Results from web search tool.
  * **type: "code_execution_tool_result":** tool_name (string, required), tool_id (string, required), output (object, required) - Results from code execution tool.
  * **type: "mcp_tool_use":** id (string, required), tool_name (string, required), input (object, required) - MCP tool use.
  * **type: "mcp_tool_result":** tool_use_id (string, required), output (object, required) - MCP tool result.
  * **type: "container_upload":** id (string, required), tool_name (string, required), input (object, required) - Container upload.
* model (string, required): The model that processed the request ³.
* stop_reason (enum<string> or null, required): The reason the model stopped. Non-null in non-streaming mode ³.
  * **Possible values:** "end_turn", "max_tokens", "stop_sequence", "tool_use", "pause_turn", "refusal" ³.
* stop_sequence (string or null, required): Which custom stop sequence was generated, if any ³.
* usage (object, required): Billing and rate-limit usage ³.
  * **input_tokens** (integer, required): Number of input tokens used ³.
  * **output_tokens** (integer, required): Number of output tokens used ³.
  * **cache_creation_input_tokens** (integer or null, required): Number of input tokens used to create cache entries ³.
  * **cache_read_input_tokens** (integer or null, required): Number of input tokens read from cache ³.
  * **cache_creation** (object or null, required): Breakdown of cache tokens by TTL ³.
    * **ephemeral_1h_input_tokens** (integer, required): Number of input tokens for 1-hour cache ³.
    * **ephemeral_5m_input_tokens** (integer, required): Number of input tokens for 5-minute cache ³.
  * **server_tool_use** (object or null, required): Number of server tool requests ³.
    * **web_search_requests** (integer, required): Number of web search tool requests ³.
  * **service_tier** (enum<string> or null, required): The tier used (priority_only, standard_only, batch) ³.
* container (object or null, required): If container tools were used, contains information about the container used ³.
  * **expires_at** (string, required): When the container expires ³.
  * **id** (string, required): Identifier of the container ³.

**Table 6: Anthropic Claude Messages API - Synchronous Response Fields**

| Field Path                        | Type                 | Description                                                                      | Possible Values/Structure                                                                                                                                                      |
| :-------------------------------- | :------------------- | :------------------------------------------------------------------------------- | :----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| id                                | string               | Unique object identifier.                                                        |                                                                                                                                                                                |
| type                              | enum<string>         | Object type.                                                                     | "message"                                                                                                                                                                      |
| role                              | enum<string>         | Conversational role that generated the message.                                  | "assistant"                                                                                                                                                                    |
| content                           | array of objects     | Array of content blocks generated by the model.                                  | type can be text, thinking, redacted_thinking, tool_use, server_tool_use, web_search_tool_result, code_execution_tool_result, mcp_tool_use, mcp_tool_result, container_upload. |
| content.type                      | string               | Content block type.                                                              | e.g., "text", "tool_use".                                                                                                                                                      |
| content.text                      | string               | Text content (when type is text).                                                |                                                                                                                                                                                |
| content.thinking                  | string               | Model's internal reasoning process (when type is thinking or redacted_thinking). |                                                                                                                                                                                |
| content.id                        | string               | ID for tool use or container upload.                                             |                                                                                                                                                                                |
| content.name                      | string               | Tool name (when type is tool_use).                                               |                                                                                                                                                                                |
| content.input                     | object               | Tool input (when type is tool_use).                                              | JSON object.                                                                                                                                                                   |
| content.tool_name                 | string               | Server tool name (when type is server_tool_use).                                 |                                                                                                                                                                                |
| content.tool_id                   | string               | Server tool ID (when type is server_tool_use).                                   |                                                                                                                                                                                |
| content.output                    | object               | Tool output (when type is web_search_tool_result, etc.).                         |                                                                                                                                                                                |
| model                             | string               | The model that processed the request.                                            |                                                                                                                                                                                |
| stop_reason                       | enum<string> or null | The reason the model stopped.                                                    | end_turn, max_tokens, stop_sequence, tool_use, pause_turn, refusal.                                                                                                            |
| stop_sequence                     | string or null       | Which custom stop sequence was matched.                                          |                                                                                                                                                                                |
| usage                             | object               | Billing and rate-limit usage.                                                    |                                                                                                                                                                                |
| usage.input_tokens                | integer              | Number of input tokens.                                                          |                                                                                                                                                                                |
| usage.output_tokens               | integer              | Number of output tokens.                                                         |                                                                                                                                                                                |
| usage.cache_creation_input_tokens | integer or null      | Number of input tokens used to create cache entries.                             |                                                                                                                                                                                |
| usage.cache_read_input_tokens     | integer or null      | Number of input tokens read from cache.                                          |                                                                                                                                                                                |
| usage.cache_creation              | object or null       | Breakdown of cache tokens by TTL.                                                | Contains ephemeral_1h_input_tokens, ephemeral_5m_input_tokens.                                                                                                                 |
| usage.server_tool_use             | object or null       | Number of server tool requests.                                                  | Contains web_search_requests.                                                                                                                                                  |
| usage.service_tier                | enum<string> or null | The tier used.                                                                   | priority_only, standard_only, batch.                                                                                                                                           |
| container                         | object or null       | Container information.                                                           | Contains expires_at, id.                                                                                                                                                       |

### **3.3. Streaming Response Format (Server-Sent Events)**

When stream: true is set, Anthropic's API uses Server-Sent Events (SSE) to incrementally deliver responses ³. Each event contains a named event type and associated JSON data ¹⁷.

SSE Event Flow ¹⁷:

1. **message_start**: Initial event. Contains a Message object with empty content. stop_reason is null ³.
2. **Content blocks**: A series of events for each content block:
   * **content_block_start**: Marks the beginning of a content block. Contains the block's index and type ¹⁷.
   * **One or more content_block_delta events**: Provide incremental updates to the content block. Each delta has an index and a delta object ¹⁷.
     * **text_delta**: For text content. Contains text (string fragment) ¹⁷.
     * **input_json_delta**: For tool_use content blocks. Contains partial_json (string fragment). These are partial JSON strings that need to be accumulated and parsed at content_block_stop ¹⁷.
     * **thinking_delta**: When extended thinking is enabled. Contains thinking (string fragment) ¹⁷.
     * **signature_delta**: Special event for thinking content, sent before content_block_stop to verify thinking block integrity ¹⁷.
   * **content_block_stop**: Marks the end of a content block. Contains index ¹⁷.
3. **One or more message_delta events**: Represent top-level changes to the final Message object.
   * The usage field in message_delta is **cumulative** ¹⁷.
   * stop_reason becomes non-null in the final message_delta event when the model stops ³.
4. **message_stop**: Final event in the stream, indicating completion ¹⁷.

Other Event Types ¹⁷:

* **ping events**: May be interspersed throughout the response to keep the connection alive ¹⁷.
* **error events**: May occasionally be sent to indicate issues like overloaded_error ¹⁷. Code should gracefully handle unknown event types.

**Differences from Synchronous Response:**

The overall structure of the accumulated streaming response matches the synchronous Message object. The main difference is the incremental nature of the content and usage fields, delivered through _delta events. stop_reason is null in message_start and then populated in message_delta when the model stops ³.

**Table 7: Anthropic Claude Messages API - Streaming Event Types and Deltas**

| Event Type                 | Description                                                 | Key Fields in Delta                                                                                    | Example Delta Structure                                                                                                                                                                                                                                       |
| :------------------------- | :---------------------------------------------------------- | :----------------------------------------------------------------------------------------------------- | :------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| event: message_start       | Stream start, contains a Message object with empty content. | type, message (id, type, role, model, stop_reason (null), stop_sequence (null), usage (empty content)) | event: message_start\ndata: {"type": "message_start", "message": {"id": "msg_...", "type": "message", "role": "assistant", "model": "claude-3-opus-20240229", "stop_reason": null, "stop_sequence": null, "usage": {"input_tokens": 10, "output_tokens": 0}}} |
| event: content_block_start | Content block start.                                        | type, index, content_block (type, text (empty))                                                        | event: content_block_start\ndata: {"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}                                                                                                                                  |
| event: content_block_delta | Text content delta.                                         | type, index, delta (type, text)                                                                        | event: content_block_delta\ndata: {"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "Hello"}}                                                                                                                               |
| event: content_block_delta | Tool use JSON input delta.                                  | type, index, delta (type, partial_json)                                                                | event: content_block_delta\ndata: {"type": "content_block_delta", "index": 1, "delta": {"type": "input_json_delta", "partial_json": "{\"location\": \"New York\"}"}}                                                                                          |
| event: content_block_delta | Thinking content delta.                                     | type, index, delta (type, thinking)                                                                    | event: content_block_delta\ndata: {"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "Let me break this down..."}}                                                                                                   |
| event: content_block_delta | Thinking content signature delta.                           | type, index, delta (type, signature)                                                                   | event: content_block_delta\ndata: {"type": "content_block_delta", "index": 0, "delta": {"type": "signature_delta", "signature": "EqQBCgIYAhIM..."}}                                                                                                           |
| event: content_block_stop  | Content block end.                                          | type, index                                                                                            | event: content_block_stop\ndata: {"type": "content_block_stop", "index": 0}                                                                                                                                                                                   |
| event: message_delta       | Top-level message changes and cumulative usage.             | type, delta (usage), usage (cumulative)                                                                | event: message_delta\ndata: {"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 50}}                                                                                                                                   |
| event: message_stop        | Stream end.                                                 | type                                                                                                   | event: message_stop\ndata: {"type": "message_stop"}                                                                                                                                                                                                           |
| event: ping                | Keep connection alive.                                      | type                                                                                                   | event: ping\ndata: {"type": "ping"}                                                                                                                                                                                                                           |
| event: error               | Error occurred in stream.                                   | type, error (type, message)                                                                            | event: error\ndata: {"type": "error", "error": {"type": "overloaded_error", "message": "The server is currently overloaded."}}                                                                                                                                |

Anthropic's API provides fine-grained visibility into the model's internal processes, such as the thinking parameter in requests ³ and corresponding thinking content blocks in responses ³. Additionally, detailed cache_creation and cache_read_input_tokens in the usage field ³ provide deep caching usage metrics. This level of detail goes beyond typical large language model APIs. This design provides an unprecedented window for developers to observe the model's internal reasoning process. This is extremely valuable for debugging, auditing, or developing more complex AI systems (e.g., self-correcting agents). Detailed caching metrics (cache_creation_input_tokens, cache_read_input_tokens, ephemeral_1h_input_tokens, ephemeral_5m_input_tokens) provide fine-grained insights into token usage and potential cost optimizations related to prompt caching. This indicates Anthropic aims to provide developers with deeper control and understanding of model behavior and resource consumption, facilitating the development of more efficient and interpretable AI applications.## **4. Goo
gle Gemini API (models.generateContent)**

This section will detail the request and response structure of Google Gemini API's generateContent method. It's worth noting that despite initial fragments showing "unavailable" information, detailed information about Gemini API parameters and response structure was obtained through in-depth research of documentation from ai.google.dev/api/generate-content and cloud.google.com/vertex-ai/generative-ai/docs/reference/rest/v1/GenerateContentResponse ⁴.

### **4.1. Request Parameters: Complete Specification**

Gemini API uses the models.generateContent method for chat completion ⁴.

* **Endpoint:** POST https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent ⁴. Vertex AI may use different endpoints ⁸.
* **Authentication:** Gemini Developer API uses x-goog-api-key request header ⁴, Vertex AI uses Google Cloud authentication ⁵.

**Core Conversation Fields:**

* contents (array of objects - Content, **required**): Current conversation content with the model ⁷.
  * **Constraints:** For single-turn queries, this is a single instance. For multi-turn chat, this is a repeated field containing conversation history and the latest request.
  * **Each Content object:**
    * role (string, required): The role of the author of this content. Typically user or model (for assistant responses).
    * parts (array of objects - Part, required): List of content parts.
      * **type: "text":** text (string, required) - Plain text ⁴.
      * **type: "inline_data":** mime_type (string, required), data (string, required, Base64-encoded) - For images, audio, etc. ¹⁹.
      * **type: "function_call":** name (string, required), args (object, required) - Model-generated function call ²⁰.
      * **type: "function_response":** name (string, required), response (object, required) - Output of function call.
      * **type: "file_data":** file_uri (string, required), mime_type (string, required) - Reference to uploaded file ²⁰.
      * **type: "video_metadata":** video_uri (string, required), start_offset_millis (integer, optional), end_offset_millis (integer, optional) - Video content.
      * **type: "uri":** uri (string, required), mime_type (string, required) - Generic URI content ²⁰.
* model (string, implicit in endpoint path, **required**): The model ID to use (e.g., gemini-2.5-flash, gemini-1.5-pro) ⁴.

**Configuration Parameters (generationConfig):** ⁶

* temperature (number, optional): Controls randomness.
  * **Default:** 0.9.
  * **Constraints:** Range 0.0 to 1.0 ⁶.
* top_p (number, optional): Nucleus sampling.
  * **Constraints:** Range 0.0 to 1.0.
* top_k (integer, optional): Only sample from the top K options.
* stop_sequences (array of strings, optional): Custom sequences to stop generation ⁶.
* response_mime_type (string, optional): Expected response MIME type.
  * **Constraints:** application/json for JSON mode, text/x-enum for enum output ²¹.
  * **Note:** Also need to instruct the model to generate JSON in the prompt ²¹.
* response_schema (object, optional): JSON schema for enforcing structured output ²¹.
  * **Supported fields:** enum, items, maxItems, nullable, properties, required ²¹.
* thinkingConfig (object, optional): Configuration for enabling Claude's extended thinking.
  * **thinkingBudget** (integer, optional): Determines the number of tokens Claude can use for internal reasoning process. 0 disables thinking ⁶.

**Safety and Context Parameters:**

* safetySettings (array of objects - SafetySetting, optional): List of unique SafetySetting instances for blocking unsafe content ⁷.
  * **Constraints:** Enforced on request contents and response candidates. At most one setting per SafetyCategory. Overrides defaults.
  * **category:** (enum<string>, required) e.g., HARM_CATEGORY_HATE_SPEECH, HARM_CATEGORY_SEXUALLY_EXPLICIT ⁷.
  * **threshold:** (enum<string>, required) e.g., BLOCK_NONE, BLOCK_LOW_AND_ABOVE.
* systemInstruction (object - Content, optional): Allows developers to set system instructions, currently limited to text ⁶.
  * **Constraints:** Must be a Content object with parts containing text.

**Advanced Features:**

* tools (array of objects - Tool, optional): List of Tools the model may use.
  * **Supported types:** Function, codeExecution ⁷.
* toolConfig (object - ToolConfig, optional): Tool configuration provided for specified tools ⁷.
* cachedContent (string, optional): Context cache content name for prediction. Format: cachedContents/{cachedContent} ⁷.

**Table 8: Google Gemini API - Request Parameters**

| Parameter Name                      | Type                             | Required/Optional | Description                                            | Constraints/Default                                                                                                                                 | Example Value                                                                      |
| :---------------------------------- | :------------------------------- | :---------------- | :----------------------------------------------------- | :-------------------------------------------------------------------------------------------------------------------------------------------------- | :--------------------------------------------------------------------------------- |
| contents                            | array of objects (Content)       | Required          | Current conversation content with the model.           | Each Content object contains role and parts. Parts can include text, inline_data, function_call, function_response, file_data, video_metadata, uri. | [{"role": "user", "parts": [{"text": "Hello"}]}]                                   |
| model                               | string                           | Required          | The model ID to use (in URL path).                     |                                                                                                                                                     | "gemini-1.5-pro"                                                                   |
| generationConfig                    | object (GenerationConfig)        | Optional          | Configuration options for model generation and output. |                                                                                                                                                     | {"temperature": 0.5}                                                               |
| generationConfig.temperature        | number                           | Optional          | Controls randomness.                                   | Range: 0.0 to 1.0. Default: 0.9.                                                                                                                    | 0.7                                                                                |
| generationConfig.top_p              | number                           | Optional          | Nucleus sampling.                                      | Range: 0.0 to 1.0.                                                                                                                                  | 0.9                                                                                |
| generationConfig.top_k              | integer                          | Optional          | Only sample from the top K options.                    |                                                                                                                                                     | 40                                                                                 |
| generationConfig.stop_sequences     | array of strings                 | Optional          | Custom sequences to stop generation.                   |                                                                                                                                                     | ["\nUser:", "###"]                                                                 |
| generationConfig.response_mime_type | string                           | Optional          | Expected response MIME type.                           | application/json or text/x-enum.                                                                                                                    | "application/json"                                                                 |
| generationConfig.response_schema    | object                           | Optional          | JSON schema for enforcing structured output.           | Supports enum, items, maxItems, nullable, properties, required.                                                                                     | {"type": "object", "properties": {"name": {"type": "string"}}}                     |
| generationConfig.thinkingConfig     | object                           | Optional          | Configuration for enabling extended thinking.          | Contains thinkingBudget (integer). 0 disables.                                                                                                      | {"thinkingBudget": 1024}                                                           |
| safetySettings                      | array of objects (SafetySetting) | Optional          | List of settings for blocking unsafe content.          | Each setting contains category and threshold.                                                                                                       | [{"category": "HARM_CATEGORY_HATE_SPEECH", "threshold": "BLOCK_MEDIUM_AND_ABOVE"}] |
| systemInstruction                   | object (Content)                 | Optional          | System instructions.                                   | Must be a Content object with parts containing text.                                                                                                | {"parts": [{"text": "You are a helpful assistant."}]}                              |
| tools                               | array of objects (Tool)          | Optional          | List of tools the model may use.                       | Supports Function and codeExecution types.                                                                                                          | [{"function_declarations": [{"name": "get_time", "parameters": {}}]}]              |
| toolConfig                          | object (ToolConfig)              | Optional          | Tool configuration provided for specified tools.       |                                                                                                                                                     |                                                                                    |
| cachedContent                       | string                           | Optional          | Context cache content name for prediction.             | Format: cachedContents/{cachedContent}.                                                                                                             | "cachedContents/my-cache"                                                          |

Gemini's contents parameter is an array of Content objects, where each Content object can contain multiple Part types (text, inline_data for images/audio, file_data for uploaded files, video_metadata, uri) ⁷. This highly flexible and integrated multimodal input approach indicates that multimodal reasoning is fundamental to Gemini's architecture, not an add-on feature. It simplifies the developer experience for creating applications that blend different modalities in conversation, as all inputs are handled within a unified contents structure. This contrasts with APIs that might require separate endpoints or more complex encoding for multimodal inputs, highlighting Gemini's advantage in natively handling various data types.

Additionally, Gemini's emphasis on structured output and caching is noteworthy. It explicitly supports structured output through response_mime_type and response_schema in generationConfig ²¹. The cachedContent parameter ⁷ and related cachedContentTokenCount in UsageMetadata ²² point to its advanced caching capabilities. Structured output functionality (often called "JSON mode" or "controlled generation") is crucial for integrating LLMs into automated workflows and downstream systems that expect predictable data formats. This reduces the need for post-processing and parsing, improving reliability and efficiency. The caching mechanism, while its full scope isn't detailed, suggests optimization for repetitive or similar prompts, potentially reducing latency and costs for certain use cases. This indicates Google's focus on robust, scalable, and cost-effective enterprise solutions.

### **4.2. Synchronous Response Format (GenerateContentResponse)**

The synchronous response is a GenerateContentResponse object ⁷.

Root Object Structure ²²:

* candidates (array of objects - Candidate, read-only, required): Generated candidate responses ²².
* modelVersion (string, read-only, required): The model version used to generate the response ²².
* createTime (string, timestamp format, read-only, required): Timestamp when the request was sent to the server ²².
* responseId (string, read-only, required): ID used to identify each response ²².
* promptFeedback (object - PromptFeedback, read-only, required): Content filtering results for the prompt. Only sent in the first stream chunk and only appears when no candidates are generated due to content violations ²².
* usageMetadata (object - UsageMetadata, required): Usage metadata about the response ²².

Detailed Nested Objects ²²:

* **Candidate**: A response candidate generated by the model.
  * index (integer, read-only): Index of the candidate.
  * content (object - Content, read-only): Content parts of the candidate.
  * avgLogprobs (number, read-only): Average log probability score of the candidate.
  * logprobsResult (object - LogprobsResult, read-only): Log-likelihood scores for response tokens.
  * finishReason (enum - FinishReason, read-only): Reason the model stopped generating tokens. If empty, the model is still generating.
  * safetyRatings (array of objects - SafetyRating, read-only): List of safety ratings for the response candidate. At most one rating per category.
  * citationMetadata (object - CitationMetadata, read-only): Source attribution for generated content.
  * groundingMetadata (object - GroundingMetadata, read-only): Metadata specifying sources used for content grounding.
  * urlContextMetadata (object - UrlContextMetadata, read-only): Metadata related to URL context retrieval tool.
  * finishMessage (string, read-only): More detailed description of the stop reason (only populated when finishReason is set).
* **LogprobsResult**: Log probability result.
  * topCandidates (array of objects - TopCandidates): Candidates with the highest log probabilities at each decoding step.
  * chosenCandidates (array of objects - Candidate): Chosen candidates.
* **Candidate (nested in LogprobsResult and TopCandidates)**: Candidate for log probability token and score.
  * token (string), tokenId (integer), logProbability (number).
* **FinishReason (enum)**: FINISH_REASON_UNSPECIFIED, STOP, MAX_TOKENS, SAFETY, RECITATION, OTHER, BLOCKLIST, PROHIBITED_CONTENT, SPII, MALFORMED_FUNCTION_CALL, IMAGE_SAFETY, IMAGE_PROHIBITED_CONTENT, IMAGE_RECITATION, IMAGE_OTHER, UNEXPECTED_TOOL_CALL.
* **SafetyRating**: Safety rating corresponding to generated content.
  * category (enum - HarmCategory), probability (enum - HarmProbability), probabilityScore (number), severity (enum - HarmSeverity), severityScore (number), blocked (boolean), overwrittenThreshold (enum - HarmBlockThreshold).
* **HarmProbability (enum)**: HARM_PROBABILITY_UNSPECIFIED, NEGLIGIBLE, LOW, MEDIUM, HIGH.
* **HarmSeverity (enum)**: HARM_SEVERITY_UNSPECIFIED, HARM_SEVERITY_NEGLIGIBLE, HARM_SEVERITY_LOW, HARM_SEVERITY_MEDIUM, HARM_SEVERITY_HIGH.
* **CitationMetadata**: Collection of source attributions for content.
  * citations (array of objects - Citation).
* **Citation**: Source attribution for content.
  * startIndex (integer), endIndex (integer), uri (string), title (string), license (string), publicationDate (object - Date).
* **GroundingMetadata**: Metadata returned to client when grounding is enabled.
  * webSearchQueries (string), groundingChunks (object - GroundingChunk), groundingSupports (object - GroundingSupport), searchEntryPoint (object - SearchEntryPoint), retrievalMetadata (object - RetrievalMetadata), googleMapsWidgetContextToken (string).
* **SearchEntryPoint**: Google Search entry point.
  * renderedContent (string), sdkBlob (string, bytes format).
* **GroundingChunk**: Grounding chunk type (union type: web, retrievedContext, maps).
* **Web**: Chunk from the web (uri, title, domain).
* **RetrievedContext**: Context chunk retrieved from retrieval tool (context_details (union type: ragChunk), uri, title, text).
* **Maps**: Chunk from Google Maps (uri, title, text, placeId).
* **GroundingSupport**: Grounding support.
  * groundingChunkIndices (integer), confidenceScores (number), segment (object - Segment).
* **Segment**: Segment of content (partIndex, startIndex, endIndex, text).
* **RetrievalMetadata**: Metadata related to retrieval in the grounding flow (googleSearchDynamicRetrievalScore).
* **UrlContextMetadata**: Metadata related to URL context retrieval tool.
  * urlMetadata (object - UrlMetadata).
* **UrlMetadata**: Context for a single URL retrieval.
  * retrievedUrl (string), urlRetrievalStatus (enum - UrlRetrievalStatus).
* **UrlRetrievalStatus (enum)**: URL_RETRIEVAL_STATUS_UNSPECIFIED, URL_RETRIEVAL_STATUS_SUCCESS, URL_RETRIEVAL_STATUS_ERROR.
* **PromptFeedback**: Content filtering results for the prompt.
  * blockReason (enum - BlockedReason), safetyRatings (object - SafetyRating), blockReasonMessage (string).
* **BlockedReason (enum)**: BLOCKED_REASON_UNSPECIFIED, SAFETY, OTHER, BLOCKLIST, PROHIBITED_CONTENT, IMAGE_SAFETY.
* **UsageMetadata**: Usage metadata about the response.
  * promptTokenCount (integer), candidatesTokenCount (integer), toolUsePromptTokenCount (integer), thoughtsTokenCount (integer), totalTokenCount (integer), cachedContentTokenCount (integer), promptTokensDetails (object - ModalityTokenCount), cacheTokensDetails (object - ModalityTokenCount), candidatesTokensDetails (object - ModalityTokenCount), toolUsePromptTokensDetails (object - ModalityTokenCount), trafficType (enum - TrafficType).
* **ModalityTokenCount**: Token count information for a single modality.
  * modality (enum - Modality), tokenCount (integer).
* **Modality (enum)**: MODALITY_UNSPECIFIED, TEXT, IMAGE, VIDEO, AUDIO, DOCUMENT.
* **TrafficType (enum)**: TRAFFIC_TYPE_UNSPECIFIED, ON_DEMAND, PROVISIONED_THROUGHPUT.

**Table 9: Google Gemini API - Synchronous Response Fields**

| Field Path                         | Type                            | Description                                             | Possible Values/Structure                                                                       |
| :--------------------------------- | :------------------------------ | :------------------------------------------------------ | :---------------------------------------------------------------------------------------------- |
| candidates                         | array of objects (Candidate)    | Generated candidate responses.                          |                                                                                                 |
| modelVersion                       | string                          | The model version used to generate the response.        |                                                                                                 |
| createTime                         | string (Timestamp)              | Timestamp when the request was sent to the server.      | RFC 3339 format.                                                                                |
| responseId                         | string                          | ID used to identify each response.                      |                                                                                                 |
| promptFeedback                     | object (PromptFeedback)         | Content filtering results for the prompt.               | Only sent in the first stream chunk, only appears when no candidates due to content violations. |
| usageMetadata                      | object (UsageMetadata)          | Usage metadata about the response.                      |                                                                                                 |
| candidates.index                   | integer                         | Index of the candidate.                                 |                                                                                                 |
| candidates.content                 | object (Content)                | Content parts of the candidate.                         |                                                                                                 |
| candidates.avgLogprobs             | number                          | Average log probability score of the candidate.         |                                                                                                 |
| candidates.logprobsResult          | object (LogprobsResult)         | Log-likelihood scores for response tokens.              |                                                                                                 |
| candidates.finishReason            | enum (FinishReason)             | Reason the model stopped generating tokens.             | STOP, MAX_TOKENS, SAFETY, etc.                                                                  |
| candidates.safetyRatings           | array of objects (SafetyRating) | List of safety ratings for the response candidate.      | At most one rating per category.                                                                |
| candidates.citationMetadata        | object (CitationMetadata)       | Source attribution for generated content.               |                                                                                                 |
| candidates.groundingMetadata       | object (GroundingMetadata)      | Metadata specifying sources used for content grounding. |                                                                                                 |
| candidates.urlContextMetadata      | object (UrlContextMetadata)     | Metadata related to URL context retrieval tool.         |                                                                                                 |
| candidates.finishMessage           | string                          | More detailed description of the stop reason.           | Only populated when finishReason is set.                                                        |
| usageMetadata.promptTokenCount     | integer                         | Number of tokens in the request.                        |                                                                                                 |
| usageMetadata.candidatesTokenCount | integer                         | Number of tokens in the response.                       |                                                                                                 |
| usageMetadata.totalTokenCount      | integer                         | Total number of tokens.                                 |                                                                                                 |
| promptFeedback.blockReason         | enum (BlockedReason)            | Reason for blocking.                                    | SAFETY, OTHER, BLOCKLIST, etc.                                                                  |
| promptFeedback.safetyRatings       | array of objects (SafetyRating) | Safety ratings.                                         |                                                                                                 |

Google Gemini API demonstrates comprehensiveness in safety and grounding mechanisms. The safetySettings ⁷ and promptFeedback ²² fields, along with detailed SafetyRating, HarmProbability, and HarmSeverity enums ²², all indicate deeply integrated safety features. Additionally, extensive groundingMetadata containing webSearchQueries, groundingChunks (web, retrieved context, maps), and citationMetadata ²² provides rich context about information retrieval. This design indicates Google's firm commitment to responsible AI development and interpretability. Detailed safety feedback enables developers to understand *why* content was blocked, improving prompt engineering and user experience. Grounding information is crucial for building factually accurate and verifiable AI applications, helping address hallucination issues. This transparency and control over safety and factual accuracy is a significant advantage of Gemini, indicating its focus on enterprise and production-grade applications where reliability and trust are paramount.

### **4.3. Streaming Response Format (models.streamGenerateContent)**

Gemini API supports streaming responses through the models.streamGenerateContent method ⁵. Unlike OpenAI and Anthropic's explicit SSE event types, Gemini's streaming appears to deliver incremental GenerateContentResponse objects ⁵.

**Method Overview and Incremental Response Delivery:**

The streamGenerateContent method produces data chunks as they are generated ⁵. Each data chunk is a GenerateContentResponse object, but it only contains the *new* or *updated* parts of the response. For text, this means content.parts.text will contain incremental string fragments ⁵. These fragments need to be concatenated to form complete text. Other fields like safetyRatings, citationMetadata, usageMetadata may appear in the first data chunk or be updated in subsequent chunks, depending on when that information becomes available. The overall structure of the accumulated streaming response will match the synchronous GenerateContentResponse object.

**Differences from Synchronous Response:**

The core object (GenerateContentResponse) remains the same, but its fields are incrementally populated across multiple data chunks. Fields like promptFeedback are explicitly stated to be "only sent in the first stream chunk" ²². FinishReason in Candidate objects will be empty until the model stops generating tokens for that candidate ²². The content field in Candidate will contain partial text or tool outputs that need to be aggregated.

**Table 10: Google Gemini API - Streaming Response Fields (Incremental)**

| Field Path                    | Type                            | Description                                             | How it appears in stream (e.g., complete object, partial update, only in first chunk, cumulative)                   |
| :---------------------------- | :------------------------------ | :------------------------------------------------------ | :------------------------------------------------------------------------------------------------------------------ |
| candidates                    | array of objects (Candidate)    | Generated candidate responses.                          | Incremental updates, each chunk may contain new Candidates or partial updates to existing Candidates.               |
| candidates.content.parts.text | string                          | Text content of the candidate.                          | Text fragments, need to be concatenated by client to form complete text.                                            |
| candidates.finishReason       | enum (FinishReason)             | Reason the model stopped generating tokens.             | Populated in the final update for the corresponding Candidate when generation stops. Null or undefined before that. |
| promptFeedback                | object (PromptFeedback)         | Content filtering results for the prompt.               | Only sent in the first stream chunk and only appears when no candidates are generated due to content violations.    |
| usageMetadata                 | object (UsageMetadata)          | Usage metadata about the response.                      | May appear in the first chunk or be updated in subsequent chunks (e.g., candidatesTokenCount will accumulate).      |
| modelVersion                  | string                          | The model version used to generate the response.        | Usually appears in the first chunk.                                                                                 |
| createTime                    | string (Timestamp)              | Timestamp when the request was sent to the server.      | Usually appears in the first chunk.                                                                                 |
| responseId                    | string                          | ID used to identify each response.                      | Usually appears in the first chunk.                                                                                 |
| candidates.safetyRatings      | array of objects (SafetyRating) | List of safety ratings for the response candidate.      | May appear in the first chunk or be updated in subsequent chunks.                                                   |
| candidates.citationMetadata   | object (CitationMetadata)       | Source attribution for generated content.               | May appear in the first chunk or be updated in subsequent chunks.                                                   |
| candidates.groundingMetadata  | object (GroundingMetadata)      | Metadata specifying sources used for content grounding. | May appear in the first chunk or be updated in subsequent chunks.                                                   |
| candidates.urlContextMetadata | object (UrlContextMetadata)     | Metadata related to URL context retrieval tool.         | May appear in the first chunk or be updated in subsequent chunks.                                                   | # |
# **5. Comparative Analysis of Chat Completion APIs**

This section directly compares the three major APIs from OpenAI, Anthropic, and Google Gemini based on the detailed specifications outlined above.

### **Comparison of Request Parameter Semantics and Naming Conventions**

* **Message Structure:** All APIs use messages or contents arrays to represent conversation history. OpenAI and Anthropic use role (system, user, assistant), while Gemini uses role (user, model). OpenAI and Anthropic use content field for message text, while Gemini uses parts array within content.
* **System Prompts:** Anthropic has a separate system parameter. OpenAI integrates system role messages into the messages array. Gemini uses systemInstruction as a top-level Content object.
* **Temperature/Top-P/Top-K:** All APIs provide similar parameters, but ranges and defaults may vary slightly.
* **Stop Sequences:** All APIs support custom stop sequences.
* **Tools/Function Calling:** All APIs support this, but with different implementations:
  * **OpenAI:** tools array containing function type, with tool_choice parameter. Responses contain tool_calls.
  * **Anthropic:** tools array containing name, description, input_schema. Responses contain tool_use content blocks. Supports disable_parallel_tool_use.
  * **Gemini:** tools array containing Function or codeExecution, with toolConfig. Responses contain function_call parts.
* **Multimodal:**
  * **OpenAI:** gpt-4o models support images in content arrays.
  * **Anthropic:** Supports base64 source type images in content arrays.
  * **Gemini:** Most comprehensive, supporting images, audio, video, and files through inline_data, file_data, video_metadata, uri in parts arrays.

### **Differences in Synchronous Response Structure and Information Provided**

* **Root Object:** OpenAI uses chat.completion object, Anthropic uses message object, Gemini uses GenerateContentResponse.
* **Content Representation:** OpenAI and Anthropic use content strings (or Anthropic's content block arrays). Gemini uses candidates.content.parts for generated content.
* **Usage Metrics:** All APIs provide token counts (prompt_tokens, completion_tokens/input_tokens, output_tokens). Anthropic provides highly detailed caching and server tool usage. Gemini provides promptTokenCount, candidatesTokenCount, totalTokenCount plus multimodal token details through UsageMetadata.
* **Stop Reasons:** All APIs provide finish_reason or stop_reason with similar values.
* **Safety/Content Moderation:** Gemini and Anthropic explicitly provide safetyRatings/promptFeedback in responses, detailing blocked content and harm probabilities. OpenAI indicates content filtering through finish_reason: content_filter.
* **Grounding/Citations:** Gemini provides extensive citationMetadata and groundingMetadata (web search, retrieved context, maps), offering rich source information. Anthropic and OpenAI are less explicit about grounding information in standard chat completion responses.

### **Analysis of Streaming Protocols (SSE vs Incremental JSON) and Their Implications**

* **OpenAI and Anthropic:** Both use Server-Sent Events (SSE) with explicit event types (like message_start, content_block_delta, message_delta, message_stop, etc.). This explicit event-driven approach allows clear parsing of different stages of the response generation process.
* **Gemini:** Streams incremental GenerateContentResponse objects. While simpler in terms of event types, it requires clients to reconstruct complete responses by accumulating partial JSON objects, which may be more complex than concatenating simple text deltas.
* **Implications:** SSE with explicit event types (OpenAI, Anthropic) provides clearer state transitions and simpler parsing for specific content blocks (e.g., Anthropic's thinking_delta). Gemini's approach may be easier to implement at a high level (just accumulate JSON), but requires more careful handling of partial objects if you need to react to specific field updates in the stream.

**Table 11: Cross-API Feature Comparison Matrix**

| Feature                       | OpenAI                                                                         | Claude                                                                                                                  | Gemini                                                                                                   |
| :---------------------------- | :----------------------------------------------------------------------------- | :---------------------------------------------------------------------------------------------------------------------- | :------------------------------------------------------------------------------------------------------- |
| **Multimodal Support**        | gpt-4o supports images in messages.content.                                    | Supports base64 images in messages.content.                                                                             | contents.parts supports text, images, audio, video, files, URI, most comprehensive design.               |
| **Tool Calling Mechanism**    | tools array (function type), tool_choice parameter. Responses have tool_calls. | tools array (name, description, input_schema), tool_choice parameter. Responses have tool_use content blocks.           | tools array (Function, codeExecution), toolConfig. Responses have function_call parts.                   |
| **System Prompt Handling**    | system role messages in messages array.                                        | Separate system parameter.                                                                                              | Separate systemInstruction object.                                                                       |
| **Streaming Protocol**        | SSE with explicit event types (data: {...}, data: [DONE]).                     | SSE with detailed event types (message_start, content_block_delta, etc.).                                               | Incremental GenerateContentResponse objects.                                                             |
| **Max Input Tokens**          | Depends on model context length.                                               | Depends on model context length.                                                                                        | Depends on model context length.                                                                         |
| **Temperature Range**         | 0 to 2.                                                                        | 0.0 to 1.0.                                                                                                             | 0.0 to 1.0.                                                                                              |
| **Safety Features**           | finish_reason: content_filter.                                                 | Explicit safetyRatings and promptFeedback in responses.                                                                 | Explicit safetySettings and promptFeedback in responses, detailed SafetyRating, HarmProbability.         |
| **Structured Output Support** | response_format parameter (json_object type).                                  | Not explicit, but achievable through prompt engineering.                                                                | generationConfig.response_mime_type and response_schema.                                                 |
| **Caching Features**          | stream_options.include_usage for final usage statistics.                       | Detailed cache_creation and cache_read_input_tokens in usage.                                                           | cachedContent parameter and usageMetadata.cachedContentTokenCount.                                       |
| **Detailed Usage Metrics**    | usage contains prompt_tokens, completion_tokens, total_tokens.                 | usage contains input_tokens, output_tokens, cache_creation_input_tokens, cache_read_input_tokens, server_tool_use, etc. | usageMetadata contains promptTokenCount, candidatesTokenCount, totalTokenCount, modalityTokenCount, etc. |
| **Grounding/Citations**       | Less explicit details.                                                         | Less explicit details.                                                                                                  | Extensive citationMetadata and groundingMetadata (web search, retrieved context, maps).                  |

By comparing usage fields and streaming events, differences in API granularity and transparency can be observed. OpenAI provides stream_options.include_usage for final usage statistics ¹⁶. Anthropic provides highly granular usage metrics including caching and server tool usage ³, and detailed thinking_delta events during streaming ¹⁷. Gemini provides modality-specific token counts and detailed safety blocking information through its UsageMetadata and promptFeedback ²². These differences indicate different philosophies regarding API transparency and developer control. Anthropic seems to prioritize deep inspection of model behavior (e.g., "thinking" processes) and cost attribution (detailed caching). Gemini is very focused on safety and grounding, providing fine-grained feedback on content moderation and factual sources. OpenAI provides basic usage while leaning toward a more streamlined output for general purposes. This means that for applications requiring high interpretability, fine-grained cost analysis, or robust safety/grounding features, Anthropic and Gemini may provide more out-of-the-box functionality, while OpenAI offers a more general-purpose, performance-oriented interface.

The evolution of multimodal and tool paradigms is also worth noting. Although all three support multimodal and tool calling, their API structures reflect different evolutionary stages or design priorities. Gemini's contents with parts structure ⁷ feels most natively multimodal, allowing diverse inputs within a single message. Anthropic's content arrays with specific image and tool_use blocks ³ are also well integrated. OpenAI's multimodal support for gpt-4o is newer and integrated into the existing content field. Similarly, tool calling mechanisms have different input/output structures. This indicates that LLM capabilities are rapidly expanding beyond pure text. Developers building multimodal applications may find Gemini's unified parts structure more intuitive for complex mixed-media inputs. The differences in tool calling mechanisms mean that abstracting agent workflows across these APIs requires a robust middleware layer to normalize tool definitions and calls. The trend is toward more powerful, more agent-like models, but integration patterns are still converging, creating both opportunities and challenges for cross-platform development.

Finally, examining the "OpenAI-compatible" ecosystem reveals some nuances. The user query explicitly mentioned "OpenAI-compatible format." APIs like OpenVINO ¹⁶, Langdock ¹³, and OpenRouter ¹² aim for OpenAI compatibility. However, these compatible APIs often have subtle deviations (e.g., Langdock doesn't support n or stream_options, OpenVINO's ignore_eos or include_stop_str_in_output aren't in the official OpenAI specification) ¹³. This means that while compatibility lowers the barrier to entry for developers familiar with OpenAI's API, it's crucial to understand that "compatible" doesn't equal "identical." Developers must carefully consult the specific documentation for *each* compatible provider, as subtle differences in supported parameters, defaults, or response structures can lead to unexpected behavior or limit feature utilization. This leads to a fragmented ecosystem where truly "universal" OpenAI-compatible clients may require conditional logic or feature subsets.

## **6. Conclusions and Recommendations**

This report has conducted an in-depth analysis of the chat completion APIs from OpenAI, Anthropic, and Google Gemini, revealing their similarities and differences in request input, synchronous responses, and streaming response formats. Each provider offers a unique feature set and API paradigm based on their core design philosophy and priorities.

**Summary of Strengths and Weaknesses of Each API:**

* **OpenAI:**
  * **Strengths:** Wide model selection, strong community support, mature tool calling functionality, relatively simple for basic text streaming.
  * **Weaknesses:** Less explicit safety feedback compared to Gemini/Claude.
* **Anthropic (Claude):**
  * **Strengths:** Explicit conversation structure, advanced safety features, detailed visibility into model thinking processes, and comprehensive token usage and caching metrics.
  * **Weaknesses:** More strict enforcement of messages role requirements.
* **Google Gemini:**
  * **Strengths:** Strong native multimodal input processing capabilities, extensive grounding/citation features, fine-grained safety controls, and structured output capabilities.
  * **Weaknesses:** Streaming event types are less explicit than OpenAI and Anthropic, potentially requiring more complex client parsing logic.

**Recommendations for Choosing APIs Based on Specific Integration Needs:**

* For **general chat applications** prioritizing ease of integration and wide model selection: Recommend **OpenAI**.
* For **applications requiring high controllability, interpretability (thinking processes), and detailed cost analysis**: Recommend **Anthropic**.
* For **multimodal applications, or those needing robust factual grounding and structured output for downstream processing**: Recommend **Google Gemini**.
* For **agent applications**, consider each API's specific tool implementation details and their fit with orchestration layers.
* For **streaming implementations**, note the different paradigms: OpenAI and Anthropic use explicit SSE event types, while Gemini uses incremental object reconstruction.

The final choice should be based on project-specific needs, team familiarity, and emphasis on particular features (such as multimodal, tool calling, safety controls, or cost transparency).

#### **Referenced Works**

1. Azure OpenAI in Azure AI Foundry Models REST API reference - Learn Microsoft, accessed July 27, 2025, [https://learn.microsoft.com/en-us/azure/ai-foundry/openai/reference](https://learn.microsoft.com/en-us/azure/ai-foundry/openai/reference)
2. Claude API | Documentation | Postman API Network, accessed July 27, 2025, [https://www.postman.com/postman/anthropic-apis/documentation/dhus72s/claude-api](https://www.postman.com/postman/anthropic-apis/documentation/dhus72s/claude-api)
3. Messages - Anthropic, accessed July 27, 2025, [https://docs.anthropic.com/en/api/messages](https://docs.anthropic.com/en/api/messages)
4. Gemini API | Google AI for Developers, accessed July 27, 2025, [https://ai.google.dev/gemini-api/docs](https://ai.google.dev/gemini-api/docs)
5. @google/genai - The GitHub pages site for the googleapis organization., accessed July 27, 2025, [https://googleapis.github.io/js-genai/](https://googleapis.github.io/js-genai/)
6. Text generation | Gemini API | Google AI for Developers, accessed July 27, 2025, [https://ai.google.dev/gemini-api/docs/text-generation](https://ai.google.dev/gemini-api/docs/text-generation)
7. Generating content | Gemini API | Google AI for Developers, accessed July 27, 2025, [https://ai.google.dev/api/generate-content](https://ai.google.dev/api/generate-content)
8. Vertex AI GenAI API | Generative AI on Vertex AI - Google Cloud, accessed July 27, 2025, [https://cloud.google.com/vertex-ai/generative-ai/docs/reference/rest](https://cloud.google.com/vertex-ai/generative-ai/docs/reference/rest)
9. All methods | Gemini API | Google AI for Developers, accessed July 27, 2025, [https://ai.google.dev/api/all-methods](https://ai.google.dev/api/all-methods)
10. Openai /v1/completions vs. /v1/chat/completions end points - Stack Overflow, accessed July 27, 2025, [https://stackoverflow.com/questions/76192496/openai-v1-completions-vs-v1-chat-completions-end-points](https://stackoverflow.com/questions/76192496/openai-v1-completions-vs-v1-chat-completions-end-points)
11. How to call the chat completion api from node? - OpenAI - Reddit, accessed July 27, 2025, [https://www.reddit.com/r/OpenAI/comments/11fxfpz/how_to_call_the_chat_completion_api_from_node/](https://www.reddit.com/r/OpenAI/comments/11fxfpz/how_to_call_the_chat_completion_api_from_node/)
12. Chat completion | OpenRouter | Documentation, accessed July 27, 2025, [https://openrouter.ai/docs/api-reference/chat-completion](https://openrouter.ai/docs/api-reference/chat-completion)
13. OpenAI Chat completion - Documentation, accessed July 27, 2025, [https://docs.langdock.com/api-endpoints/completion/openai](https://docs.langdock.com/api-endpoints/completion/openai)
14. REST API Reference - xAI Docs, accessed July 27, 2025, [https://docs.x.ai/docs/api-reference](https://docs.x.ai/docs/api-reference)
15. Model - OpenAI API, accessed July 27, 2025, [https://platform.openai.com/docs/models/gpt-4o](https://platform.openai.com/docs/models/gpt-4o)
16. OpenAI API chat/completions endpoint - OpenVINO™ documentation, accessed July 27, 2025, [https://docs.openvino.ai/2025/model-server/ovms_docs_rest_api_chat.html](https://docs.openvino.ai/2025/model-server/ovms_docs_rest_api_chat.html)
17. Streaming Messages - Anthropic, accessed July 27, 2025, [https://docs.anthropic.com/en/api/messages-streaming](https://docs.anthropic.com/en/api/messages-streaming)
18. Gemini API reference | Google AI for Developers, accessed July 27, 2025, [https://ai.google.dev/api](https://ai.google.dev/api)
19. deprecated-generative-ai-python/docs/api/google/generativeai ..., accessed July 27, 2025, [https://github.com/google-gemini/generative-ai-python/blob/main/docs/api/google/generativeai/protos.md](https://github.com/google-gemini/generative-ai-python/blob/main/docs/api/google/generativeai/protos.md)
20. googleapis/python-genai: Google Gen AI Python SDK provides an interface for developers to integrate Google's generative models into their Python applications. - GitHub, accessed July 27, 2025, [https://github.com/googleapis/python-genai](https://github.com/googleapis/python-genai)
21. Generate structured output (like JSON and enums) using the Gemini API | Firebase AI Logic, accessed July 27, 2025, [https://firebase.google.com/docs/ai-logic/generate-structured-output](https://firebase.google.com/docs/ai-logic/generate-structured-output)
22. GenerateContentResponse | Generative AI on Vertex AI | Google ..., accessed July 27, 2025, [https://cloud.google.com/vertex-ai/generative-ai/docs/reference/rest/v1/GenerateContentResponse](https://cloud.google.com/vertex-ai/generative-ai/docs/reference/rest/v1/GenerateContentResponse)
23. Generate streaming text content with Generative Model | Generative AI on Vertex AI | Google Cloud, accessed July 27, 2025, [https://cloud.google.com/vertex-ai/generative-ai/docs/samples/googlegenaisdk-textgen-with-txt-stream](https://cloud.google.com/vertex-ai/generative-ai/docs/samples/googlegenaisdk-textgen-with-txt-stream)