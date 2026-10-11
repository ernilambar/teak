use std::path::Path;

use gpui_kit::SharedString;

pub(crate) const LANGUAGES: [&str; 12] = [
    "Plain Text",
    "Rust",
    "TOML",
    "JSON",
    "Markdown",
    "JavaScript",
    "TypeScript",
    "Python",
    "HTML",
    "CSS",
    "PHP",
    "SVG",
];

pub(crate) fn language_for(path: &Path) -> SharedString {
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("rs") => "Rust".into(),
        Some("toml") => "TOML".into(),
        Some("json") => "JSON".into(),
        Some("md") => "Markdown".into(),
        Some("js") => "JavaScript".into(),
        Some("ts") => "TypeScript".into(),
        Some("py") => "Python".into(),
        Some("html") | Some("htm") => "HTML".into(),
        Some("css") | Some("scss") => "CSS".into(),
        Some("php") | Some("phtml") => "PHP".into(),
        Some("svg") => "SVG".into(),
        _ => "Plain Text".into(),
    }
}

pub(crate) fn editor_language_id(language: &str) -> &'static str {
    match language {
        "Rust" => "rust",
        "TOML" => "toml",
        "JSON" => "json",
        "Markdown" => "markdown",
        "JavaScript" => "javascript",
        "TypeScript" => "typescript",
        "Python" => "python",
        "HTML" => "html",
        "CSS" => "css",
        "PHP" => "php",
        // No XML grammar is bundled; HTML highlights SVG markup well enough.
        "SVG" => "html",
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
        ];
        for (file, language) in cases {
            assert_eq!(language_for(Path::new(file)), language, "{file}");
        }
    }

    #[test]
    fn language_for_ignores_extension_case() {
        for (file, language) in [
            ("MAIN.RS", "Rust"),
            ("Index.HTML", "HTML"),
            ("a.Md", "Markdown"),
        ] {
            assert_eq!(language_for(Path::new(file)), language, "{file}");
        }
    }

    #[test]
    fn language_for_falls_back_to_plain_text() {
        for file in ["notes.txt", "Makefile", ".gitignore", "archive.tar.gz"] {
            assert_eq!(language_for(Path::new(file)), "Plain Text", "{file}");
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
