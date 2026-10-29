//! Seti file icons.
//!
//! The SVGs are vendored from Seti UI (MIT); see `assets/file-icons/LICENSE`
//! and `scripts/vendor-seti-icons.sh`. The name and extension associations are
//! Teak's own: Seti publishes its associations as a VS Code font plus
//! `languageIds`, not as an extension map that could be copied.

use std::path::Path;

macro_rules! icon {
    ($name:literal) => {
        include_bytes!(concat!("../assets/file-icons/", $name, ".svg")) as &[u8]
    };
}

const DEFAULT_FILE: &[u8] = icon!("default");
const DEFAULT_FOLDER: &[u8] = icon!("folder");

pub(crate) fn folder_icon() -> &'static [u8] {
    DEFAULT_FOLDER
}

/// Whether `bytes` paints itself. Seti's icons are half brand-colored and half
/// bare paths meant to be tinted, so the painter needs to tell them apart: an
/// SVG with no `fill` renders black, which disappears on a dark sidebar.
pub(crate) fn has_own_color(bytes: &[u8]) -> bool {
    const MARKERS: [&[u8]; 3] = [b"fill=\"#", b"fill:#", b"stop-color=\"#"];
    MARKERS
        .iter()
        .any(|marker| bytes.windows(marker.len()).any(|window| window == *marker))
}

pub(crate) fn file_icon(path: &Path) -> &'static [u8] {
    let by_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_ascii_lowercase)
        .and_then(|name| icon_for_name(&name));
    if let Some(icon) = by_name {
        return icon;
    }

    let by_extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .and_then(|ext| icon_for_extension(&ext));
    by_extension.unwrap_or(DEFAULT_FILE)
}

fn icon_for_name(name: &str) -> Option<&'static [u8]> {
    let icon: &'static [u8] = match name {
        ".gitignore" | ".gitignore_global" | ".gitattributes" | ".gitmodules" | ".gitconfig" => {
            icon!("git")
        }
        ".editorconfig" => icon!("editorconfig"),
        ".npmrc" => icon!("npm"),
        "package.json" | "package-lock.json" | "npm-shrinkwrap.json" => icon!("npm"),
        "tsconfig.json" => icon!("tsconfig"),
        ".babelrc" | ".babelrc.js" | ".babelrc.cjs" | "babel.config.js" | "babel.config.cjs"
        | "babel.config.json" => icon!("babel"),
        "vite.config.js" | "vite.config.ts" | "vite.config.mjs" | "vite.config.mts"
        | "vite.config.cjs" | "vite.config.cts" => icon!("vite"),
        "webpack.config.js"
        | "webpack.config.ts"
        | "webpack.config.mjs"
        | "webpack.config.cjs"
        | "webpack.config.babel.js" => icon!("webpack"),
        "makefile" | "makefile.am" | "makefile.in" | "gnumakefile" => icon!("makefile"),
        "settings.json" => icon!("settings"),
        "yarn.lock" => icon!("yarn"),
        "bower.json" | ".bowerrc" => icon!("bower"),
        "firebase.json" | ".firebaserc" => icon!("firebase"),
        ".envrc" => icon!("config"),
        ".bashrc" | ".bash_profile" | ".bash_login" | ".bash_aliases" | ".profile" | ".zshrc"
        | ".zprofile" | ".zshenv" | ".zlogin" | ".kshrc" | ".cshrc" | ".tcshrc" | ".inputrc" => {
            icon!("shell")
        }
        _ => {
            if name.starts_with("dockerfile") || name.starts_with("docker-compose") {
                return Some(icon!("docker"));
            }
            if name.starts_with("license") || name.starts_with("licence") {
                return Some(icon!("license"));
            }
            if name.starts_with(".eslintrc") || name.starts_with("eslint.config.") {
                return Some(icon!("eslint"));
            }
            if name.starts_with(".prettierrc") || name.starts_with("prettier.config.") {
                return Some(icon!("config"));
            }
            if name.starts_with(".env") {
                return Some(icon!("config"));
            }
            return None;
        }
    };
    Some(icon)
}

