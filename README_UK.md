[English](README.md) · **Українська**

# Переклад коментарів коду в Zed

Розширення для **macOS та Linux**, яке показує переклад коментаря мовою системи
(або мовою з `target_language`) у стандартному hover-вікні Zed. На macOS та Linux типовий провайдер —
**Google Translate** без API-ключа. **Apple Translation** доступний на macOS 26+ через конфігурацію.
Підтримує **C, C++, Rust, Python, Shell Script (Bash), Swift і Dart**: рядкові, блокові
й документаційні коментарі, а також docstring Python. Рядки, сирі рядки, інтерполяція та heredoc
відрізняються від коментарів через Tree-sitter. Код у файлі не змінюється.

| Мова Zed | Коментарі | Примітки |
| --- | --- | --- |
| C, C++ | `//`, `///`, `//!`, `/* */`, `/** */`, `/*! */` | `.h` працює і в режимі C, і в режимі C++ |
| Rust | `//`, `///`, `//!`, `/* */`, `/** */`, `/*! */` | вкладені блокові коментарі |
| Swift | `//`, `///`, `/* */`, `/** */` | вкладені блокові коментарі; мову надає розширення Swift |
| Dart | `//`, `///`, `/* */`, `/** */` | вкладені блокові коментарі; мову надає розширення Dart |
| Python | `#`, docstring | shebang `#!` у першому рядку не перекладається |
| Shell Script | `#` | shebang `#!` у першому рядку не перекладається |

## Архітектура

```text
Zed → WASM-розширення → Rust LSP → Google Translate (Rust, усі платформи)
                              → Apple Translation (Swift, лише macOS)
```

- `src/lib.rs` — розширення Zed, яке запускає сервер.
- `server/src/` — LSP, розпізнавання коментарів, UTF-16 координати й кеш.
- `server/src/languages.rs` — таблиця мов: languageId, розширення файлів, граматика й маркери.
- `server/src/locale.rs` — визначення цільової мови із системної локалі.
- `server/src/google.rs` — спільний Google HTTP-клієнт на reqwest/rustls.
- `native/` — Apple-перекладач Swift, вибір стратегії та C-сумісний міст.
- `server/build.rs` — компонування Swift лише для macOS з feature `apple`.
- `examples/` — налаштування Zed, гаряча клавіша та C-файли.
- `scripts/` — збірка й перевірки.

