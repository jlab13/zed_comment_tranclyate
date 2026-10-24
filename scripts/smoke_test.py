"""Перевірка справжнього LSP зі статично вбудованою Swift-бібліотекою."""

import json
import os
import queue
import subprocess
import sys
import threading
from pathlib import Path


def read_messages(stream, messages):
    try:
        while True:
            headers = {}
            while True:
                line = stream.readline()
                if not line:
                    raise EOFError("LSP завершився до відповіді")
                if line == b"\r\n":
                    break
                key, value = line.decode().split(":", 1)
                headers[key.lower()] = value.strip()
            messages.put(json.loads(stream.read(int(headers["content-length"]))))
    except Exception as error:
        messages.put(error)


def main():
    binary = Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/comment-translate-lsp").resolve()
    if not binary.is_file():
        raise SystemExit("Спочатку виконайте cargo build -p comment-translate-lsp")
    provider = sys.argv[2] if len(sys.argv) > 2 else "google"
    if provider not in ("apple", "google"):
        raise SystemExit("Провайдер має бути apple або google")
    process = subprocess.Popen([str(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    messages = queue.Queue()
    threading.Thread(target=read_messages, args=(process.stdout, messages), daemon=True).start()
    next_id = 0

    def send(method, params=None, request=False):
        nonlocal next_id
        message = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            message["params"] = params
        if request:
            next_id += 1
            message["id"] = next_id
        payload = json.dumps(message).encode()
        process.stdin.write(f"Content-Length: {len(payload)}\r\n\r\n".encode() + payload)
        process.stdin.flush()
        if request:
            while True:
                response = messages.get(timeout=10)
                if isinstance(response, Exception):
                    raise response
                if response.get("id") == next_id:
                    assert "error" not in response, response
                    return response.get("result")

    uri = "file:///tmp/comment-translate-smoke.c"

    def hover(line=0, character=4):
        return send("textDocument/hover", {
            "textDocument": {"uri": uri},
            "position": {"line": line, "character": character},
        }, request=True)

    def change(version, text):
        send("textDocument/didChange", {
            "textDocument": {"uri": uri, "version": version},
            "contentChanges": [{"text": text}],
        })

    try:
        result = send("initialize", {
            "processId": None,
            "capabilities": {},
            "initializationOptions": {
                "source_language": "uk",
                "provider": provider,
            },
        }, request=True)
        assert result["capabilities"]["hoverProvider"] is True
        send("initialized", {})
        send("textDocument/didOpen", {"textDocument": {
            "uri": uri, "languageId": "c", "version": 1,
            "text": '// Повернути значення.\nconst char *s = "// fake";\n',
        }})
        result = hover()
        assert "Повернути значення." in result["contents"]["value"], result
        assert hover() == result
        assert hover(1, 20) is None, "Рядки C не повинні перекладатися"
        assert hover(20) is None
        change(2, "/*\n * Повернути інше значення.\n */")
        assert "Повернути інше значення." in hover(1)["contents"]["value"]
        change(3, "// Перший рядок.\n// Другий рядок.")
        first = hover(0)
        second = hover(1)
        assert first == second, "Кожний рядок має повертати весь блок і однаковий діапазон"
        assert "Перший рядок. Другий рядок." in first["contents"]["value"]
        assert first["range"] == {"start": {"line": 0, "character": 0}, "end": {"line": 1, "character": 16}}
        change(4, "// Перший\n// абзац.\n//\n// Другий\n// абзац.")
        assert "Перший абзац.\n\nДругий абзац." in hover(1)["contents"]["value"]
        change(5, "// " + "x" * 4001)
        assert "max_comment_chars" in hover()["contents"]["value"]
        change(6, "/**\n * @brief Ініціалізувати контекст.\n *\n * Дії:\n * 1. Відкрити інтерфейс\n * 2. Створити чергу\n *\n * Параметри:\n * - Вузол\n *   - Ідентифікатор\n * @param node Вказівник на вузол.\n * @return 0 у разі успіху.\n */")
        formatted = hover(5)["contents"]["value"]
        expected = "@brief Ініціалізувати контекст.\n\nДії:\n1. Відкрити інтерфейс\n2. Створити чергу\n\nПараметри:\n- Вузол\n  - Ідентифікатор\n@param node Вказівник на вузол.\n@return 0 у разі успіху."
        assert formatted == "Переклад українською\n\n" + expected, formatted
        send("textDocument/didClose", {"textDocument": {"uri": uri}})
        assert hover() is None
        uri = "file:///tmp/comment-translate-smoke.h"
        send("textDocument/didOpen", {"textDocument": {
            "uri": uri, "languageId": "c", "version": 1,
            "text": "#ifndef DEMO_H\n#define DEMO_H\n/* Повернути значення заголовка. */\nint header_value(void);\n#endif\n",
        }})
        header = hover(2)
        assert "Повернути значення заголовка." in header["contents"]["value"], header
        send("textDocument/didClose", {"textDocument": {"uri": uri}})
        assert hover(2) is None
        send("shutdown", request=True)
        send("exit")
        assert process.wait(timeout=5) == 0
        print(f"LSP smoke test ({provider}): OK — C і .h, hover, кеш, зміни, закриття, вбудований Swift та ліміти")
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()
        process.stdin.close()
        process.stdout.close()


if __name__ == "__main__":
    main()
