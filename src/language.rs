use std::path::Path;

use gpui_kit::SharedString;

pub(crate) const LANGUAGES: [&str; 16] = [
    "Plain Text",
    "Rust",
    "TOML",
    "JSON",
    "YAML",
    "Markdown",
    "JavaScript",
    "TypeScript",
    "TSX",
    "Python",
    "Shell",
    "HTML",
    "XML",
    "CSS",
    "PHP",
    "SVG",
];

/// The language for a file, from its name, then its extension, then a
/// shebang on `first_line` (for scripts with no extension).
pub(crate) fn language_for(path: &Path, first_line: &str) -> SharedString {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    match name {
        ".bashrc" | ".bash_profile" | ".zshrc" | ".zprofile" | ".zshenv" | ".profile" | ".env" => {
            return "Shell".into();
        }
        "Cargo.lock" => return "TOML".into(),
        _ if name.starts_with(".env.") => return "Shell".into(),
        _ => {}
    }

    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("rs") => "Rust".into(),
        Some("toml") => "TOML".into(),
        Some("json") | Some("jsonc") => "JSON".into(),
        Some("yml") | Some("yaml") => "YAML".into(),
        Some("md") | Some("markdown") => "Markdown".into(),
        Some("js") | Some("jsx") | Some("mjs") | Some("cjs") => "JavaScript".into(),
        Some("ts") | Some("mts") | Some("cts") => "TypeScript".into(),
        Some("tsx") => "TSX".into(),
        Some("py") | Some("pyi") => "Python".into(),
        Some("sh") | Some("bash") | Some("zsh") => "Shell".into(),
        Some("html") | Some("htm") => "HTML".into(),
        Some("xml") | Some("plist") => "XML".into(),
        Some("css") | Some("scss") => "CSS".into(),
        Some("php") | Some("phtml") => "PHP".into(),
        Some("svg") => "SVG".into(),
        _ => shebang_language(first_line).unwrap_or("Plain Text").into(),
    }
}

/// The language named by a `#!` line: `#!/bin/bash`, `#!/usr/bin/env python3`.
fn shebang_language(first_line: &str) -> Option<&'static str> {
    let mut words = first_line.strip_prefix("#!")?.split_whitespace();
    let mut program = words.next()?.rsplit('/').next()?;
    if program == "env" {
        program = words.find(|word| !word.starts_with('-'))?;
    }
    // `python3.12` and `python3` are both Python.
    let program = program.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    match program {
        "sh" | "bash" | "zsh" | "dash" | "ksh" => Some("Shell"),
        "python" => Some("Python"),
        "node" | "deno" | "bun" => Some("JavaScript"),
        "php" => Some("PHP"),
        _ => None,
    }
}

pub(crate) fn editor_language_id(language: &str) -> &'static str {
    match language {
        "Rust" => "rust",
        "TOML" => "toml",
        "JSON" => "json",
        "YAML" => "yaml",
        "Markdown" => "markdown",
        "JavaScript" => "javascript",
        "TypeScript" => "typescript",
        "TSX" => "tsx",
        "Python" => "python",
        "Shell" => "bash",
        "HTML" => "html",
        // No XML grammar is bundled; HTML highlights XML and SVG markup well enough.
        "XML" | "SVG" => "html",
        "CSS" => "css",
        "PHP" => "php",
        _ => "plaintext",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_for_maps_known_extensions() {
        let cases = [
            ("main.rs", "Rust"),
            ("Cargo.toml", "TOML"),
            ("data.json", "JSON"),
            ("README.md", "Markdown"),
            ("app.js", "JavaScript"),
            ("app.ts", "TypeScript"),
            ("script.py", "Python"),
            ("index.html", "HTML"),
            ("index.htm", "HTML"),
            ("style.css", "CSS"),
            ("style.scss", "CSS"),
            ("index.php", "PHP"),
            ("view.phtml", "PHP"),
            ("icon.svg", "SVG"),
            ("config.yml", "YAML"),
            ("config.yaml", "YAML"),
            ("tsconfig.jsonc", "JSON"),
            ("App.jsx", "JavaScript"),
            ("module.mjs", "JavaScript"),
            ("config.cjs", "JavaScript"),
            ("module.mts", "TypeScript"),
            ("App.tsx", "TSX"),
            ("stubs.pyi", "Python"),
            ("build.sh", "Shell"),
            ("Info.plist", "XML"),
            ("feed.xml", "XML"),
            ("NOTES.markdown", "Markdown"),
        ];
        for (file, language) in cases {
            assert_eq!(language_for(Path::new(file), ""), language, "{file}");
        }
    }

    #[test]
    fn language_for_ignores_extension_case() {
        for (file, language) in [
            ("MAIN.RS", "Rust"),
            ("Index.HTML", "HTML"),
            ("a.Md", "Markdown"),
        ] {
            assert_eq!(language_for(Path::new(file), ""), language, "{file}");
        }
    }

    #[test]
    fn language_for_maps_known_file_names() {
        for (file, language) in [
            (".zshrc", "Shell"),
            ("/Users/me/.bashrc", "Shell"),
            (".env", "Shell"),
            (".env.local", "Shell"),
            ("Cargo.lock", "TOML"),
        ] {
            assert_eq!(language_for(Path::new(file), ""), language, "{file}");
        }
    }

    #[test]
    fn language_for_reads_a_shebang_when_the_name_says_nothing() {
        for (line, language) in [
            ("#!/bin/sh", "Shell"),
            ("#!/usr/bin/env bash", "Shell"),
            ("#!/usr/bin/env -S python3 -u", "Python"),
            ("#!/usr/local/bin/python3.12", "Python"),
            ("#!/usr/bin/env node", "JavaScript"),
            ("#!/usr/bin/env ruby", "Plain Text"),
            ("echo no shebang", "Plain Text"),
        ] {
            assert_eq!(language_for(Path::new("script"), line), language, "{line}");
        }
        // The extension wins over the shebang.
        assert_eq!(language_for(Path::new("a.py"), "#!/bin/sh"), "Python");
    }

    #[test]
    fn language_for_falls_back_to_plain_text() {
        for file in ["notes.txt", "Makefile", ".gitignore", "archive.tar.gz"] {
            assert_eq!(language_for(Path::new(file), ""), "Plain Text", "{file}");
        }
    }

    #[test]
    fn every_listed_language_has_a_compiled_grammar() {
        use gpui_kit::component::highlighter::Language;
        // An id whose gpui-kit feature is off silently falls back to plain text.
        for language in LANGUAGES.iter().skip(1) {
            let id = editor_language_id(language);
            assert_ne!(Language::from_str(id).name(), "text", "{language}");
        }
    }

    #[test]
    fn every_listed_language_has_an_editor_id() {
        for language in LANGUAGES.iter().skip(1) {
            assert_ne!(editor_language_id(language), "plaintext", "{language}");
        }
        assert_eq!(editor_language_id("Plain Text"), "plaintext");
        assert_eq!(editor_language_id("Unknown"), "plaintext");
    }
}
