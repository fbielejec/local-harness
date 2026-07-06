#!/usr/bin/env python3
"""
Reasoning quality gate for the local coding harness (autoperf).

Methodology adapted from Sebastian Raschka's `local-coding-agent-evals`
(hard-tool-reasoning-benchmark): five one-shot tool-reasoning tasks, deterministic
substring grading (1.0 / 0.5 / 0.0). The repo carries no license, so this is a
reimplementation of the *method* for our stack — the transport is repointed from
Ollama `/api/chat` to our llama.cpp OpenAI-compatible `/v1/chat/completions`
endpoint (reached over the SSH tunnel), and the grader follows Raschka's rubric.

Deviation from the original: we add `read_file` to the tool catalog. Raschka's
catalog omits it, but task `triage_import_error_after_refactor` expects `read_file`
(likely an oversight); adding it makes all five tasks winnable, which is what we want
for a *relative* quality gate against our own Q4_K_M baseline.

Emits one machine-readable summary line the autoperf loop parses:
    QUALITY_REASONING total=<X.XX>/5 mean=<0.XXX> strict_passed=<n>/5 runs=<R>
Exit code 0 iff strict_passed == number of tasks (all tasks pass every repeat).
"""
import argparse
import csv
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

DEFAULT_BASE_URL = os.environ.get("OPENAI_BASE_URL", "http://localhost:8080/v1")
DEFAULT_MODEL = os.environ.get(
    "OPENAI_MODEL", "unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF:Q4_K_M"
)
DEFAULT_API_KEY = os.environ.get("OPENAI_API_KEY", "dummy")
DEFAULT_TASKS = Path(__file__).with_name("reasoning_tasks.jsonl")

TOOLS = [
    {
        "name": "read_file",
        "description": "Read one file before acting.",
        "arguments": {"path": "relative path"},
    },
    {
        "name": "edit_file",
        "description": "Edit one file with precise instructions.",
        "arguments": {"path": "relative path", "instructions": "short edit instruction"},
    },
    {
        "name": "ask_clarification",
        "description": "Ask one concise question when the next action is ambiguous or risky.",
        "arguments": {"question": "question text"},
    },
    {
        "name": "final_answer",
        "description": "Answer directly when the task can be solved from the prompt.",
        "arguments": {"answer": "concise answer"},
    },
]

SYSTEM_PROMPT = """You are being evaluated on hard tool-use reasoning.
Choose exactly one next action. Do not execute tools. Do not explain outside JSON.
Return only one JSON object with this schema:
{"tool": "<tool name>", "arguments": {"key": "value"}}

Tool catalog:
{tools}
"""

JSON_SCHEMA = {
    "type": "object",
    "properties": {"tool": {"type": "string"}, "arguments": {"type": "object"}},
    "required": ["tool", "arguments"],
}


def load_tasks(path):
    tasks = []
    for line_number, line in enumerate(
        path.read_text(encoding="utf-8").splitlines(), start=1
    ):
        if not line.strip():
            continue
        try:
            tasks.append(json.loads(line))
        except json.JSONDecodeError as exc:
            raise ValueError(f"{path}:{line_number}: invalid JSON: {exc}") from exc
    return tasks


def build_prompt(task):
    return (
        SYSTEM_PROMPT.replace("{tools}", json.dumps(TOOLS, indent=2))
        + "\nTask:\n"
        + task["prompt"]
        + "\n\nReturn only JSON."
    )


def extract_json(text):
    stripped = text.strip()
    fence_match = re.fullmatch(r"```(?:json)?\s*(.*?)\s*```", stripped, re.DOTALL)
    if stripped.startswith("```") and fence_match:
        stripped = fence_match.group(1).strip()
    try:
        return json.loads(stripped)
    except json.JSONDecodeError:
        start = stripped.find("{")
        end = stripped.rfind("}")
        if start == -1 or end == -1 or end <= start:
            raise
        return json.loads(stripped[start : end + 1])


def normalize_text(value):
    if value is None:
        return ""
    return " ".join(str(value).strip().lower().split())


def contains_all(value, expected_parts):
    normalized = normalize_text(value)
    return all(normalize_text(part) in normalized for part in expected_parts)


def score_response(task, response):
    expected = task["expected"]
    expected_tool = expected["tool"]
    actual_tool = response.get("tool")
    if actual_tool != expected_tool:
        return {"passed": False, "score": 0.0,
                "reason": f"wrong tool: expected {expected_tool}, got {actual_tool}"}

    arguments = response.get("arguments")
    if not isinstance(arguments, dict):
        return {"passed": False, "score": 0.0, "reason": "arguments must be an object"}

    for key, expected_value in expected.get("required_arguments", {}).items():
        if arguments.get(key) != expected_value:
            return {"passed": False, "score": 0.5,
                    "reason": f"wrong argument {key}: expected {expected_value!r}, got {arguments.get(key)!r}"}

    for key, expected_parts in expected.get("argument_contains", {}).items():
        if not contains_all(arguments.get(key), expected_parts):
            return {"passed": False, "score": 0.5,
                    "reason": f"argument {key} missing required content"}

    if expected.get("answer_contains"):
        if not contains_all(arguments.get("answer"), expected["answer_contains"]):
            return {"passed": False, "score": 0.5, "reason": "answer missing required content"}

    return {"passed": True, "score": 1.0, "reason": "ok"}


