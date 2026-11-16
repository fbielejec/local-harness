"""Unit tests for the pure parts of tools/render_design.py (no Open WebUI, no network).

Run:  python3 -m unittest discover -s deploy/openwebui/tests
"""
import base64
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
import render_design as rd  # noqa: E402

PHOTO = {"type": "image", "url": "/api/v1/files/photo/content"}
RENDER = {"type": "image", "url": "/api/v1/files/render/content"}
PDF = {"type": "file", "content_type": "application/pdf", "url": "/api/v1/files/doc/content"}


class FindSourceImage(unittest.TestCase):
    history = [
        {"role": "user", "files": [PHOTO]},
        {"role": "assistant", "files": [RENDER]},
        {"role": "user", "files": []},
    ]

    def test_latest_prefers_the_newest_image_which_is_the_render(self):
        self.assertEqual(rd.find_source_image(self.history, "latest"), RENDER["url"])

    def test_photo_skips_renders_and_returns_the_upload(self):
        self.assertEqual(rd.find_source_image(self.history, "photo"), PHOTO["url"])

    def test_newest_of_several_images_in_one_message(self):
        second = {"type": "image", "url": "/api/v1/files/second/content"}
        history = [{"role": "user", "files": [PHOTO, second]}]
        self.assertEqual(rd.find_source_image(history, "photo"), second["url"])

    def test_content_type_counts_and_other_files_do_not(self):
        upload = {"type": "file", "content_type": "image/jpeg", "url": "/api/v1/files/jpg/content"}
        history = [{"role": "user", "files": [upload, PDF]}]
        self.assertEqual(rd.find_source_image(history, "photo"), upload["url"])

    def test_no_image_gives_none(self):
        history = [{"role": "user", "files": [PDF]}, {"role": "assistant"}]
        self.assertIsNone(rd.find_source_image(history, "latest"))


class MessagesFromRequest(unittest.TestCase):
    def test_inline_user_images_become_files(self):
        messages = [
            {"role": "system", "content": "be nice"},
            {"role": "user", "content": [
                {"type": "text", "text": "my room"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA"}},
            ]},
        ]
        converted = rd.messages_from_request(messages)
        self.assertEqual(converted[0], {"role": "system", "files": []})
        self.assertEqual(
            rd.find_source_image(converted, "photo"), "data:image/png;base64,AAAA"
        )


class DailyCap(unittest.TestCase):
    def setUp(self):
        rd._renders.clear()

    def test_counts_per_user_per_day(self):
        rd.record_render("alice", "2026-11-16")
        rd.record_render("alice", "2026-11-16")
        self.assertEqual(rd.renders_left("alice", 3, "2026-11-16"), 1)
        self.assertEqual(rd.renders_left("bob", 3, "2026-11-16"), 3)
        self.assertEqual(rd.renders_left("alice", 3, "2026-11-17"), 3)


class DecodeDataUrl(unittest.TestCase):
    def test_bytes_and_mime(self):
        data, mime = rd.decode_data_url("data:image/jpeg;base64," + base64.b64encode(b"jpg").decode())
        self.assertEqual((data, mime), (b"jpg", "image/jpeg"))


if __name__ == "__main__":
    unittest.main()
