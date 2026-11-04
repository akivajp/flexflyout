//! Whether a newer release exists: reading GitHub's answer about the latest
//! release, and comparing versions. Fetching it is the host's job.

use serde::Deserialize;

/// Where to ask for the latest release (GitHub's REST API; drafts and
/// pre-releases are not "latest").
pub const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/akivajp/flexflyout/releases/latest";

/// A published release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// Its version, without the leading "v" of the tag ("0.6.0").
    pub version: String,
    /// Its page, with the notes and the downloads.
    pub url: String,
}

/// The part of GitHub's answer that is used.
#[derive(Deserialize)]
struct LatestRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// Reads GitHub's answer about the latest release. `None` when it is not
/// one (an error message, say), or a draft or a pre-release.
pub fn parse_latest(json: &str) -> Option<Release> {
    let latest: LatestRelease = serde_json::from_str(json).ok()?;
    if latest.draft || latest.prerelease {
        return None;
    }
    let version = latest.tag_name.trim().trim_start_matches('v').to_owned();
    parse_version(&version)?;
    // Only a page of this project is opened.
    if !latest
        .html_url
        .starts_with("https://github.com/akivajp/flexflyout/")
    {
        return None;
    }
    Some(Release {
        version,
        url: latest.html_url,
    })
}

/// Whether `latest` is a newer version than `current` ("0.5.0" style, an
/// optional leading "v"). A version that cannot be read is never newer.
pub fn is_newer(current: &str, latest: &str) -> bool {
    match (parse_version(current), parse_version(latest)) {
        (Some(current), Some(latest)) => latest > current,
        _ => false,
    }
}

/// "1.2.3" (or "v1.2.3") as numbers. Pre-release suffixes ("1.2.3-rc.1")
/// are not read: such versions are not offered.
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let mut next = || parts.next()?.parse::<u64>().ok();
    let version = (next()?, next()?, next()?);
    parts.next().is_none().then_some(version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_compared_by_number() {
        assert!(is_newer("0.5.0", "0.6.0"));
        assert!(is_newer("0.5.0", "v0.5.1"));
        assert!(is_newer("0.9.0", "0.10.0"));
        assert!(is_newer("0.5.3", "1.0.0"));
        assert!(!is_newer("0.5.0", "0.5.0"));
        assert!(!is_newer("0.6.0", "0.5.9"));
        // Never offered: what cannot be read.
        assert!(!is_newer("0.5.0", "0.6.0-rc.1"));
        assert!(!is_newer("0.5.0", "0.6"));
        assert!(!is_newer("0.5.0", "latest"));
        assert!(!is_newer("dev", "0.6.0"));
    }

    #[test]
    fn reads_the_latest_release() {
        let json = r#"{
            "url": "https://api.github.com/repos/akivajp/flexflyout/releases/1",
            "html_url": "https://github.com/akivajp/flexflyout/releases/tag/v0.6.0",
            "tag_name": "v0.6.0",
            "name": "v0.6.0",
            "draft": false,
            "prerelease": false,
            "assets": []
        }"#;
        assert_eq!(
            parse_latest(json),
            Some(Release {
                version: "0.6.0".into(),
                url: "https://github.com/akivajp/flexflyout/releases/tag/v0.6.0".into(),
            })
        );
    }

    #[test]
    fn ignores_what_is_not_a_published_release() {
        let release = |tag: &str, url: &str, prerelease: bool| {
            format!(r#"{{"tag_name":"{tag}","html_url":"{url}","prerelease":{prerelease}}}"#)
        };
        let page = "https://github.com/akivajp/flexflyout/releases/tag/v0.6.0";
        assert!(parse_latest(&release("v0.6.0", page, false)).is_some());
        assert_eq!(parse_latest(&release("v0.6.0", page, true)), None);
        assert_eq!(parse_latest(&release("nightly", page, false)), None);
        assert_eq!(
            parse_latest(&release("v0.6.0", "https://example.com/", false)),
            None
        );
        // GitHub's answer when there is no release, or too many requests.
        assert_eq!(parse_latest(r#"{"message":"Not Found"}"#), None);
        assert_eq!(parse_latest("<html>"), None);
    }
}
