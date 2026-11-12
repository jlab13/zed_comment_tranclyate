"""Перевірка справжнього LSP без мережі: вихідна й цільова мова збігаються."""

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
    if provider not in ("google", "mymemory", "deepl"):
        raise SystemExit("Провайдер має бути google, mymemory або deepl")
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
                "target_language": "uk",
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
        assert formatted == "Переклад → uk\n\n" + expected, formatted
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

        def open_document(path, language_id, text):
            nonlocal uri
            uri = f"file:///tmp/{path}"
            send("textDocument/didOpen", {"textDocument": {
                "uri": uri, "languageId": language_id, "version": 1, "text": text,
            }})

        def value(result):
            assert result is not None, "Очікувався hover"
            return result["contents"]["value"]

        # Zed типово відкриває .h у режимі C++.
        open_document("comment-translate-smoke-cpp.h", "cpp", "/// Повернути значення C++.\nint value();\n")
        assert "Повернути значення C++." in value(hover(0, 5))
        send("textDocument/didClose", {"textDocument": {"uri": uri}})

        rust_doc = "/// Поточне значення не змінюється."
        open_document("comment-translate-smoke.rs", "rust", (
            "//! Модуль лічильника.\n"
            "\n"
            "/// Повертає наступне значення.\n"
            f"{rust_doc}\n"
            "fn next(value: u32) -> u32 {\n"
            '    let marker = r#"// не коментар"#;\n'
            "    value + 1 // Додати одиницю.\n"
            "}\n"
        ))
        assert "Модуль лічильника." in value(hover(0, 5))
        documentation = hover(2, 5)
        assert "Повертає наступне значення. Поточне значення не змінюється." in value(documentation)
        assert documentation == hover(3, 5)
        assert documentation["range"] == {
            "start": {"line": 2, "character": 0},
            "end": {"line": 3, "character": len(rust_doc)},
        }, "Діапазон doc-коментаря Rust має закінчуватися до переносу рядка"
        assert hover(3, len(rust_doc)) is None
        assert hover(5, 22) is None, "Сирі рядки Rust не повинні перекладатися"
        plain = hover(6, 20)
        assert "Додати одиницю." in value(plain)
        assert documentation["contents"]["kind"] == "markdown", "Документація Rust показується як Markdown"
        assert plain["contents"]["kind"] == "plaintext", "Звичайний коментар Rust показується як текст"
        assert "Повертає наступне значення.  \nПоточне" not in value(documentation)
        send("textDocument/didClose", {"textDocument": {"uri": uri}})

        open_document("comment-translate-smoke.py", "python", (
            "#!/usr/bin/env python3\n"
            '"""Модуль прикладу."""\n'
            "# Перший рядок.\n"
            "# Другий рядок.\n"
            'marker = "# не коментар"\n'
            "\n"
            "\n"
            "def value():\n"
            '    """\n'
            "    Повертає значення.\n"
            "\n"
            "    Другий абзац.\n"
            '    """\n'
            '    """Не docstring."""\n'
            "    return 1\n"
        ))
        assert hover(0, 3) is None, "Shebang не повинен перекладатися"
        assert "Модуль прикладу." in value(hover(1, 5))
        assert "Перший рядок. Другий рядок." in value(hover(3, 3))
        assert hover(4, 12) is None, "Рядки Python не повинні перекладатися"
        docstring = hover(9, 6)
        assert "Повертає значення.\n\nДругий абзац." in value(docstring), docstring
        assert docstring["range"] == {"start": {"line": 8, "character": 4}, "end": {"line": 12, "character": 7}}
        assert hover(13, 8) is None, "Лише перший рядковий літерал функції є docstring"
        send("textDocument/didClose", {"textDocument": {"uri": uri}})

        open_document("comment-translate-smoke.sh", "shellscript", '#!/bin/sh\n# Зібрати проєкт.\necho "$#"\n')
        assert hover(0, 3) is None
        assert "Зібрати проєкт." in value(hover(1, 3))
        assert hover(2, 7) is None
        send("textDocument/didClose", {"textDocument": {"uri": uri}})

        open_document("comment-translate-smoke.md", "markdown", "# Заголовок\n<!-- коментар -->\n")
        assert hover(0, 3) is None, "Непідтримувані мови ігноруються"
        send("textDocument/didClose", {"textDocument": {"uri": uri}})

        send("shutdown", request=True)
        send("exit")
        assert process.wait(timeout=5) == 0
        print(f"LSP smoke test ({provider}): OK — C, .h (C і C++), Rust, Python, Shell Script, "
              "hover, кеш, зміни, закриття та ліміти")
    finally:
        if process.poll() is None:
            process.kill()
        process.wait()
        process.stdin.close()
        process.stdout.close()


if __name__ == "__main__":
    main()
