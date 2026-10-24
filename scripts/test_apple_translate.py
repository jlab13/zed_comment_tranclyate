"""Перевірки вбудованої Swift-бібліотеки через CLI LSP без системних моделей."""

import json
import os
import subprocess
import sys
import unittest
from pathlib import Path

BINARY = Path(sys.argv.pop(1) if len(sys.argv) > 1 else "target/release/comment-translate-lsp").resolve()

APPLE_AVAILABLE = sys.platform == "darwin" and os.environ.get("COMMENT_TRANSLATE_APPLE_AVAILABLE", "1") != "0"


class AppleTranslateTests(unittest.TestCase):
    def invoke(self, payload, arguments=()):
        result = subprocess.run([str(BINARY), *(arguments or ("--translate",))], input=payload,
                                text=True, capture_output=True, timeout=5)
        self.assertEqual(result.stderr, "")
        return result.returncode, json.loads(result.stdout)

    def test_returns_ukrainian_unchanged(self):
        text = "Повернути `next_value()` без змін.\nДругий рядок."
        status, body = self.invoke(json.dumps({"text": text, "source_language": "uk"}))
        self.assertEqual(status, 0)
        self.assertEqual(body["translation"], text)

    def test_rejects_invalid_json(self):
        status, body = self.invoke("{")
        self.assertNotEqual(status, 0)
        self.assertIn("JSON", body["error"])

    def test_rejects_missing_text(self):
        status, body = self.invoke("{}")
        self.assertNotEqual(status, 0)
        self.assertIn("text", body["error"])

    def test_rejects_empty_text(self):
        status, body = self.invoke(json.dumps({"text": " \n "}))
        self.assertNotEqual(status, 0)
        self.assertIn("Немає тексту", body["error"])

    def test_rejects_invalid_timeout(self):
        status, body = self.invoke(json.dumps({"text": "Текст", "timeout_seconds": 0}))
        self.assertNotEqual(status, 0)
        self.assertIn("timeout_seconds", body["error"])

    def test_checks_same_language_without_service(self):
        status, body = self.invoke("", ("--check", "uk"))
        self.assertEqual(status, 0)
        self.assertEqual(body["status"], "network_required")

    def test_google_can_be_selected_without_apple_models(self):
        status, body = self.invoke(json.dumps({"provider": "google", "operation": "check"}))
        self.assertEqual(status, 0)
        self.assertEqual(body["provider"], "google")
        self.assertEqual(body["status"], "network_required")

    def test_google_returns_explicit_ukrainian_without_a_request(self):
        status, body = self.invoke(json.dumps({"provider": "google", "source_language": "uk", "text": "Український текст"}))
        self.assertEqual(status, 0)
        self.assertEqual(body["provider"], "google")
        self.assertEqual(body["translation"], "Український текст")

    def test_rejects_unknown_provider(self):
        status, body = self.invoke(json.dumps({"provider": "unknown", "source_language": "uk", "text": "Текст"}))
        self.assertNotEqual(status, 0)
        self.assertIn("provider", body["error"])

    def test_omitted_provider_defaults_to_google(self):
        status, body = self.invoke(json.dumps({"source_language": "uk", "text": "Текст"}))
        self.assertEqual(status, 0)
        self.assertEqual(body["provider"], "google")

    @unittest.skipIf(APPLE_AVAILABLE, "Потребує збірки без Apple")
    def test_apple_is_rejected_without_native_support(self):
        status, body = self.invoke(json.dumps({"provider": "apple", "text": "Hello"}))
        self.assertNotEqual(status, 0)
        self.assertIn("macOS", body["error"])


if __name__ == "__main__":
    unittest.main()
