//! Reading a web address, for the update check.

use std::time::Duration;

/// The most that is read (GitHub's answer about a release is a few KB).
const MAX_BYTES: u64 = 1 << 20;

/// Reads `url` as text. Fails unless the server answers 200.
pub fn get(url: &str) -> anyhow::Result<String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .user_agent(crate::update::user_agent())
        .build()
        .new_agent();
    let text = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .call()?
        .body_mut()
        .with_config()
        .limit(MAX_BYTES)
        .read_to_string()?;
    Ok(text)
}
