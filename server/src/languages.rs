//! Таблиця мов, коментарі яких перекладає сервер.

use tower_lsp::lsp_types::Url;

/// Опис синтаксису коментарів однієї мови.
pub struct LanguageSpec {
    /// Значення `languageId`, які надсилає клієнт LSP.
    pub ids: &'static [&'static str],
    /// Суфікси імен файлів для випадку, коли `languageId` невідомий.
    pub extensions: &'static [&'static str],
    /// Граматика Tree-sitter.
    pub grammar: fn() -> tree_sitter::Language,
    /// Типи вузлів Tree-sitter, які є коментарями.
    pub comment_kinds: &'static [&'static str],
    /// Маркери однорядкових коментарів.
    pub line_markers: &'static [&'static str],
    /// Початок і кінець блокового коментаря.
    pub block: Option<(&'static str, &'static str)>,
    /// Чи вважати docstring Python коментарем.
    pub docstrings: bool,
    /// Чи пропускати shebang (`#!` на початку файлу).
    pub shebang: bool,
    /// Мова блоків коду в документації Markdown; `None`, якщо документація мови не є Markdown.
    pub markdown_fence: Option<&'static str>,
}

const SLASHES: &[&str] = &["//"];
const HASH: &[&str] = &["#"];
const C_BLOCK: Option<(&str, &str)> = Some(("/*", "*/"));

pub static C: LanguageSpec = LanguageSpec {
    ids: &["c"],
    extensions: &["c", "h"],
    grammar: || tree_sitter_c::LANGUAGE.into(),
    comment_kinds: &["comment"],
    line_markers: SLASHES,
    block: C_BLOCK,
    docstrings: false,
    shebang: false,
    markdown_fence: None,
};

pub static CPP: LanguageSpec = LanguageSpec {
    // Без мапи `language_ids` Zed надсилає назву мови в нижньому регістрі, тобто `c++`.
    ids: &["cpp", "c++"],
    extensions: &[
        "cc", "ccm", "hh", "cpp", "cppm", "hpp", "cxx", "cxxm", "hxx", "c++", "c++m", "h++", "hip",
        "ipp", "inl", "ino", "ixx", "cu", "cuh", "C", "H",
    ],
    grammar: || tree_sitter_cpp::LANGUAGE.into(),
    comment_kinds: &["comment"],
    line_markers: SLASHES,
    block: C_BLOCK,
    docstrings: false,
    shebang: false,
    markdown_fence: None,
};

pub static RUST: LanguageSpec = LanguageSpec {
    ids: &["rust"],
    extensions: &["rs"],
    grammar: || tree_sitter_rust::LANGUAGE.into(),
    // Документаційні коментарі є дочірніми вузлами `doc_comment` цих двох типів.
    comment_kinds: &["line_comment", "block_comment"],
    line_markers: SLASHES,
    block: C_BLOCK,
    docstrings: false,
    shebang: false,
    markdown_fence: Some("rust"),
};

pub static PYTHON: LanguageSpec = LanguageSpec {
    ids: &["python"],
    extensions: &["py", "pyi", "pyw", "mpy"],
    grammar: || tree_sitter_python::LANGUAGE.into(),
    comment_kinds: &["comment"],
    line_markers: HASH,
    block: None,
    docstrings: true,
    shebang: true,
    markdown_fence: None,
};

pub static BASH: LanguageSpec = LanguageSpec {
    // Zed називає мову «Shell Script»; без мапи `language_ids` надходить `shell script`.
    ids: &["shellscript", "shell script", "bash", "sh", "zsh"],
    extensions: &[
        "sh",
        "bash",
        "bashrc",
        "bash_profile",
        "bash_aliases",
        "bash_login",
        "bash_logout",
        "bats",
        "envrc",
        "profile",
        "zsh",
        "zshrc",
        "zshenv",
        "zprofile",
        "zlogin",
        "PKGBUILD",
        "APKBUILD",
        "ebuild",
    ],
    grammar: || tree_sitter_bash::LANGUAGE.into(),
    comment_kinds: &["comment"],
    line_markers: HASH,
    block: None,
    docstrings: false,
    shebang: true,
    markdown_fence: None,
};

