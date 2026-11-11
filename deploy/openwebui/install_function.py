#!/usr/bin/env python3
"""Install or update an Open WebUI function from a file, then make it Active and Global.

Usage:  python3 deploy/openwebui/install_function.py <function.py> <id>
  e.g.  python3 deploy/openwebui/install_function.py \
            deploy/openwebui/functions/deepseek_search_filter.py deepseek_search

Name and description come from the file's frontmatter (`title:`, `description:`).
Auth: OPENWEBUI_API_KEY (an admin key) from the environment, else from the `.env` beside this
script. OPENWEBUI_URL overrides the default http://localhost:3000. Standard library only.
"""
import json
import os
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path


def api_key() -> str:
    if key := os.environ.get("OPENWEBUI_API_KEY"):
        return key
    env_file = Path(__file__).with_name(".env")
    if env_file.exists():
        for line in env_file.read_text().splitlines():
            if line.startswith("OPENWEBUI_API_KEY="):
                return line.split("=", 1)[1].strip()
    sys.exit("OPENWEBUI_API_KEY not set and not found in deploy/openwebui/.env")


def frontmatter(content: str, field: str) -> str:
    match = re.search(rf"^{field}:\s*(.+)$", content, re.MULTILINE)
    return match.group(1).strip() if match else ""


def call(method: str, path: str, body: dict | None = None) -> tuple[int, dict | None]:
    base = os.environ.get("OPENWEBUI_URL", "http://localhost:3000")
    request = urllib.request.Request(
        f"{base}/api/v1/functions{path}",
        method=method,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"Authorization": f"Bearer {api_key()}", "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return response.status, json.loads(response.read() or "null")
    except urllib.error.HTTPError as error:
        return error.code, json.loads(error.read() or "null")


def main() -> None:
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    path, function_id = Path(sys.argv[1]), sys.argv[2]
    content = path.read_text()
    form = {
        "id": function_id,
        "name": frontmatter(content, "title") or function_id,
        "content": content,
        "meta": {"description": frontmatter(content, "description"), "manifest": {}},
    }

    # A missing function answers 401, not 404.
    exists = call("GET", f"/id/{function_id}")[0] == 200
    action, endpoint = ("update", f"/id/{function_id}/update") if exists else ("create", "/create")
    status, body = call("POST", endpoint, form)
    if status != 200:
        sys.exit(f"{action} failed: HTTP {status} {body}")
    print(f"{action}d {function_id}")

    function = call("GET", f"/id/{function_id}")[1]
    # These endpoints flip the stored value, so call them only when the flag is off.
    for flag, toggle in (("is_active", "toggle"), ("is_global", "toggle/global")):
        if not function[flag]:
            status, function = call("POST", f"/id/{function_id}/{toggle}")
            if status != 200:
                sys.exit(f"{toggle} failed: HTTP {status}")

    print({key: function[key] for key in ("type", "name", "is_active", "is_global")})


if __name__ == "__main__":
    main()
