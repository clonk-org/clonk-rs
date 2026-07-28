//! Where an update is fetched from.
//!
//! Two different GitHub release URL shapes are in play, and mixing them up is
//! silently wrong rather than loudly broken, so they are separated here.

use crate::error::TransportError;
use clonk_update::{Manifest, TargetArchive};

/// The manifest endpoint: the only URL that may use the `latest` redirect.
pub const DEFAULT_UPDATE_BASE_URL: &str =
    "https://github.com/syb0rg/clonk-rs/releases/latest/download";

/// Release-tag download root. Component archives resolve under this, against
/// the tag of the release that actually published them.
pub const RELEASES_DOWNLOAD_BASE_URL: &str = "https://github.com/syb0rg/clonk-rs/releases/download";

pub const MANIFEST_FILE_NAME: &str = "manifest.json";

/// Hosts a release download may legitimately be redirected to.
///
/// `releases/latest/download/…` answers 302 towards GitHub's asset CDN, which
/// has been served under both `objects.` and `release-assets.` names. Nothing
/// else is a place an update may come from, so nothing else is accepted.
pub const ALLOWED_REDIRECT_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

/// The manifest URL under `base`, tolerating a trailing separator.
pub fn manifest_url(base: &str) -> String {
    format!("{}/{MANIFEST_FILE_NAME}", base.trim_end_matches('/'))
}

/// Where this build looks for the published manifest.
pub fn default_manifest_url() -> String {
    manifest_url(DEFAULT_UPDATE_BASE_URL)
}

/// Resolves a component archive against the release that published it.
///
/// Deliberately *not* `latest/download`: a component whose bytes did not change
/// is not republished, so its archive stays in an older release and `latest`
/// cannot reach it.
pub fn component_archive_url(version: &str, archive: &str) -> Result<String, TransportError> {
    let tag = release_tag(version)?;
    is_plain_asset_name(archive)
        .then(|| format!("{RELEASES_DOWNLOAD_BASE_URL}/{tag}/{archive}"))
        .ok_or_else(|| TransportError::UnsafeArchiveName(archive.to_owned()))
}

/// Resolves one manifest entry against the release the manifest describes.
pub fn archive_url_for(
    manifest: &Manifest,
    target: &TargetArchive,
) -> Result<String, TransportError> {
    component_archive_url(&manifest.version, &target.archive)
}

/// `v`-prefixed release tag, accepting a version that already carries one.
fn release_tag(version: &str) -> Result<String, TransportError> {
    let bare = version.strip_prefix('v').unwrap_or(version);
    is_plain_version(bare)
        .then(|| format!("v{bare}"))
        .ok_or_else(|| TransportError::UnsafeReleaseVersion(version.to_owned()))
}

/// A single path segment that needs no escaping and cannot leave its release.
///
/// Both inputs arrive inside a manifest fetched over the network, and both are
/// pasted straight into a URL, so neither may contain a separator, a dot-dot,
/// or anything a URL parser would reinterpret.
fn is_plain_asset_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.starts_with('.')
        && name
            .chars()
            .all(|part| part.is_ascii_alphanumeric() || matches!(part, '.' | '_' | '-'))
}

fn is_plain_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 64
        && version
            .chars()
            .all(|part| part.is_ascii_alphanumeric() || matches!(part, '.' | '_' | '-' | '+'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clonk_update::Manifest;

    #[test]
    fn the_manifest_is_fetched_through_the_latest_release_redirect() {
        assert_eq!(
            default_manifest_url(),
            "https://github.com/syb0rg/clonk-rs/releases/latest/download/manifest.json"
        );
    }

    #[test]
    fn a_component_archive_is_fetched_from_the_release_that_published_it() {
        // `latest/download` only ever resolves inside the newest release. A
        // component whose bytes did not change is not republished, so its
        // archive lives in an older release and is unreachable that way.
        assert_eq!(
            component_archive_url("0.4.0", "content-0123456789abcdef.zip").expect("plain name"),
            "https://github.com/syb0rg/clonk-rs/releases/download/v0.4.0/\
             content-0123456789abcdef.zip"
        );
    }

    #[test]
    fn a_version_that_already_carries_its_tag_prefix_is_not_doubled() {
        assert_eq!(
            component_archive_url("v0.4.0", "planet.zip").expect("plain name"),
            "https://github.com/syb0rg/clonk-rs/releases/download/v0.4.0/planet.zip"
        );
    }

    #[test]
    fn an_archive_name_that_could_leave_its_release_is_refused() {
        // The name arrives inside a manifest fetched over the network. Every
        // one of these still resolves to github.com, so the redirect allowlist
        // would not catch it — only rejecting the name does.
        for name in [
            "../../../other/repo/releases/download/v1/evil.zip",
            "sub/dir.zip",
            "back\\slash.zip",
            ".hidden.zip",
            "with space.zip",
            "query?.zip",
            "",
        ] {
            assert!(
                component_archive_url("0.4.0", name).is_err(),
                "archive name {name:?} must be refused"
            );
        }
    }

    #[test]
    fn an_ordinary_archive_name_is_accepted() {
        for name in [
            "content-0123456789abcdef.zip",
            "clonk-rust-0.4.0-engine-x86_64-pc-windows-gnu.zip",
            "planet_1.zip",
        ] {
            assert!(
                component_archive_url("0.4.0", name).is_ok(),
                "archive name {name:?} must be accepted"
            );
        }
    }

    #[test]
    fn a_manifest_entry_resolves_against_the_release_it_describes() {
        let manifest = Manifest::parse(MANIFEST.as_bytes()).expect("valid manifest");
        let engine = manifest.component("engine").expect("engine component");
        let target = engine
            .target_for("x86_64-unknown-linux-gnu")
            .expect("linux target");

        assert_eq!(
            archive_url_for(&manifest, target).expect("plain name"),
            "https://github.com/syb0rg/clonk-rs/releases/download/v0.4.0/\
             clonk-rust-0.4.0-engine-x86_64-unknown-linux-gnu.zip"
        );
    }

    #[test]
    fn a_custom_base_keeps_exactly_one_separator() {
        assert_eq!(
            manifest_url("https://mirror.example/u"),
            "https://mirror.example/u/manifest.json"
        );
        assert_eq!(
            manifest_url("https://mirror.example/u/"),
            "https://mirror.example/u/manifest.json"
        );
    }

    #[test]
    fn the_redirect_allowlist_covers_the_hosts_a_release_download_passes_through() {
        // `releases/latest/download/...` answers 302 to the asset CDN, which
        // GitHub has served under both names.
        assert!(ALLOWED_REDIRECT_HOSTS.contains(&"github.com"));
        assert!(ALLOWED_REDIRECT_HOSTS.contains(&"objects.githubusercontent.com"));
        assert!(ALLOWED_REDIRECT_HOSTS.contains(&"release-assets.githubusercontent.com"));
        assert!(!ALLOWED_REDIRECT_HOSTS.contains(&"githubusercontent.com"));
    }

    const MANIFEST: &str = r#"{
      "schema": 1,
      "version": "0.4.0",
      "engine_version": [4, 9, 11, 0, 362],
      "released_at": "2026-07-28T10:00:00Z",
      "components": [
        {
          "name": "engine",
          "targets": {
            "x86_64-unknown-linux-gnu": {
              "archive": "clonk-rust-0.4.0-engine-x86_64-unknown-linux-gnu.zip",
              "sha256": "aa00112233445566778899aabbccddeeff00112233445566778899aabbccddee",
              "size": 24000000,
              "install": ""
            }
          }
        }
      ]
    }"#;
}
