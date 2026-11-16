"""
title: Render Design
author: local_coding_harness
description: Renders interior-design concepts with OpenAI GPT Image. Restyles the latest room photo or render in the chat, or draws a new room from a description. Images and instructions go to OpenAI.
version: 0.1.0
"""
import asyncio
import base64
import datetime
import json
import mimetypes
from typing import Literal

import requests
from pydantic import BaseModel, Field

SOURCES = ("latest", "photo", "none")

# Renders per (user id, ISO date). In process memory, so it resets when the container restarts:
# a guard against a runaway chat, not a billing control. The OpenAI spend limit is the hard cap.
_renders: dict[tuple[str, str], int] = {}


def is_image(file: dict) -> bool:
    return file.get("type") == "image" or (file.get("content_type") or "").startswith("image/")


def find_source_image(messages: list[dict], source: str) -> str | None:
    """URL of the newest image in `messages` (oldest first). `photo` skips the model's renders."""
    for message in reversed(messages):
        if source == "photo" and message.get("role") != "user":
            continue
        urls = [f["url"] for f in message.get("files") or [] if is_image(f) and f.get("url")]
        if urls:
            return urls[-1]
    return None


def messages_from_request(messages: list[dict]) -> list[dict]:
    """Request messages (OpenAI shape, images inline) in the {role, files} shape of chat history.

    Fallback for chats with no stored history (temporary chats, API calls). Only user images
    appear here: Open WebUI drops the model's earlier renders from the request.
    """
    converted = []
    for message in messages:
        content = message.get("content")
        parts = content if isinstance(content, list) else []
        files = [
            {"type": "image", "url": part["image_url"]["url"]}
            for part in parts
            if part.get("type") == "image_url" and part.get("image_url", {}).get("url")
        ]
        converted.append({"role": message.get("role"), "files": files})
    return converted


def renders_left(user_id: str, limit: int, today: str) -> int:
    return limit - _renders.get((user_id, today), 0)


def record_render(user_id: str, today: str) -> None:
    _renders[(user_id, today)] = _renders.get((user_id, today), 0) + 1


def decode_data_url(url: str) -> tuple[bytes, str]:
    header, data = url.split(",", 1)
    return base64.b64decode(data), header[len("data:") :].split(";")[0] or "image/png"


