use std::path::{Path, PathBuf};

// Only the Linux and Windows installers write into the user's home directory;
// the macOS one installs into `/usr/local/bin`.
#[cfg(not(target_os = "macos"))]
use crate::paths::home_dir;

/// What the installed command-line tool launches: the `.app` bundle on macOS,
/// the executable itself elsewhere.
#[cfg(target_os = "macos")]
pub(crate) fn launcher_target() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .ancestors()
        .find(|path| path.extension().is_some_and(|ext| ext == "app"))
        .map(Path::to_path_buf)
}

/// What the installed command-line tool launches: the `.app` bundle on macOS,
/// the executable itself elsewhere.
#[cfg(not(target_os = "macos"))]
pub(crate) fn launcher_target() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// The launcher script for a macOS `.app` bundle: options go to the binary
/// inside the bundle, and everything else opens the app.
#[cfg(any(target_os = "macos", test))]
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

/// The launcher script for a Linux executable: the binary already handles
/// options and paths, so the script only forwards its arguments.
#[cfg(any(all(unix, not(target_os = "macos")), test))]
fn linux_launcher_script(executable: &Path) -> String {
    format!(
        "#!/bin/sh\n# Command-line launcher installed by the app.\nexec \"{}\" \"$@\"\n",
        executable.display()
    )
}

/// The launcher script for a Windows executable: a `.cmd` shim that forwards
/// its arguments to the binary.
#[cfg(any(target_os = "windows", test))]
fn windows_launcher_script(executable: &Path) -> String {
    format!(
        "@echo off\r\nrem Command-line launcher installed by the app.\r\n\"{}\" %*\r\n",
        executable.display()
    )
}

/// Installs the command-line tool. On macOS this needs an administrator
/// prompt to write into `/usr/local/bin`; on Linux and Windows the command goes
/// into a directory the user owns.
#[cfg(target_os = "macos")]
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

/// Installs the command-line tool into `~/.local/bin`, which needs no
/// administrator rights. The directory is created when missing.
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) fn install_cli(executable: &Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt as _;

    let target = home_dir()
        .ok_or("Could not find the home directory")?
        .join(".local")
        .join("bin")
        .join("teak");

    let directory = target
        .parent()
        .ok_or("Could not find the install directory")?;
    std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;

    // Stage beside the target so the rename never crosses filesystems, which
    // fails with `EXDEV` when `/tmp` is a separate mount.
    let temp = directory.join(format!(".teak-{}", std::process::id()));
    std::fs::write(&temp, linux_launcher_script(executable)).map_err(|error| error.to_string())?;
    std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o755))
        .map_err(|error| error.to_string())?;

    if let Err(error) = std::fs::rename(&temp, &target) {
        let _ = std::fs::remove_file(&temp);
        return Err(error.to_string());
    }

    Ok(target)
}

/// Installs the command-line tool as a `.cmd` shim in
/// `%LOCALAPPDATA%\Microsoft\WindowsApps`, a directory on `PATH` that the user
/// owns, so no administrator rights are needed.
#[cfg(target_os = "windows")]
pub(crate) fn install_cli(executable: &Path) -> Result<PathBuf, String> {
    let target = home_dir()
        .ok_or("Could not find the home directory")?
        .join("AppData")
        .join("Local")
        .join("Microsoft")
        .join("WindowsApps")
        .join("teak.cmd");

    if let Some(directory) = target.parent() {
        std::fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    }
    std::fs::write(&target, windows_launcher_script(executable))
        .map_err(|error| error.to_string())?;

    Ok(target)
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

    #[test]
    fn linux_launcher_script_forwards_to_the_executable() {
        let script = linux_launcher_script(Path::new("/opt/example/example"));
        assert!(script.starts_with("#!/bin/sh\n"));
        assert!(script.contains(r#"exec "/opt/example/example" "$@""#));
    }

    #[test]
    fn windows_launcher_script_shims_to_the_executable() {
        let script = windows_launcher_script(Path::new(r"C:\Program Files\Example\example.exe"));
        assert!(script.contains(r#""C:\Program Files\Example\example.exe" %*"#));
        assert!(script.contains("\r\n"), "the shim keeps CRLF line endings");
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
