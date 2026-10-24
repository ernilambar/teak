use std::time::Duration;

const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/ernilambar/teak/releases/latest";

/// The command the update dialog copies, where the platform has an installer.
/// Only macOS has one; Linux and Windows ship bare archives.
#[cfg(target_os = "macos")]
pub(crate) const INSTALL_COMMAND: Option<&str> = Some(
    "curl -fsSL https://raw.githubusercontent.com/ernilambar/teak/main/scripts/install.sh | sh",
);
#[cfg(not(target_os = "macos"))]
pub(crate) const INSTALL_COMMAND: Option<&str> = None;

pub(crate) const RELEASES_URL: &str = "https://github.com/ernilambar/teak/releases/latest";

/// The line above the install hint in the update dialog.
#[cfg(target_os = "macos")]
pub(crate) const UPDATE_HINT: &str = "Save your work, then run this in Terminal:";
#[cfg(not(target_os = "macos"))]
pub(crate) const UPDATE_HINT: &str =
    "Download the latest release, then replace your Teak executable.";

/// The update dialog's confirm button.
#[cfg(target_os = "macos")]
pub(crate) const UPDATE_ACTION_LABEL: &str = "Copy Command";
#[cfg(not(target_os = "macos"))]
pub(crate) const UPDATE_ACTION_LABEL: &str = "Open Download Page";

const TIMEOUT: Duration = Duration::from_secs(10);

/// Returns the version of the latest GitHub release, without the leading `v`.
///
/// The request is made in-process so the check works on systems without a
/// `curl` binary, and bundles its own root certificates.
pub(crate) fn latest_version() -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .user_agent(concat!(
            env!("CARGO_PKG_NAME"),
            "/",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .into();

    let mut response = agent
        .get(LATEST_RELEASE_URL)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|error| error.to_string())?;

    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|error| error.to_string())?;

    parse_tag(body.as_bytes())
}

fn parse_tag(body: &[u8]) -> Result<String, String> {
    let release: serde_json::Value = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    let tag = release["tag_name"].as_str().ok_or("Release has no tag")?;
    Ok(tag.trim_start_matches('v').to_string())
}

/// Whether `latest` is a higher `X.Y.Z` version than `current`.
pub(crate) fn is_newer(latest: &str, current: &str) -> bool {
    fn parts(version: &str) -> Option<Vec<u64>> {
        version.split('.').map(|part| part.parse().ok()).collect()
    }

    match (parts(latest), parts(current)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tag_strips_leading_v() {
        let body = br#"{"tag_name": "v0.2.0", "name": "v0.2.0"}"#;
        assert_eq!(parse_tag(body).unwrap(), "0.2.0");
    }

    #[test]
    fn parse_tag_rejects_missing_tag() {
        assert!(parse_tag(br#"{"message": "Not Found"}"#).is_err());
    }

    #[test]
    fn is_newer_compares_numerically() {
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }
}
