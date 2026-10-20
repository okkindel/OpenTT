//! The update check: once per windowed start, a background thread asks
//! GitHub for the latest release; a newer one gets a notice in the main
//! menu (`MenuBackend::update_available`) with a button to the releases
//! page. It runs the system's `curl` (Windows 10+, macOS and Linux have
//! it), so the game needs no HTTP library; without `curl`, offline or on
//! any error there is simply no notice. `TT_NO_UPDATE_CHECK` (any value)
//! turns it off, and runs under cargo (`cargo run`, the tests) skip it.

use std::sync::OnceLock;

/// The project's page (the workspace's `repository`).
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
/// Where the notice's button leads.
pub const RELEASES: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/releases");

static NEWER: OnceLock<String> = OnceLock::new();

/// Starts the check in the background (returns at once).
pub fn start() {
    if std::env::var_os("TT_NO_UPDATE_CHECK").is_some() || std::env::var_os("CARGO_MANIFEST_DIR").is_some() {
        return;
    }
    std::thread::spawn(|| {
        if let Some(tag) = fetch().and_then(|json| newer(&json, env!("CARGO_PKG_VERSION"))) {
            let _ = NEWER.set(tag);
        }
    });
}

/// The newer release's tag ("v0.2.0"), once the check found one.
pub fn available() -> Option<&'static str> {
    NEWER.get().map(String::as_str)
}

/// The latest release's JSON (5 s at most).
fn fetch() -> Option<String> {
    let api = format!("https://api.github.com/repos/{}/releases/latest", REPOSITORY.trim_start_matches("https://github.com/"));
    let mut cmd = std::process::Command::new("curl");
    cmd.args(["-fsSL", "--max-time", "5", "-H", "Accept: application/vnd.github+json", &api]).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000); // CREATE_NO_WINDOW
    let out = cmd.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The release's `tag_name` when it is a newer version than `ours`.
pub fn newer(json: &str, ours: &str) -> Option<String> {
    let tag = json.split_once("\"tag_name\"")?.1.split('"').nth(1)?;
    (version(tag)? > version(ours)?).then(|| tag.to_string())
}

/// (major, minor, patch) of "v1.2.3" / "1.2" / "1.2.3-rc1" (the
/// pre-release suffix is ignored).
fn version(v: &str) -> Option<(u64, u64, u64)> {
    let mut n = v.trim().trim_start_matches(['v', 'V']).split(['.', '-', '+']).map(|s| s.parse::<u64>().ok());
    Some((n.next()??, n.next()??, n.next().flatten().unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_compares_the_tag_with_our_version() {
        let json = |tag: &str| format!(r#"{{"url":"x","tag_name": "{tag}","name":"OpenTT {tag}"}}"#);
        assert_eq!(newer(&json("v0.1.3"), "0.1.2").as_deref(), Some("v0.1.3"));
        assert_eq!(newer(&json("v0.10.0"), "0.9.9").as_deref(), Some("v0.10.0"));
        assert_eq!(newer(&json("1.0"), "0.1.2").as_deref(), Some("1.0"));
        assert_eq!(newer(&json("v0.1.2"), "0.1.2"), None);
        assert_eq!(newer(&json("v0.1.1"), "0.1.2"), None);
        assert_eq!(newer(&json("v0.1.2-rc1"), "0.1.2"), None);
        assert_eq!(newer(&json("nightly"), "0.1.2"), None);
        assert_eq!(newer(r#"{"message":"API rate limit exceeded"}"#, "0.1.2"), None);
        assert_eq!(newer("", "0.1.2"), None);
    }

    #[test]
    fn the_releases_page_is_the_repository_s() {
        assert!(RELEASES.starts_with("https://github.com/") && RELEASES.ends_with("/releases"), "{RELEASES}");
    }
}
