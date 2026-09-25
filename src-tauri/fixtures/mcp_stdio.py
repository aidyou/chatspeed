"""Deterministic, network-free MCP stdio fixture for the control-plane lifecycle test."""

import json
import sys


def main():
    for line in sys.stdin:
        request = json.loads(line)
        method = request.get("method")
        if "id" not in request:
            continue
        if method == "initialize":
            result = {
                "protocolVersion": request["params"]["protocolVersion"],
                "capabilities": {"tools": {"listChanged": False}},
                "serverInfo": {"name": "local-fixture", "version": "1.0.0"},
            }
        elif method == "tools/list":
            result = {
                "tools": [
                    {
                        "name": "fixture_echo",
                        "description": "Declared only; not executed by the test",
                        "inputSchema": {"type": "object", "properties": {}},
                    }
                ]
            }
        elif method == "ping":
            result = {}
        else:
            # An accidental tool invocation must fail, not masquerade as a read.
            response = {
                "jsonrpc": "2.0",
                "id": request["id"],
                "error": {"code": -32601, "message": "Method not found"},
            }
            sys.stdout.write(json.dumps(response) + "\n")
            sys.stdout.flush()
            continue
        sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
