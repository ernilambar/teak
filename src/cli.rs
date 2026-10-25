use std::path::{Path, PathBuf};

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

# Options are handled by the binary itself; `open` would treat them as files.
for arg in "$@"; do
    case "$arg" in
        --) break ;;
        -*) exec "$APP/Contents/MacOS/teak" "$@" ;;
    esac
done

if [ "${1:-}" = "--" ]; then
    shift
fi

if [ "$#" -eq 0 ]; then
    exec open -a "$APP"
fi

exec open -a "$APP" -- "$@"
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
    Run,
    Help,
    Version,
    Unknown(String),
}

/// Scans arguments (without the program name) for options. The first option
/// wins; everything after `--` is treated as a path.
pub(crate) fn parse_args(args: impl IntoIterator<Item = String>) -> CliCommand {
    for arg in args {
        match arg.as_str() {
            "--" => break,
            "-h" | "--help" => return CliCommand::Help,
            "-V" | "--version" => return CliCommand::Version,
            // Older macOS passes a process serial number when launched from Finder.
            _ if arg.starts_with("-psn_") => {}
            _ if arg.starts_with('-') => return CliCommand::Unknown(arg),
            _ => {}
        }
    }
    CliCommand::Run
}

pub(crate) fn print_help() {
    println!(
        "\
{name} {version}
A fast, lightweight text editor

Usage:
  {name} [OPTIONS] [FOLDER]...

Arguments:
  [FOLDER]...  Add the given folders to the sidebar

Options:
  -h, --help     Print this help and exit
  -V, --version  Print version and exit",
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
        assert!(!script.contains("__BUNDLE__"));
    }

    fn parse(args: &[&str]) -> CliCommand {
        parse_args(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn parse_args_detects_options() {
        assert_eq!(parse(&[]), CliCommand::Run);
        assert_eq!(parse(&["notes"]), CliCommand::Run);
        assert_eq!(parse(&["--help"]), CliCommand::Help);
        assert_eq!(parse(&["-V"]), CliCommand::Version);
        assert_eq!(parse(&["notes", "--version"]), CliCommand::Version);
        assert_eq!(
            parse(&["--versioin"]),
            CliCommand::Unknown("--versioin".to_string())
        );
        assert_eq!(parse(&["--", "--version"]), CliCommand::Run);
        assert_eq!(parse(&["-psn_0_12345"]), CliCommand::Run);
    }
}