Для Apple LSP викликає Swift-бібліотеку в тому самому процесі через C ABI. Відповідь надходить
асинхронним callback. Swift копіює вхідний рядок, Rust копіює відповідь до завершення
callback; між мовами не передається право власності на пам’ять. В Apple-збірці головний потік
обслуговує чергу macOS, а Tokio/LSP працює на іншому потоці того самого процесу.
На Linux Tokio/LSP запускається напряму без Swift і Apple-фреймворків.
Цільова мова береться із системної локалі або з `target_language` (див. [Мова перекладу](#мова-перекладу)).
Вихідна мова задається через `source_language`;
`auto` використовує NaturalLanguage для Apple і визначення мови Google для Google.
Якщо вихідна мова збігається з цільовою, текст повертається без змін.
Apple на macOS 26.4+ спочатку перевіряє `highFidelity`, потім `lowLatency`; переклад
виконується локально. Google використовує переносний HTTP-клієнт Rust reqwest із TLS rustls та інтернет.
Провайдери не перемикаються автоматично в разі помилки.
До 256 успішних результатів кешуються в пам’яті; після заповнення кеш очищується.

## Збірка

На обох платформах потрібні Rust через rustup, C-компілятор для Tree-sitter і доступ
до crates.io для першого завантаження залежностей. Для Debian/Ubuntu C-компілятор
можна встановити через `sudo apt install build-essential`. OpenSSL для Google не потрібен.

На macOS Apple-збірка додатково потребує macOS 26+ та Xcode із SDK macOS 26.4+.
На Linux Swift, Xcode й Apple SDK не використовуються.

```sh
rustup target add wasm32-wasip2
./scripts/build.sh
```

Скрипт створює `target/release/comment-translate-lsp` і WASM-розширення. На macOS Cargo
автоматично вбудовує Swift-бібліотеку; на Linux `build.rs` пропускає цей крок.
`cfg(all(target_os = "macos", feature = "apple"))` відокремлює всі виклики Apple.

Для збірки без Apple навіть на macOS:

```sh
cargo build --release -p comment-translate-lsp --no-default-features
```

У цьому режимі типовий провайдер — Google, `xcrun` не викликається, а вибір `apple`
повертає зрозумілу помилку. На Linux feature `apple` не додає Apple-код навіть коли
ввімкнена за замовчуванням. Не збирайте весь workspace для WASM: LSP є нативним процесом.
Якщо раніше було задано `initialization_options.helper_path`, видаліть цей параметр.

## Вибір провайдера

Без поля `provider` використовується **Google на всіх платформах**. Для Google додайте до налаштувань Zed:

```json
{
  "lsp": {
    "comment-translate": {
      "initialization_options": {
        "provider": "google",
        "source_language": "auto",
        "timeout_seconds": 60,
        "max_comment_chars": 4000
      }
    }
  }
}
```

Щоб повернутися до Apple на macOS, встановіть `"provider": "apple"`.
Поле `target_language` описано в розділі [Мова перекладу](#мова-перекладу).
На Linux явний вибір `apple` відхиляється без звернення до Google.
Збережіть наявне поле `binary` та інші налаштування. Після перемикання перезапустіть
мовні сервери або Zed. Приклади: [`zed-settings.json`](examples/zed-settings.json)
і [`zed-settings-google.json`](examples/zed-settings-google.json). Для Linux є
[`zed-settings-linux.json`](examples/zed-settings-linux.json): замініть шлях до сервера.

Google працює без API-ключів і мовних пакетів Apple, але потребує інтернету та надсилає
текст коментаря Google. Використовується веб endpoint
`https://translate.googleapis.com/translate_a/single` з `client=gtx`, а не платний Cloud
Translation API. Цей вебінтерфейс може змінюватися або обмежувати частоту запитів.
HTTP 403/429, помилки мережі й некоректний JSON показуються в hover українською.
Кеш та об’єднання рядків в абзаци працюють для обох провайдерів.

Перевірка Google через CLI:

```sh
echo '{"provider":"google","text":"Return the next counter value.","source_language":"en"}' \
  | ./target/release/comment-translate-lsp --translate
```

`--check en` повертає `network_required` для типового Google; доступ до Google перевіряйте справжнім перекладом
через `--translate` з `"provider":"google"`.

## Мова перекладу

Поле `target_language` в `initialization_options` задає мову перекладу кодом BCP 47:
`"uk"`, `"en"`, `"de"`, `"pl"`, `"zh-Hans"`, `"zh-Hant"` тощо. Регістр і `_` замість `-`
не мають значення: `uk_UA` стає `uk-UA`.

```json
{
  "lsp": {
    "comment-translate": {
      "initialization_options": {
        "target_language": "de"
      }
    }
  }
}
```

Якщо поле відсутнє або дорівнює `"auto"`, мова береться із системи під час запуску LSP:

- macOS: перша мова зі списку **Системні параметри → Загальні → Мова й регіон**
  (`defaults read -g AppleLanguages`), потім змінні оточення;
- Linux: `LC_ALL`, `LC_MESSAGES`, `LANG` у порядку пріоритету POSIX; непорожній список
  `LANGUAGE` (наприклад `uk:en`) має перевагу, крім локалі `C`/`POSIX`;
- якщо мову не визначено (наприклад, `LANG=C.UTF-8`), використовується англійська.

Регіон локалі не впливає на переклад (`uk_UA.UTF-8` → `uk`), але для китайської
зберігається письмо: `zh_TW` → `zh-Hant`, `zh_CN` → `zh-Hans`. Вибрана мова та її джерело
записуються в журнал мовного сервера (у палітрі команд Zed — `open language server logs`).
Після зміни `target_language` або системної мови перезапустіть мовні сервери або Zed.
Заголовок підказки показує код мови: `Переклад → uk`. Повідомлення про помилки
залишаються українською.

## Моделі перекладу Apple (лише macOS)

На macOS 26.4+ перевірте, чи ввімкнено Apple Intelligence та завантажено його моделі:
`highFidelity` може працювати без окремих мовних пакетів. Apple може використати
традиційні моделі як заміну; назва стратегії не гарантує конкретну модель. Якщо жодна
стратегія не готова, завантажте вихідну та цільову мови через **Системні параметри →
Загальні → Мова й регіон → Мови перекладу**. Доступність перевіряється для конкретної
мовної пари. На macOS 26.0–26.3 використовуються звичайні мовні пакети.
Перевірка англійської → української:

```sh
echo '{"provider":"apple","operation":"check","source_language":"en","target_language":"uk"}' \
  | ./target/release/comment-translate-lsp --translate
```

Відповідь `"status":"installed"` означає готовність. Поле `strategy` показує вибрану
стратегію (`highFidelity` або `lowLatency`), а `checked_strategies` — результати виконаних
перевірок. Готова `highFidelity` використовується одразу, без перевірки `lowLatency`.
`supported` означає, що потрібні моделі ще не готові; `unsupported` — непідтримувану пару.
Перевірка має ліміт 15 секунд.
LSP не показує вікон завантаження. Якщо пакетів немає, hover пояснює,
що потрібно зробити. Завантаження пакетів відбувається через системний інтерфейс.

Перевірка тексту через вбудовану бібліотеку:

```sh
echo '{"provider":"apple","text":"Return the next counter value.","source_language":"en"}' \
  | ./target/release/comment-translate-lsp --translate
```

## Підключення до Zed

1. Виконайте `./scripts/build.sh`.
2. У палітрі Zed виконайте `zed: install dev extension` і виберіть корінь проєкту.
3. Додайте потрібні поля з [`examples/zed-settings.json`](examples/zed-settings.json)
   в особисті налаштування Zed. Перевірте абсолютний шлях до LSP.
4. Після зміни `initialization_options` перезапустіть мовні сервери або Zed.
5. Відкрийте файл підтримуваної мови, наведіть на коментар або поставте курсор усередину
   коментаря та викличте `editor: hover`. Приклад гарячої клавіші `ctrl-alt-t` наведено
   в [`examples/keymap.json`](examples/keymap.json).

Основні сервери мов (`clangd`, `rust-analyzer`, `basedpyright` тощо) працюють як раніше:
Zed запускає `comment-translate` поряд із ними через типове значення
`"language_servers": ["..."]` і об’єднує їхні hover. Якщо для мови задано власний список
`language_servers` без `"..."`, додайте до нього `"comment-translate"`.
Swift і Dart з’являються в Zed лише після встановлення розширень Swift і Dart.
Поза коментарями перекладач не викликається.
Для англомовного коду можна встановити `"source_language": "en"`: визначення мови
дуже коротких коментарів може бути неточним. Типове значення — `auto`.

## Заголовки C (`.h`)

Zed типово відкриває `.h` у режимі **C++**. Сервер зареєстровано і для C, і для C++,
тому коментарі в заголовках перекладаються без асоціації `file_types`. Якщо проєкт
перемикає `.h` на **C** через `file_types`, переклад теж працює.

## Перевірки

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build -p comment-translate-lsp
python3 scripts/smoke_test.py
./scripts/test_native.sh
```

Smoke test використовує справжній LSP і статичний Swift-міст з українським текстом,
який не потребує мовних моделей. Перевіряє `.c`, `.h` у режимах C і C++, Rust, Python,
Shell Script, ігнорування непідтримуваних мов, hover, редагування, закриття,
ліміти й завершення процесу. Rust-тести для кожної мови перевіряють типи вузлів коментарів,
маркери всередині рядків, групування рядків, docstring, shebang і UTF-16 координати. Rust-тести окремо перевіряють кешування, помилки, тайм-аут,
передачу Unicode та безпечне ігнорування пізнього callback після скасування.
`test_native.sh` перевіряє Swift та CLI лише для Apple-збірки на macOS. Спільні Rust-тести
перевіряють Google: кодування параметрів, JSON і HTTP-помилки. Два loopback HTTP-тести
позначені `ignored`, оскільки потребують дозволу створювати локальні TCP-сокети:

```sh
cargo test -p comment-translate-lsp -- --include-ignored
cargo test -p comment-translate-lsp --no-default-features
```

Linux CI запускає ці перевірки, збирає сервер і WASM, перевіряє CLI та LSP.
Звичайні тести не потребують інтернету чи мовних моделей. Справжній переклад потребує
готових моделей Apple або доступу до Google.

## Обмеження

Підтримуються C, C++, Rust, Python, Shell Script, Swift і Dart на macOS та Linux;
Apple-переклад потребує macOS 26+. Інші мови ігноруються: hover для них не з’являється.
Мова визначається за `languageId` від Zed, а якщо він невідомий — за розширенням файлу.
Сусідні рядкові коментарі (`//`, `///`, `//!` або `#`) без коду перед маркером
перекладаються одним блоком. Порожній рядок, код або блоковий коментар відокремлює блоки.
Порожній коментар `//` чи `#` усередині блоку зберігає розділення абзаців. Перед перекладом
рядки всередині абзацу з’єднуються пробілами; межі абзаців зберігаються як подвійний перенос.
Пункти списків (`1.`, `1)`, `-`, `*`, `+`, `•`) та Doxygen-теги (`@brief`, `@param`,
`@return`, також форма `\param`) залишаються окремими рядками. Перенесений опис пункту
з’єднується пробілами, а відступи вкладених пунктів зберігаються. Заголовки Markdown
(`# Examples`) залишаються окремими рядками, а блоки коду в огородженні ```` ``` ```` чи `~~~`
передаються дослівно, без з’єднання рядків. У вкладених коментарях Rust, Swift і Dart
прибираються лише зовнішні `/*` та `*/`. Docstring Python — лише перший рядковий літерал
модуля, класу чи функції без префіксів `f`/`b`/`t`; лапки й префікси `r`/`u` прибираються,
відступи вирівнюються як в `inspect.cleandoc`. Інші рядки, f-рядки, heredoc і shebang
не перекладаються. Усе це відбувається перед передачею тексту обраному перекладачу. Одночасно працює
один запит перекладу; запити й очікування черги обмежуються `timeout_seconds`.
За тайм-ауту запит Google скасовується в Rust; для Apple Rust від’єднує callback
і скасовує Swift-задачу та активну сесію. Системні
виклики можуть не реагувати на скасування відразу, тому Apple-міст утримує щонайбільше
8 незавершених задач і відхиляє нові до звільнення місця. Помилка нативної бібліотеки
тепер впливає на процес LSP, оскільки перекладач працює всередині нього.
Власної команди перекладу немає: використовується стандартний hover Zed, тому переклад
може запускатися й наведенням миші. Автоматичного встановлення нативних програм немає.

Документація: [розширення Zed](https://zed.dev/docs/extensions/developing-extensions),
[Apple Translation](https://developer.apple.com/documentation/translation),
[сесія для встановлених мов](https://developer.apple.com/documentation/translation/translationsession/init(installedsource:target:)).
