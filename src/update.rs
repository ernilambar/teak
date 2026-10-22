const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/ernilambar/tine/releases/latest";

pub(crate) const INSTALL_COMMAND: &str =
    "curl -fsSL https://raw.githubusercontent.com/ernilambar/tine/main/scripts/install.sh | sh";

/// Returns the version of the latest GitHub release, without the leading `v`.
pub(crate) fn latest_version() -> Result<String, String> {
    let output = std::process::Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "10",
            "-H",
            "Accept: application/vnd.github+json",
            LATEST_RELEASE_URL,
        ])
        .output()
        .map_err(|error| error.to_string())?;

    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        return Err(message.trim().trim_start_matches("curl: ").to_string());
    }

    parse_tag(&output.stdout)
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