pub static SWIFT: LanguageSpec = LanguageSpec {
    ids: &["swift"],
    extensions: &["swift", "swiftinterface"],
    grammar: || tree_sitter_swift::LANGUAGE.into(),
    comment_kinds: &["comment", "multiline_comment"],
    line_markers: SLASHES,
    block: C_BLOCK,
    docstrings: false,
    shebang: false,
    markdown_fence: Some("swift"),
};

pub static DART: LanguageSpec = LanguageSpec {
    ids: &["dart"],
    extensions: &["dart"],
    grammar: || tree_sitter_dart::LANGUAGE.into(),
    // `block_comment` і `documentation_block_comment` вкладені в ці вузли.
    comment_kinds: &["comment", "documentation_comment"],
    line_markers: SLASHES,
    block: C_BLOCK,
    docstrings: false,
    shebang: false,
    markdown_fence: Some("dart"),
};

/// Усі підтримувані мови; C стоїть перед C++, тому `.h` без `languageId` вважається C.
pub static ALL: [&LanguageSpec; 7] = [&C, &CPP, &RUST, &PYTHON, &BASH, &SWIFT, &DART];

/// Визначає мову за `languageId`, а якщо він невідомий — за ім’ям файлу з URI.
pub fn resolve(language_id: &str, uri: &Url) -> Option<&'static LanguageSpec> {
    if let Some(spec) = ALL.iter().find(|spec| spec.ids.contains(&language_id)) {
        return Some(spec);
    }
    let name = uri.path().rsplit('/').next()?;
    ALL.iter()
        .find(|spec| {
            spec.extensions.iter().any(|suffix| {
                name == *suffix
                    || name
                        .strip_suffix(suffix)
                        .is_some_and(|stem| stem.ends_with('.'))
            })
        })
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(language_id: &str, path: &str) -> Option<&'static LanguageSpec> {
        resolve(
            language_id,
            &Url::parse(&format!("file:///project/{path}")).unwrap(),
        )
    }

    #[test]
    fn language_ids_take_priority_over_file_names() {
        assert!(std::ptr::eq(resolved("c", "main.rs").unwrap(), &C));
        assert!(std::ptr::eq(resolved("cpp", "demo.h").unwrap(), &CPP));
        assert!(std::ptr::eq(resolved("c++", "demo.h").unwrap(), &CPP));
        assert!(std::ptr::eq(resolved("rust", "x").unwrap(), &RUST));
        assert!(std::ptr::eq(resolved("python", "x").unwrap(), &PYTHON));
        assert!(std::ptr::eq(resolved("shellscript", "x").unwrap(), &BASH));
        assert!(std::ptr::eq(resolved("shell script", "x").unwrap(), &BASH));
        assert!(std::ptr::eq(resolved("swift", "x").unwrap(), &SWIFT));
        assert!(std::ptr::eq(resolved("dart", "x").unwrap(), &DART));
    }

    #[test]
    fn unknown_language_ids_fall_back_to_file_names() {
        for (path, expected) in [
            ("demo.c", &C),
            ("demo.h", &C),
            ("demo.hpp", &CPP),
            ("demo.C", &CPP),
            ("lib.rs", &RUST),
            ("tool.pyi", &PYTHON),
            ("build.sh", &BASH),
            (".bashrc", &BASH),
            ("PKGBUILD", &BASH),
            ("App.swift", &SWIFT),
            ("main.dart", &DART),
        ] {
            let spec = resolved("unknown", path).unwrap();
            assert!(std::ptr::eq(spec, expected), "{path}");
        }
    }

    #[test]
    fn unsupported_documents_are_ignored() {
        assert!(resolved("markdown", "README.md").is_none());
        assert!(resolved("plaintext", "notes").is_none());
        assert!(resolved("plaintext", "rs").is_some_and(|spec| std::ptr::eq(spec, &RUST)));
        assert!(resolved("plaintext", "xrs").is_none());
    }

    #[test]
    fn every_grammar_loads_and_declares_its_comment_kinds() {
        for spec in ALL {
            let language = (spec.grammar)();
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&language).unwrap();
            for kind in spec.comment_kinds {
                assert_ne!(language.id_for_node_kind(kind, true), 0, "{kind}");
            }
        }
    }
}