def post_json(url, payload, timeout_s, api_key):
    body = json.dumps(payload).encode("utf-8")
    request = urllib.request.Request(
        url,
        data=body,
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {api_key}"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout_s) as response:
            return json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as exc:
        error_body = exc.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"HTTP {exc.code}: {error_body}") from exc
    except urllib.error.URLError as exc:
        raise RuntimeError(f"could not connect to {url}: {exc}") from exc


# Remember which response_format the server accepts, to avoid retrying every call.
_FORMAT_MODE = {"mode": None}  # None -> untried, "schema", "object", or "none"


def call_model(base_url, api_key, model, prompt, timeout_s, temperature, max_tokens, fmt):
    url = f"{base_url.rstrip('/')}/chat/completions"

    def payload_for(mode):
        p = {
            "model": model,
            "messages": [{"role": "user", "content": prompt}],
            "temperature": temperature,
            "max_tokens": max_tokens,
        }
        if mode == "schema":
            p["response_format"] = {
                "type": "json_schema",
                "json_schema": {"name": "tool_action", "schema": JSON_SCHEMA, "strict": True},
            }
        elif mode == "object":
            p["response_format"] = {"type": "json_object"}
        return p

    # Decide order of modes to try.
    if fmt == "auto":
        order = [_FORMAT_MODE["mode"]] if _FORMAT_MODE["mode"] else ["schema", "object", "none"]
    else:
        order = [fmt]

    last_err = None
    for mode in order:
        try:
            resp = post_json(url, payload_for(mode), timeout_s, api_key)
            _FORMAT_MODE["mode"] = mode
            return resp["choices"][0]["message"].get("content", "") or ""
        except RuntimeError as exc:
            last_err = exc
            if fmt != "auto":
                raise
            continue
    raise last_err


def run_benchmark(args):
    tasks = load_tasks(args.tasks)
    rows = []
    # per-task list of scores across repeats
    per_task_scores = {t["id"]: [] for t in tasks}
    for rep in range(1, args.repeats + 1):
        for task in tasks:
            started = time.monotonic()
            try:
                raw = call_model(args.base_url, args.api_key, args.model,
                                 build_prompt(task), args.timeout, args.temperature,
                                 args.max_tokens, args.format)
            except RuntimeError as exc:
                raw = ""
                parsed = None
                result = {"passed": False, "score": 0.0, "reason": f"request failed: {exc}"}
            else:
                try:
                    parsed = extract_json(raw)
                    result = score_response(task, parsed)
                except Exception as exc:
                    parsed = None
                    result = {"passed": False, "score": 0.0, "reason": f"invalid JSON: {exc}"}
            elapsed = time.monotonic() - started
            per_task_scores[task["id"]].append(result["score"])
            rows.append({
                "repeat": rep,
                "id": task["id"],
                "category": task.get("category", ""),
                "score": result["score"],
                "reason": result["reason"],
                "expected_tool": task["expected"]["tool"],
                "actual_tool": parsed.get("tool") if isinstance(parsed, dict) else "",
                "elapsed_s": f"{elapsed:.2f}",
                "raw": raw,
            })
            status = "PASS" if result["passed"] else ("HALF" if result["score"] == 0.5 else "FAIL")
            print(f"[rep {rep}] {status} {task['id']}: {result['reason']}", flush=True)

    # aggregate
    n = len(tasks)
    task_means = {tid: sum(s) / len(s) for tid, s in per_task_scores.items()}
    total = sum(task_means.values())
    mean = total / n if n else 0.0
    strict_passed = sum(1 for tid in per_task_scores if min(per_task_scores[tid]) == 1.0)

    print()
    for t in tasks:
        tid = t["id"]
        print(f"  {tid}: mean {task_means[tid]:.2f}  (scores {per_task_scores[tid]})")
    print()
    print(f"QUALITY_REASONING total={total:.2f}/{n} mean={mean:.3f} "
          f"strict_passed={strict_passed}/{n} runs={args.repeats}")

    if args.csv:
        with args.csv.open("w", newline="", encoding="utf-8") as fh:
            w = csv.DictWriter(fh, fieldnames=list(rows[0].keys()))
            w.writeheader()
            w.writerows(rows)
        print(f"Wrote CSV: {args.csv}")

    return 0 if strict_passed == n else 1


def build_parser():
    p = argparse.ArgumentParser(description="Reasoning quality gate against an OpenAI-compatible endpoint.")
    p.add_argument("--base-url", default=DEFAULT_BASE_URL, help=f"Default: {DEFAULT_BASE_URL}")
    p.add_argument("--model", default=DEFAULT_MODEL, help=f"Default: {DEFAULT_MODEL}")
    p.add_argument("--api-key", default=DEFAULT_API_KEY)
    p.add_argument("--tasks", type=Path, default=DEFAULT_TASKS)
    p.add_argument("--timeout", type=float, default=120.0, help="Per-call timeout s. Default: 120.")
    p.add_argument("--temperature", type=float, default=0.0)
    p.add_argument("--max-tokens", type=int, default=768)
    p.add_argument("--repeats", type=int, default=3, help="Runs per task (determinism guard). Default: 3.")
    p.add_argument("--format", choices=["auto", "schema", "object", "none"], default="auto",
                   help="response_format mode. auto = try json_schema, then json_object, then none.")
    p.add_argument("--csv", type=Path)
    return p


def main(argv=None):
    args = build_parser().parse_args(argv)
    try:
        return run_benchmark(args)
    except (RuntimeError, ValueError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
