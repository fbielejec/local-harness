#!/usr/bin/env python3
"""Install or update an Open WebUI function from a file, then make it Active and Global.

Usage:  python3 deploy/openwebui/install_function.py <function.py> <id>
  e.g.  python3 deploy/openwebui/install_function.py \
            deploy/openwebui/functions/deepseek_search_filter.py deepseek_search

Name and description come from the file's frontmatter (`title:`, `description:`).
Auth: OPENWEBUI_API_KEY (an admin key) from the environment, else from the `.env` beside this
script. OPENWEBUI_URL overrides the default http://localhost:3000. Standard library only.
"""
import sys
from pathlib import Path

import owui


def call(method: str, path: str, body: dict | None = None) -> tuple[int, dict | None]:
    return owui.call(method, f"/api/v1/functions{path}", body)


def main() -> None:
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    path, function_id = Path(sys.argv[1]), sys.argv[2]
    content = path.read_text()
    form = {
        "id": function_id,
        "name": owui.frontmatter(content, "title") or function_id,
        "content": content,
        "meta": {"description": owui.frontmatter(content, "description"), "manifest": {}},
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
