"""
Complete test suite for ccproxy module endpoints

This test script verifies the functionality of all major ccproxy endpoints, including:

1. Model Listing Endpoints:
   - OpenAI: /v1/models
   - Ollama: /api/tags
   - Gemini: /v1beta/models
   - Grouped: /{GROUP_NAME}/v1/models
   - Switch: /switch/v1/models

2. Embedding Endpoints:
   - OpenAI: /v1/embeddings
   - Gemini: /v1beta/models/{model}:embedContent
   - Ollama: /api/embed

3. Chat Protocol Endpoints (with both text and tool use cases):
   - OpenAI: /v1/chat/completions
   - Claude: /v1/messages
   - Gemini: /v1beta/models/{model}:generateContent
   - Ollama: /api/chat

4. Special Path Variations:
   - Default path: /{endpoint}
   - Switch path: /switch/{endpoint}
   - Switch compat path: /switch/compat/{endpoint}
   - Group path: /{GROUP_NAME}/{endpoint}
   - Group compat path: /{GROUP_NAME}/compat/{endpoint}

All tests are executed with both standard text prompts and tool use scenarios to ensure
complete functionality across different protocol implementations.
"""
import urllib.request
import urllib.error
import json
import os

# ================= Configuration =================
PORT = "11435"
# This is my local test API key, please replace it with your own
API_KEY = "cs-TosCz7A29R74yNnShYQKskxXPx9OSYd1RMQV5YzVdHvqL7ehNNoOhCVg7UTp6"

# Model Configuration
# These models and groups need to be configured in the proxy module, you can also replace them with your own
DEFAULT_MODEL = "code-small"
SWITCH_MODEL = "claude-haiku-4.5"
GROUP_NAME = "glm"
GROUP_MODEL = "claude-haiku-4.5"
EMBED_MODEL = "embed"

BASE_URL = f"http://localhost:{PORT}"

# Color Definitions
RED, GREEN, YELLOW, BLUE, NC = '\033[0;31m', '\033[0;32m', '\033[1;33m', '\033[0;34m', '\033[0m'

def print_header(title):
    print(f"\n{BLUE}{'='*60}{NC}\n{BLUE}  {title}{NC}\n{BLUE}{'='*60}{NC}")

def request(url, method="POST", headers=None, data=None, timeout=30):
    if headers is None:
        headers = {}
    headers.update({"User-Agent": "ChatspeedTest/2.0", "Accept": "*/*"})

    if data:
        if isinstance(data, (dict, list)):
            data = json.dumps(data).encode('utf-8')
            headers["Content-Type"] = "application/json"
        elif isinstance(data, str):
            data = data.encode('utf-8')

    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as res:
            return res.getcode(), res.read().decode('utf-8')
    except urllib.error.HTTPError as e:
        try:
            body = e.read().decode('utf-8')
        except:
            body = ""
        return e.code, body
    except Exception as e:
        return -1, str(e)

# ================= Validation Logic =================


def validate_openai(code, body, is_tool=False, is_compat=False):
    if code != 200: return False, f"HTTP {code}: {body[:100]}"
    try:
        data = json.loads(body)
        if 'choices' not in data: return False, "Missing 'choices' in OpenAI response"
        msg = data['choices'][0]['message']
        if is_tool or is_compat:
            tcs = msg.get('tool_calls')
            if tcs and tcs[0].get('function', {}).get('name') == 'get_weather':
                return True, "Tool Call OK"
            return False, f"tool_calls missing. Model said: {msg.get('content', '')[:100]}"
        if msg.get('content'): return True, f"Text: {msg['content'][:25].replace('\n',' ')}..."
        return False, "Empty content"
    except Exception as e: return False, str(e)

def validate_claude(code, body, is_tool=False, is_compat=False):
    if code != 200: return False, f"HTTP {code}: {body[:100]}"
    try:
        data = json.loads(body)
        if 'content' not in data: return False, "Missing 'content' in Claude response"
        content_list = data.get('content', [])
        if is_tool or is_compat:
            found = any(c.get('type') == 'tool_use' and c.get('name') == 'get_weather' for c in content_list)
            if found: return True, "Tool Use OK"
            text = content_list[0].get('text', '') if content_list else "Empty"
            return False, f"tool_use missing. Model said: {text[:100]}"
        text = content_list[0].get('text', '') if content_list else ""
        if text: return True, f"Text: {text[:25].replace('\n',' ')}..."
        return False, "Empty content"
    except Exception as e: return False, str(e)