fn icon_for_extension(ext: &str) -> Option<&'static [u8]> {
    let icon: &'static [u8] = match ext {
        "rs" => icon!("rust"),
        "js" | "mjs" | "cjs" => icon!("javascript"),
        "jsx" => icon!("react"),
        "ts" | "mts" | "cts" => icon!("typescript"),
        "tsx" => icon!("react"),
        "vue" => icon!("vue"),
        "svelte" => icon!("svelte"),
        "py" | "pyi" | "pyw" => icon!("python"),
        "php" | "phtml" | "php3" | "php4" | "php5" => icon!("php"),
        "html" | "htm" | "razor" | "cshtml" => icon!("html"),
        "css" => icon!("css"),
        "scss" | "sass" => icon!("sass"),
        "less" => icon!("less"),
        "styl" | "stylus" => icon!("stylus"),
        "coffee" => icon!("coffee"),
        "ls" => icon!("livescript"),
        "c" | "h" => icon!("c"),
        "cpp" | "cc" | "cxx" | "c++" | "cp" | "hpp" | "hh" | "hxx" | "h++" => icon!("cpp"),
        "cs" | "csx" => icon!("c-sharp"),
        "java" => icon!("java"),
        "kt" | "kts" => icon!("kotlin"),
        "swift" => icon!("swift"),
        "go" => icon!("go"),
        "rb" | "rake" | "gemspec" => icon!("ruby"),
        "lua" | "luau" => icon!("lua"),
        "dart" => icon!("dart"),
        "scala" | "sc" => icon!("scala"),
        "ex" | "exs" => icon!("elixir"),
        "hs" => icon!("haskell"),
        "clj" | "cljs" | "cljc" | "edn" => icon!("clojure"),
        "elm" => icon!("elm"),
        "nim" | "nims" => icon!("nim"),
        "zig" | "zon" => icon!("zig"),
        "jl" => icon!("julia"),
        "r" => icon!("r"),
        "asm" | "s" => icon!("asm"),
        "ml" | "mli" => icon!("ocaml"),
        "fs" | "fsi" | "fsx" | "fsscript" => icon!("f-sharp"),
        "purs" => icon!("purescript"),
        "re" | "rei" => icon!("reasonml"),
        "res" | "resi" => icon!("rescript"),
        "graphql" | "gql" => icon!("graphql"),
        "prisma" => icon!("prisma"),
        "tex" | "latex" | "sty" | "cls" => icon!("tex"),
        "tf" | "tfvars" | "tfstate" => icon!("terraform"),
        "nginx" | "nginxconf" => icon!("config"),
        "sh" | "bash" | "zsh" | "ksh" | "fish" | "ash" => icon!("shell"),
        "ps1" | "psm1" | "psd1" => icon!("powershell"),
        "ipynb" => icon!("notebook"),
        "j2" | "jinja" | "jinja2" => icon!("jinja"),
        "pug" => icon!("pug"),
        "jade" => icon!("jade"),
        "ejs" => icon!("ejs"),
        "twig" => icon!("twig"),
        "haml" => icon!("haml"),
        "json" | "jsonc" | "json5" | "jsonl" => icon!("json"),
        "yaml" | "yml" => icon!("yml"),
        "toml" => icon!("config"),
        "xml" | "xsd" | "xsl" | "xslt" | "dtd" | "plist" => icon!("xml"),
        "md" | "markdown" | "mdx" => icon!("markdown"),
        "sql" | "db" | "sqlite" | "sqlite3" | "mysql" | "pgsql" => icon!("db"),
        "csv" | "tsv" => icon!("csv"),
        "lock" | "key" | "pem" | "p12" | "pfx" | "crt" | "cer" | "der" => icon!("lock"),
        "svg" => icon!("svg"),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff" | "avif" => {
            icon!("image")
        }
        "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" => icon!("audio"),
        "mp4" | "mov" | "avi" | "mkv" | "webm" | "m4v" => icon!("video"),
        "ttf" | "otf" | "woff" | "woff2" | "eot" => icon!("font"),
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "rar" | "7z" | "zst" => icon!("zip"),
        "pdf" => icon!("pdf"),
        "doc" | "docx" | "odt" | "rtf" | "pages" => icon!("word"),
        "wasm" => icon!("wasm"),
        "wat" => icon!("wat"),
        _ => return None,
    };
    Some(icon)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_known_extensions() {
        for (file, expected) in [
            ("main.rs", icon!("rust")),
            ("app.js", icon!("javascript")),
            ("app.mjs", icon!("javascript")),
            ("component.jsx", icon!("react")),
            ("app.ts", icon!("typescript")),
            ("view.tsx", icon!("react")),
            ("index.html", icon!("html")),
            ("style.css", icon!("css")),
            ("style.scss", icon!("sass")),
            ("script.py", icon!("python")),
            ("index.php", icon!("php")),
            ("data.json", icon!("json")),
            ("config.yaml", icon!("yml")),
            ("Cargo.toml", icon!("config")),
            ("Cargo.lock", icon!("lock")),
            ("notes.txt", DEFAULT_FILE),
            ("photo.png", icon!("image")),
            ("archive.zip", icon!("zip")),
            ("program.wasm", icon!("wasm")),
        ] {
            assert_eq!(file_icon(Path::new(file)), expected, "{file}");
        }
    }

    #[test]
    fn matches_special_file_names_before_extension() {
        for (file, expected) in [
            ("package.json", icon!("npm")),
            ("package-lock.json", icon!("npm")),
            ("tsconfig.json", icon!("tsconfig")),
            (".gitignore", icon!("git")),
            (".editorconfig", icon!("editorconfig")),
            ("Dockerfile", icon!("docker")),
            ("Dockerfile.prod", icon!("docker")),
            ("docker-compose.yml", icon!("docker")),
            ("Makefile", icon!("makefile")),
            ("LICENSE", icon!("license")),
            (".npmrc", icon!("npm")),
            (".env", icon!("config")),
            (".bashrc", icon!("shell")),
            ("yarn.lock", icon!("yarn")),
        ] {
            assert_eq!(file_icon(Path::new(file)), expected, "{file}");
        }
    }

    #[test]
    fn falls_back_to_default_file_icon() {
        for file in ["notes.unknownext", "Makefile.unknown", "noextension"] {
            assert_eq!(file_icon(Path::new(file)), DEFAULT_FILE, "{file}");
        }
    }

    #[test]
    fn tells_colored_icons_from_bare_ones() {
        // Seti's Go icon paints itself; its Rust icon has no `fill` and relies
        // on the theme tint, so the painter must not rasterize it as-is.
        assert!(has_own_color(icon!("go")));
        assert!(!has_own_color(icon!("rust")));
        assert!(!has_own_color(icon!("default")));
    }
}
