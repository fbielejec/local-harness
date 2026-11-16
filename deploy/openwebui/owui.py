"""Shared helpers for the Open WebUI install scripts: settings from the environment or `.env`,
and one JSON call against the admin API. Standard library only."""
import json
import os
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path

ENV_FILE = Path(__file__).with_name(".env")


def setting(name: str) -> str:
    """`name` from the environment, else from the `.env` beside this file. Exits if missing."""
    if value := os.environ.get(name):
        return value
    if ENV_FILE.exists():
        for line in ENV_FILE.read_text().splitlines():
            if line.startswith(f"{name}="):
                return line.split("=", 1)[1].strip()
    sys.exit(f"{name} not set and not found in {ENV_FILE}")


def frontmatter(content: str, field: str) -> str:
    """The value of a `field: value` line in a file's frontmatter docstring, or ""."""
    match = re.search(rf"^{field}:\s*(.+)$", content, re.MULTILINE)
    return match.group(1).strip() if match else ""


def call(method: str, path: str, body: dict | None = None) -> tuple[int, dict | list | None]:
    """`path` is relative to the server root, e.g. `/api/v1/tools/create` or `/openai/config`."""
    base = os.environ.get("OPENWEBUI_URL", "http://localhost:3000")
    request = urllib.request.Request(
        f"{base}{path}",
        method=method,
        data=json.dumps(body).encode() if body is not None else None,
        headers={
            "Authorization": f"Bearer {setting('OPENWEBUI_API_KEY')}",
            "Content-Type": "application/json",
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return response.status, json.loads(response.read() or "null")
    except urllib.error.HTTPError as error:
        return error.code, json.loads(error.read() or "null")
