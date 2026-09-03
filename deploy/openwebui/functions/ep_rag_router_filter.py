"""
title: EP RAG Router
author: local_coding_harness
description: On each user turn, ask rag-mcp /route whether to ground in EP committee
             documents; if so, inject the retrieved+grounded context. Deterministic, no tool-calling.
version: 0.1.0
"""
import requests
from pydantic import BaseModel, Field


class Filter:
    class Valves(BaseModel):
        route_url: str = Field(default="http://localhost:8082/route")
        timeout_s: int = Field(default=120)

    def __init__(self):
        self.valves = self.Valves()

    def inlet(self, body: dict, __user__: dict = None) -> dict:
        msgs = body.get("messages", [])
        last_user = next((m for m in reversed(msgs) if m.get("role") == "user"), None)
        if not last_user or not last_user.get("content"):
            return body
        try:
            r = requests.post(self.valves.route_url,
                              json={"message": last_user["content"]},
                              timeout=self.valves.timeout_s)
            data = r.json()
        except Exception:
            return body  # fail open: never block a turn if the router is down
        if data.get("should_ground") and data.get("context"):
            # Prepend grounded context as a system message; the persona's own model generates.
            body["messages"] = [{"role": "system", "content": data["context"]}] + msgs
        return body
