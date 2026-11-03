use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use crate::paths::hex_digit;

/// What the installed command-line tool launches: the `.app` bundle.
pub(crate) fn launcher_target() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .ancestors()
        .find(|path| path.extension().is_some_and(|ext| ext == "app"))
        .map(Path::to_path_buf)
}

/// The launcher script for a macOS `.app` bundle: options go to the binary
/// inside the bundle, and everything else opens the app.
fn mac_launcher_script(bundle: &Path) -> String {
    let template = r#"#!/usr/bin/env bash
# Command-line launcher installed by the app.
set -euo pipefail

APP="${TEAK_APP:-__BUNDLE__}"

# Help, version and unknown options go to the binary. `--new-window` is
# separated out so it can also reach an already-running app; everything else is
# a path for `open`.
new_window=0
separator=0
paths=()
for arg in "$@"; do
    if [ "$separator" = 1 ]; then
        paths+=("$arg")
        continue
    fi
    case "$arg" in
        --) separator=1 ;;
        -h|--help|-V|--version) exec "$APP/Contents/MacOS/teak" "$@" ;;
        -n|--new-window) new_window=1 ;;
        -*) exec "$APP/Contents/MacOS/teak" "$@" ;;
        *) paths+=("$arg") ;;
    esac
done

# `open` starts the app with `/` as its working directory, so a relative path
# is resolved here, against the directory the command was run from.
absolute() {
    case "$1" in
        /*) printf '%s\n' "$1" ;;
        *)
            local dir
            if dir=$(cd -- "$(dirname -- "$1")" 2>/dev/null && pwd); then
                printf '%s/%s\n' "$dir" "$(basename -- "$1")"
            else
                printf '%s/%s\n' "$PWD" "$1"
            fi
            ;;
    esac
}
if [ "${#paths[@]}" -gt 0 ]; then
    for i in "${!paths[@]}"; do
        paths[i]=$(absolute "${paths[i]}")
    done
fi

# A running app cannot see argv, so a URL carries the paths through
# LaunchServices. A cold launch uses argv, which also skips nothing: the new
# window joins the restored layout just as it does while running.
if [ "$new_window" = 1 ]; then
    if pgrep -f "$APP/Contents/MacOS/teak" >/dev/null 2>&1; then
        payload=""
        if [ "${#paths[@]}" -gt 0 ]; then
            payload=$(printf '%s\0' "${paths[@]}" | od -An -tx1 | tr -d ' \n')
        fi
        exec open "teak://new-window?payload=$payload"
    fi
    if [ "${#paths[@]}" -gt 0 ]; then
        exec open -a "$APP" --args --new-window "${paths[@]}"
    fi
    exec open -a "$APP" --args --new-window
fi

if [ "${#paths[@]}" -eq 0 ]; then
    exec open -a "$APP"
fi
exec open -a "$APP" -- "${paths[@]}"
"#;
    template.replace("__BUNDLE__", &bundle.display().to_string())
}

/// Installs the command-line tool. This needs an administrator prompt to write
/// into `/usr/local/bin`.
pub(crate) fn install_cli(bundle: &Path) -> Result<PathBuf, String> {
    const TARGET: &str = "/usr/local/bin/teak";

    let temp = std::env::temp_dir().join(format!("cli-launcher-{}", std::process::id()));
    std::fs::write(&temp, mac_launcher_script(bundle)).map_err(|error| error.to_string())?;

    let command = format!(
        "mkdir -p /usr/local/bin && install -m 755 \"{}\" {TARGET}",
        temp.display()
    );
    let script = format!(
        "do shell script \"{}\" with administrator privileges",
        command.replace('"', "\\\"")
    );

    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map_err(|error| error.to_string())?;

    let _ = std::fs::remove_file(&temp);

    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        let message = message.trim();
        let message = message.strip_prefix("execution error: ").unwrap_or(message);
        if message.contains("User canceled") || message.contains("-128") {
            return Err("Installation cancelled".to_string());
        }
        return Err(message.to_string());
    }

    Ok(PathBuf::from(TARGET))
}

#[derive(Debug, PartialEq)]
pub(crate) enum CliCommand {
    Run { new_window: bool },
    Help,
    Version,
    Unknown(String),
}

/// Scans arguments (without the program name) for options. The first option
/// wins; everything after `--` is treated as a path.
pub(crate) fn parse_args(args: impl IntoIterator<Item = String>) -> CliCommand {
    let mut new_window = false;
    for arg in args {
        match arg.as_str() {
            "--" => break,
            "-h" | "--help" => return CliCommand::Help,
            "-V" | "--version" => return CliCommand::Version,
            "-n" | "--new-window" => new_window = true,
            // Older macOS passes a process serial number when launched from Finder.
            _ if arg.starts_with("-psn_") => {}
            _ if arg.starts_with('-') => return CliCommand::Unknown(arg),
            _ => {}
        }
    }
    CliCommand::Run { new_window }
}

/// Parses the URL the launcher builds for an already-running app:
/// `teak://new-window?payload=<hex of NUL-separated paths>`.
pub(crate) fn parse_new_window_url(url: &str) -> Option<Vec<PathBuf>> {
    let rest = url.strip_prefix("teak://new-window")?;
    if rest.is_empty() {
        return Some(Vec::new());
    }
    if !(rest.starts_with('?') || rest.starts_with('/')) {
        return None;
    }
    let rest = rest.trim_start_matches('/');
    if rest.is_empty() {
        return Some(Vec::new());
    }
    let payload = rest.strip_prefix("?payload=")?;
    let payload = payload.split('&').next().unwrap_or(payload);
    let bytes = decode_hex(payload)?;
    Some(
        bytes
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| PathBuf::from(OsString::from_vec(path.to_vec())))
            .collect(),
    )
}

fn decode_hex(input: &str) -> Option<Vec<u8>> {
    let bytes = input.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return None;
    }
    let mut decoded = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.as_chunks::<2>().0 {
        let high = hex_digit(pair[0])?;
        let low = hex_digit(pair[1])?;
        decoded.push((high << 4) | low);
    }
    Some(decoded)
}

pub(crate) fn print_help() {
    println!(
        "\
{name} {version}
A fast, lightweight text editor

Usage:
  {name} [OPTIONS] [PATH]...

Arguments:
  [PATH]...  Folders to open in the sidebar, files to open in tabs, or a
             .teak-workspace file whose folders become the workspace

Options:
  -n, --new-window  Open the given paths in a new window
  -h, --help        Print this help and exit
  -V, --version     Print version and exit",
        name = env!("CARGO_PKG_NAME"),
        version = env!("CARGO_PKG_VERSION"),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_launcher_script_embeds_bundle_path() {
        let script = mac_launcher_script(Path::new("/Applications/Example.app"));
        assert!(script.starts_with("#!/usr/bin/env bash\n"));
        assert!(script.contains(r#"APP="${TEAK_APP:-/Applications/Example.app}""#));
        assert!(script.contains("teak://new-window"));
        assert!(script.contains("--args --new-window"));
        assert!(script.contains("pgrep -f"));
        assert!(!script.contains("__BUNDLE__"));
    }

    fn parse(args: &[&str]) -> CliCommand {
        parse_args(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn parse_args_detects_options() {
        assert_eq!(parse(&[]), CliCommand::Run { new_window: false });
        assert_eq!(parse(&["notes"]), CliCommand::Run { new_window: false });
        assert_eq!(parse(&["--help"]), CliCommand::Help);
        assert_eq!(parse(&["-V"]), CliCommand::Version);
        assert_eq!(parse(&["notes", "--version"]), CliCommand::Version);
        assert_eq!(
            parse(&["--new-window", "notes"]),
            CliCommand::Run { new_window: true }
        );
        assert_eq!(
            parse(&["-n", "notes"]),
            CliCommand::Run { new_window: true }
        );
        assert_eq!(
            parse(&["--versioin"]),
            CliCommand::Unknown("--versioin".to_string())
        );
        assert_eq!(
            parse(&["--", "--version"]),
            CliCommand::Run { new_window: false }
        );
        assert_eq!(
            parse(&["-psn_0_12345"]),
            CliCommand::Run { new_window: false }
        );
    }

    /// Runs the launcher with a stand-in `open` that records its arguments, from
    /// `cwd`, and returns what `open` was given.
    fn run_launcher(cwd: &Path, args: &[&str]) -> Vec<String> {
        let bin = cwd.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = cwd.join("open.log");
        let open = bin.join("open");
        std::fs::write(
            &open,
            format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n", log.display()),
        )
        .unwrap();
        std::fs::write(bin.join("pgrep"), "#!/bin/sh\nexit 1\n").unwrap();
        for tool in ["open", "pgrep"] {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(bin.join(tool), std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        let script = cwd.join("teak.sh");
        std::fs::write(
            &script,
            mac_launcher_script(Path::new("/Applications/Example.app")),
        )
        .unwrap();
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
        let status = std::process::Command::new("bash")
            .arg(&script)
            .args(args)
            .current_dir(cwd)
            .env("PATH", path)
            .env_remove("PWD")
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::read_to_string(log)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }

    /// `open` starts the app in `/`, so the launcher hands it absolute paths,
    /// resolved against the directory the command ran in.
    #[test]
    fn the_launcher_makes_relative_paths_absolute() {
        let dir = std::env::temp_dir().join(format!("teak-launcher-rel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("notes")).unwrap();
        std::fs::write(dir.join("notes/a.txt"), "").unwrap();

        let given = run_launcher(&dir, &["-n", "notes/a.txt", "/abs/b.txt", "new.txt"]);

        assert_eq!(&given[..3], ["-a", "/Applications/Example.app", "--args"]);
        assert_eq!(given[3], "--new-window");
        let real = dir.canonicalize().unwrap();
        assert_eq!(
            PathBuf::from(&given[4]).canonicalize().unwrap(),
            real.join("notes/a.txt")
        );
        assert_eq!(given[5], "/abs/b.txt", "an absolute path is left alone");
        assert!(Path::new(&given[6]).is_absolute(), "a file not made yet");
        assert!(given[6].ends_with("/new.txt"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn mac_launcher_script_is_valid_bash() {
        let script = mac_launcher_script(Path::new("/Applications/Example.app"));
        let path = std::env::temp_dir().join(format!(
            "teak-launcher-{}-{}.sh",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        std::fs::write(&path, script).unwrap();
        let status = std::process::Command::new("bash")
            .arg("-n")
            .arg(&path)
            .status()
            .unwrap();
        let _ = std::fs::remove_file(path);
        assert!(status.success(), "the launcher script must parse as bash");
    }

    #[test]
    fn parse_new_window_url_decodes_hex_paths() {
        assert_eq!(parse_new_window_url("teak://new-window"), Some(Vec::new()));
        assert_eq!(
            parse_new_window_url("teak://new-window?payload="),
            Some(Vec::new())
        );
        // /tmp\0/tmp/a\0
        assert_eq!(
            parse_new_window_url("teak://new-window?payload=2f746d70002f746d702f6100"),
            Some(vec![PathBuf::from("/tmp"), PathBuf::from("/tmp/a")])
        );
        assert_eq!(
            parse_new_window_url("teak://new-window/?payload=2f746d70"),
            Some(vec![PathBuf::from("/tmp")])
        );
        assert_eq!(parse_new_window_url("https://example.com"), None);
        assert_eq!(parse_new_window_url("teak://new-window?payload=zz"), None);
    }
}