class Tools:
    class Valves(BaseModel):
        openai_api_key: str = Field(default="", description="Set by install_interior_designer.py")
        base_url: str = "https://api.openai.com/v1"
        model: str = "gpt-image-1.5"
        quality: Literal["low", "medium", "high", "auto"] = "medium"
        size: str = Field(default="auto", description="auto follows the photo's orientation")
        input_fidelity: Literal["", "low", "high"] = Field(
            default="", description="high keeps more of the photo, at a higher input cost"
        )
        daily_renders_per_user: int = Field(default=20, ge=0)
        timeout_s: int = 180

    def __init__(self):
        self.valves = self.Valves()

    async def render_design(
        self,
        instruction: str,
        source: str = "latest",
        __user__: dict = None,
        __request__=None,
        __event_emitter__=None,
        __chat_id__: str = None,
        __message_id__: str = None,
        __messages__: list = None,
    ) -> str:
        """
        Render an interior-design image and show it to the user. Each call costs money: render
        once per reply unless the user asks for more.

        :param instruction: Complete, self-contained image instruction. The image model sees only this and the source image: say what must stay the same, then name every change (object, new form, material, colour), the lighting, and "photorealistic interior photograph".
        :param source: "latest" edits the newest image in the chat (your last render, else the user's photo) and suits follow-ups; "photo" edits the user's newest uploaded photo, for a fresh concept; "none" draws a new room from the instruction alone.
        :return: Whether the render succeeded. The image is already visible to the user.
        """
        if source not in SOURCES:
            return json.dumps({"error": f"source must be one of {SOURCES}"})
        if not self.valves.openai_api_key:
            return json.dumps({"error": "Rendering is not configured (no API key). Tell the user."})

        from open_webui.models.users import UserModel

        user = UserModel(**__user__)
        today = datetime.date.today().isoformat()
        if renders_left(user.id, self.valves.daily_renders_per_user, today) <= 0:
            return json.dumps(
                {"error": f"Daily limit of {self.valves.daily_renders_per_user} renders reached. "
                          "Tell the user it resets tomorrow."}
            )

        async def status(description: str, done: bool) -> None:
            if __event_emitter__:
                await __event_emitter__(
                    {"type": "status", "data": {"description": description, "done": done}}
                )

        source_url = None
        if source != "none":
            source_url = await self._find_image(__chat_id__, __message_id__, user, source)
            if source_url is None:
                source_url = find_source_image(messages_from_request(__messages__ or []), source)
            if source_url is None:
                return json.dumps(
                    {"error": "No photo in this chat. Ask the user to upload one, or call again "
                              "with source='none' to draw a room from the description."}
                )

        await status("Rendering design…", False)
        try:
            image = await self._load_image(source_url, user) if source_url else None
            png = await asyncio.to_thread(self._call_openai, instruction, image)
            url = await self._save(png, __request__, user, __chat_id__, __message_id__, __event_emitter__)
        except Exception as error:  # reported to the model, which explains it to the user
            await status("Rendering failed", True)
            return json.dumps({"error": f"Rendering failed: {error}"})

        record_render(user.id, today)
        await status("Design rendered", True)
        return json.dumps(
            {
                "status": "success",
                "url": url,
                "message": "The render is already shown to the user: do not embed or link it. "
                           "Describe what changed and offer one or two next steps.",
            }
        )

    async def _find_image(self, chat_id, message_id, user, source: str) -> str | None:
        if not chat_id or chat_id.startswith(("local:", "channel:")):
            return None
        from open_webui.models.chats import Chats
        from open_webui.utils.misc import get_message_list

        chat = await Chats.get_chat_by_id_and_user_id(chat_id, user.id)
        if chat is None:
            return None
        history = chat.chat.get("history", {})
        messages_map = history.get("messages", {})
        start = message_id if message_id in messages_map else history.get("currentId")
        if not start:
            return None
        return find_source_image(get_message_list(messages_map, start), source)

    async def _load_image(self, url: str, user) -> tuple[bytes, str]:
        if url.startswith("data:"):
            return decode_data_url(url)
        if url.startswith(("http://", "https://")):
            raise ValueError("external image URLs are not fetched")  # no SSRF from chat content

        from open_webui.models.files import Files
        from open_webui.storage.provider import Storage

        file_id = url.split("/api/v1/files/")[1].split("/")[0] if "/api/v1/files/" in url else url
        file = await Files.get_file_by_id(file_id)
        # Images in a user's own chat are theirs: uploads and earlier renders alike.
        if file is None or (file.user_id != user.id and user.role != "admin"):
            raise ValueError("source image not found")
        path = await asyncio.to_thread(Storage.get_file, file.path)
        with open(path, "rb") as f:
            data = f.read()
        mime = (file.meta or {}).get("content_type") or mimetypes.guess_type(path)[0]
        return data, mime or "image/png"

    def _call_openai(self, instruction: str, image: tuple[bytes, str] | None) -> bytes:
        valves = self.valves
        headers = {"Authorization": f"Bearer {valves.openai_api_key}"}
        fields = {
            "model": valves.model,
            "prompt": instruction,
            "quality": valves.quality,
            "size": valves.size,
            "n": 1,
        }
        if image is None:
            response = requests.post(
                f"{valves.base_url}/images/generations",
                headers=headers, json=fields, timeout=valves.timeout_s,
            )
        else:
            if valves.input_fidelity:
                fields["input_fidelity"] = valves.input_fidelity
            data, mime = image
            extension = mimetypes.guess_extension(mime) or ".png"
            response = requests.post(
                f"{valves.base_url}/images/edits",
                headers=headers, data=fields, timeout=valves.timeout_s,
                files=[("image[]", (f"source{extension}", data, mime))],
            )
        body = response.json()
        if response.status_code != 200:
            raise RuntimeError(body.get("error", {}).get("message", f"HTTP {response.status_code}"))
        return base64.b64decode(body["data"][0]["b64_json"])

    async def _save(self, png: bytes, request, user, chat_id, message_id, event_emitter) -> str:
        from open_webui.models.chats import Chats
        from open_webui.routers.images import upload_image

        _, url = await upload_image(
            request, png, "image/png", {"chat_id": chat_id, "message_id": message_id}, user
        )
        files = [{"type": "image", "url": url}]
        # Same steps as Open WebUI's built-in edit_image: attach to the message, then show it.
        if chat_id and message_id:
            stored = await Chats.add_message_files_by_id_and_message_id(chat_id, message_id, files)
            files = stored if stored is not None else files
        if event_emitter:
            await event_emitter({"type": "chat:message:files", "data": {"files": files}})
        return url
