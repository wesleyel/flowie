import json
import subprocess
import sys
from pathlib import Path

requests = (
    {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}},
    {"jsonrpc": "2.0", "id": 2, "method": "ping", "params": {}},
)
response = subprocess.run(
    [str(Path(sys.argv[1]).resolve()), "mcp"],
    input="".join(json.dumps(request) + "\n" for request in requests),
    text=True,
    capture_output=True,
    check=True,
    timeout=60,
)
print(response.stdout, end="")
replies = {reply["id"]: reply for reply in map(json.loads, response.stdout.splitlines())}
if replies[1]["result"]["serverInfo"]["name"] != "flowie" or replies[2]["result"] != {}:
    raise SystemExit("MCP smoke test failed")
