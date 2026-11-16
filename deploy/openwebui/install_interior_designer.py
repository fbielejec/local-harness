#!/usr/bin/env python3
"""Install or update the Interior Designer in Open WebUI. Safe to re-run.

Usage:  python3 deploy/openwebui/install_interior_designer.py     (or: make install-designer)

Four steps, each create-or-update:
  1. the OpenAI connection, allow-listed to the designer model so the raw model list stays short
  2. the `interior-design` skill   (skills/interior-design.md)
  3. the `render_design` tool      (tools/render_design.py), with OPENAI_API_KEY in its valves
  4. the `interior-designer` model (models/interior-designer.json)
Skill, tool and model get a public-read grant: every account sees them, including later ones.

Needs OPENWEBUI_API_KEY (admin) and OPENAI_API_KEY, from the environment or deploy/openwebui/.env.
Design: docs/plans/2026-11-16-interior-designer-openwebui-design.md
"""
import json
import re
import sys
from pathlib import Path

import owui

HERE = Path(__file__).parent
OPENAI_URL = "https://api.openai.com/v1"
PUBLIC_READ = [{"principal_type": "user", "principal_id": "*", "permission": "read"}]


def ok(status: int, body, what: str):
    if status != 200:
        sys.exit(f"{what} failed: HTTP {status} {body}")
    return body


def upsert(kind: str, item_id: str, form: dict, get_path: str, update_path: str) -> None:
    """Create or update a workspace item. A missing item answers 401 or 404, not 200."""
    exists = owui.call("GET", get_path)[0] == 200
    path = update_path if exists else f"/api/v1/{kind}/create"
    ok(*owui.call("POST", path, form), f"{kind} {item_id}")
    print(f"{'updated' if exists else 'created'} {kind[:-1]} {item_id}")


def ensure_connection(model_id: str, api_key: str) -> None:
    config = ok(*owui.call("GET", "/openai/config"), "read connections")
    urls, keys = config["OPENAI_API_BASE_URLS"], config["OPENAI_API_KEYS"]
    configs = config.get("OPENAI_API_CONFIGS") or {}
    if OPENAI_URL in urls:
        index = urls.index(OPENAI_URL)
        keys[index] = api_key
    else:
        index = len(urls)
        urls.append(OPENAI_URL)
        keys.append(api_key)
    entry = configs.get(str(index), {})
    allowed = entry.get("model_ids") or []
    configs[str(index)] = {
        **entry,
        "enable": True,
        "connection_type": "external",
        "model_ids": allowed if model_id in allowed else [*allowed, model_id],
    }
    form = {
        "ENABLE_OPENAI_API": True,
        "OPENAI_API_BASE_URLS": urls,
        "OPENAI_API_KEYS": keys,
        "OPENAI_API_CONFIGS": configs,
    }
    ok(*owui.call("POST", "/openai/config/update", form), "update connections")
    print(f"connection {OPENAI_URL} (#{index}) allows {configs[str(index)]['model_ids']}")


def install_skill(path: Path) -> None:
    match = re.match(r"^---\n(.*?)\n---\n(.*)$", path.read_text(), re.DOTALL)
    if not match:
        sys.exit(f"{path}: missing frontmatter")
    meta = dict(line.split(":", 1) for line in match.group(1).splitlines() if ":" in line)
    skill_id = path.stem
    form = {
        "id": skill_id,
        "name": meta["name"].strip(),
        "description": meta.get("description", "").strip(),
        "content": match.group(2).strip(),
        "is_active": True,
        "access_grants": PUBLIC_READ,
    }
    upsert("skills", skill_id, form, f"/api/v1/skills/id/{skill_id}",
           f"/api/v1/skills/id/{skill_id}/update")


def install_tool(path: Path, api_key: str) -> None:
    content = path.read_text()
    tool_id = path.stem
    form = {
        "id": tool_id,
        "name": owui.frontmatter(content, "title") or tool_id,
        "content": content,
        "meta": {"description": owui.frontmatter(content, "description"), "manifest": {}},
        "access_grants": PUBLIC_READ,
    }
    upsert("tools", tool_id, form, f"/api/v1/tools/id/{tool_id}",
           f"/api/v1/tools/id/{tool_id}/update")

    # Keep valves an admin may have tuned in the UI; only the key is ours to set.
    valves = owui.call("GET", f"/api/v1/tools/id/{tool_id}/valves")[1] or {}
    valves["openai_api_key"] = api_key
    ok(*owui.call("POST", f"/api/v1/tools/id/{tool_id}/valves/update", valves), "tool valves")
    print(f"set the OpenAI key on {tool_id}")


def install_model(path: Path) -> None:
    form = {**json.loads(path.read_text()), "is_active": True, "access_grants": PUBLIC_READ}
    model_id = form["id"]
    upsert("models", model_id, form, f"/api/v1/models/model?id={model_id}",
           "/api/v1/models/model/update")


def main() -> None:
    model = json.loads((HERE / "models/interior-designer.json").read_text())
    api_key = owui.setting("OPENAI_API_KEY")
    ensure_connection(model["base_model_id"], api_key)
    install_skill(HERE / "skills/interior-design.md")
    install_tool(HERE / "tools/render_design.py", api_key)
    install_model(HERE / "models/interior-designer.json")
    print("done. Pick 'Interior Designer' in the model menu.")


if __name__ == "__main__":
    main()