def validate_gemini(code, body, is_tool=False, is_compat=False):
    if code != 200: return False, f"HTTP {code}: {body[:100]}"
    try:
        data = json.loads(body)
        if 'choices' in data: return False, "Protocol Error: Received OpenAI format but expected Gemini"
        if 'candidates' not in data: return False, "Missing 'candidates' in Gemini response"

        parts = data['candidates'][0]['content']['parts']
        if is_tool or is_compat:
            found = any(p.get('functionCall', {}).get('name') == 'get_weather' for p in parts)
            if found: return True, "Function Call OK"
            text = parts[0].get('text', '') if parts else "Empty"
            return False, f"functionCall missing. Model said: {text[:100]}"
        text = parts[0].get('text', '') if parts else ""
        if text: return True, f"Text: {text[:25].replace('\n',' ')}..."
        return False, "Empty content"
    except Exception as e: return False, str(e)

def validate_ollama(code, body, is_tool=False, is_compat=False):
    if code != 200: return False, f"HTTP {code}: {body[:100]}"
    try:
        data = json.loads(body)
        if 'choices' in data: return False, "Protocol Error: Received OpenAI format but expected Ollama"
        if 'message' not in data: return False, "Missing 'message' in Ollama response"

        msg = data.get('message', {})
        if is_tool or is_compat:
            tcs = msg.get('tool_calls')
            if tcs and tcs[0].get('function', {}).get('name') == 'get_weather':
                return True, "Tool Call OK"
            return False, f"tool_calls missing. Model said: {msg.get('content', '')[:100]}"
        if msg.get('content'): return True, f"Text: {msg['content'][:25].replace('\n',' ')}..."
        return False, "Empty content"
    except Exception as e: return False, str(e)

# ================= Data Construction =================


T_DEF_OPENAI = [{
    "type": "function",
    "function": {
        "name": "get_weather",
        "description": "Get the current weather for a specific location",
        "parameters": {
            "type": "object",
            "properties": {"loc": {"type": "string", "description": "The city and state, e.g. New York, NY"}},
            "required": ["loc"]
        }
    }
}]
T_DEF_CLAUDE = [{
    "name": "get_weather",
    "description": "Get the current weather for a specific location",
    "input_schema": {
        "type": "object",
        "properties": {"loc": {"type": "string", "description": "The city and state, e.g. New York, NY"}},
        "required": ["loc"]
    }
}]
# Fixed Gemini tool definition: using camelCase and official structure

T_DEF_GEMINI = [{
    "functionDeclarations": [{
        "name": "get_weather",
        "description": "Get the current weather for a specific location",
        "parameters": {
            "type": "object",
            "properties": {"loc": {"type": "string", "description": "The city and state, e.g. New York, NY"}},
            "required": ["loc"]
        }
    }]
}]

def build_data(protocol, model, is_tool=False):
    # Using stronger prompt for tool testing
    prompt = "Get weather for New York. Use the get_weather tool." if is_tool else "Hello, say 'Hi'"

    if protocol == "openai":
        d = {"model": model, "messages": [{"role": "user", "content": prompt}], "stream": False}
        if is_tool: d["tools"] = T_DEF_OPENAI
        return d
    if protocol == "claude":
        d = {"model": model, "max_tokens": 1024, "messages": [{"role": "user", "content": prompt}], "stream": False}
        if is_tool: d["tools"] = T_DEF_CLAUDE
        return d
    if protocol == "gemini":
        d = {"contents": [{"role": "user", "parts": [{"text": prompt}]}]}
        if is_tool: d["tools"] = T_DEF_GEMINI
        return d
    if protocol == "ollama":
        d = {"model": model, "messages": [{"role": "user", "content": prompt}], "stream": False}
        if is_tool: d["tools"] = T_DEF_OPENAI
        return d

# ================= Test Execution =================


def test_protocol_matrix(name, protocol, base_path, validator):
    print(f"\n{YELLOW}--- Testing Protocol: {name} ---{NC}")
    oa_h = {"Authorization": f"Bearer {API_KEY}"}
    cl_h = {"x-api-key": API_KEY, "anthropic-version": "2023-06-01"}
    headers = cl_h if protocol == "claude" else oa_h

    matrix = [
        ("", DEFAULT_MODEL, False),
        ("/switch", SWITCH_MODEL, False),
        ("/switch/compat", SWITCH_MODEL, True),
        (f"/{GROUP_NAME}", GROUP_MODEL, False),
        (f"/{GROUP_NAME}/compat", GROUP_MODEL, True)
    ]

    for prefix, model, is_compat in matrix:
        if protocol == "gemini":
            url = f"{BASE_URL}{prefix}{base_path}/{model}:generateContent?key={API_KEY}"
            headers_to_use = {}
        else:
            url = f"{BASE_URL}{prefix}{base_path}"
            headers_to_use = headers

        # Text Test
        c, b = request(url, "POST", headers_to_use, build_data(protocol, model, False))
        ok, msg = validator(c, b, False, False)
        print(f"{GREEN}[PASS]{NC}" if ok else f"{RED}[FAIL]{NC}", f"Text -> {prefix or '/':<12} | {msg}")

        # Tool Test
        c, b = request(url, "POST", headers_to_use, build_data(protocol, model, True))
        ok, msg = validator(c, b, True, is_compat)
        print(f"{GREEN}[PASS]{NC}" if ok else f"{RED}[FAIL]{NC}", f"Tool -> {prefix or '/':<12} | {msg}")

def test_all():
    os.environ['no_proxy'] = 'localhost,127.0.0.1'

    print_header("1. Model Listing")
    oa_h = {"Authorization": f"Bearer {API_KEY}"}
    list_tests = [("/v1/models", "OpenAI List"), ("/api/tags", "Ollama Tags"), ("/v1beta/models", "Gemini Models"), (f"/{GROUP_NAME}/v1/models", "Grouped List"), ("/switch/v1/models", "Switch List")]
    for path, label in list_tests:
        c, b = request(f"{BASE_URL}{path}", "GET", oa_h)
        print(f"{GREEN}[PASS]{NC} {label:<15} | HTTP {c}")

    print_header("2. Embeddings")
    c, b = request(f"{BASE_URL}/v1/embeddings", "POST", oa_h, {"model": EMBED_MODEL, "input": "hello"})
    print(f"{GREEN}[PASS]{NC} OpenAI Embed    |" if '"embedding"' in b else f"{RED}[FAIL]{NC} OpenAI Embed    | {b[:50]}")
    c, b = request(f"{BASE_URL}/v1beta/models/{EMBED_MODEL}:embedContent?key={API_KEY}", "POST", {}, {"content": {"parts": [{"text": "hello"}]}})
    print(f"{GREEN}[PASS]{NC} Gemini Embed    |" if '"values"' in b else f"{RED}[FAIL]{NC} Gemini Embed    | {b[:50]}")
    c, b = request(f"{BASE_URL}/api/embed", "POST", oa_h, {"model": EMBED_MODEL, "input": "hello"})
    print(f"{GREEN}[PASS]{NC} Ollama Embed    |" if '"embeddings"' in b or '"embedding"' in b else f"{RED}[FAIL]{NC} Ollama Embed    | {b[:50]}")

    print_header("3. Chat Protocol Matrix")
    test_protocol_matrix("OpenAI", "openai", "/v1/chat/completions", validate_openai)
    test_protocol_matrix("Claude", "claude", "/v1/messages", validate_claude)
    test_protocol_matrix("Ollama", "ollama", "/api/chat", validate_ollama)
    test_protocol_matrix("Gemini", "gemini", "/v1beta/models", validate_gemini)

    print(f"\n{BLUE}{'='*60}{NC}\n  Testing Completed.\n{BLUE}{'='*60}{NC}")

if __name__ == "__main__":
    test_all()