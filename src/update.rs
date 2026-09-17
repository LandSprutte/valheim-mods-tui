//! Self-update: replace this binary with a newer published build.

use anyhow::{anyhow, bail, Context, Result};
use std::path::Path;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The triple this binary was compiled for, recorded by build.rs.
pub const TARGET: &str = env!("BUILD_TARGET");

/// Where a replacement binary comes from.
#[derive(Debug, PartialEq, Eq)]
pub enum Source {
    /// A GitHub repository, given as "owner/repo"; its latest release is used.
    GitHub(String),
    /// A direct URL to a binary for this platform.
    Url(String),
}

/// Interprets an update source: a URL, or GitHub "owner/repo" shorthand.
pub fn parse_source(raw: &str) -> Result<Source> {
    let raw = raw.trim();
    if raw.starts_with("http://") || raw.starts_with("https://") {
        return Ok(Source::Url(raw.to_string()));
    }
    let parts: Vec<&str> = raw.split('/').collect();
    if parts.len() == 2 && parts.iter().all(|p| !p.is_empty()) {
        return Ok(Source::GitHub(raw.to_string()));
    }
    bail!("update source must be a URL or a GitHub \"owner/repo\", got {raw:?}")
}

/// Picks the release asset matching a target triple.
///
/// An exact triple match wins; otherwise the architecture and operating system
/// both have to appear in the name, so an arm64 build is never handed an x86
/// binary.
pub fn asset_for_target<'a>(assets: &'a [(String, String)], target: &str) -> Option<&'a (String, String)> {
    if let Some(hit) = assets.iter().find(|(name, _)| name.contains(target)) {
        return Some(hit);
    }
    let mut bits = target.split('-');
    let arch = bits.next()?;
    let os = if target.contains("linux") {
        "linux"
    } else if target.contains("apple") || target.contains("darwin") {
        "darwin"
    } else {
        return None;
    };
    assets.iter().find(|(name, _)| {
        let n = name.to_lowercase();
        n.contains(arch) && (n.contains(os) || (os == "darwin" && n.contains("apple")))
    })
}

/// True when `remote` is a different, higher version than `current`.
///
/// Anything unparseable falls back to "different means newer", so a build with
/// an odd version string still updates rather than refusing forever.
pub fn is_newer(current: &str, remote: &str) -> bool {
    let nums = |v: &str| -> Vec<u64> {
        v.trim_start_matches(['v', 'V'])
            .split(['.', '-', '+'])
            .map(|p| p.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let (a, b) = (nums(current), nums(remote));
    if a.is_empty() || b.is_empty() {
        return current != remote;
    }
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return y > x;
        }
    }
    false
}

/// Rejects payloads that are not a native executable for this platform.
///
/// Without this a rate-limit page or an HTML error would be written straight
/// over the binary.
pub fn looks_like_executable(bytes: &[u8]) -> bool {
    if bytes.len() < 4 {
        return false;
    }
    let magic = &bytes[..4];
    let elf = magic == [0x7f, b'E', b'L', b'F'];
    let macho = matches!(
        magic,
        [0xfe, 0xed, 0xfa, 0xcf] | [0xcf, 0xfa, 0xed, 0xfe] | [0xca, 0xfe, 0xba, 0xbe]
    );
    if cfg!(target_os = "linux") {
        elf
    } else if cfg!(target_os = "macos") {
        macho
    } else {
        elf || macho
    }
}

fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(format!("valheim-mods-tui/{VERSION}"))
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(10 * 60))
        .build()?)
}

/// Latest release tag and its assets, as (name, download URL) pairs.
fn github_release(
    client: &reqwest::blocking::Client,
    repo: &str,
) -> Result<(String, Vec<(String, String)>)> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let resp = client
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .with_context(|| format!("asking GitHub for the latest release of {repo}"))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        bail!("{repo} has no published releases (or the repository is private)");
    }
    let resp = resp.error_for_status().context("GitHub rejected the request")?;
    let body: serde_json::Value = resp.json().context("reading GitHub's response")?;

    let tag = body["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow!("release has no tag_name"))?
        .to_string();
    let assets = body["assets"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    Some((
                        x["name"].as_str()?.to_string(),
                        x["browser_download_url"].as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok((tag, assets))
}

/// Swaps the running binary for `bytes`.
///
/// The replacement is staged beside the target and renamed into place, which is
/// atomic and is also the only way to overwrite a running executable — writing
/// to it directly fails with ETXTBSY on Linux.
fn replace_binary(exe: &Path, bytes: &[u8]) -> Result<()> {
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow!("cannot locate the directory of {}", exe.display()))?;
    let staged = dir.join(format!(
        ".{}.update",
        exe.file_name().unwrap_or_default().to_string_lossy()
    ));

    let write = || -> Result<()> {
        std::fs::write(&staged, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_file(&staged);
        return Err(e).with_context(|| {
            format!(
                "staging the new binary in {} — if it is a system directory, re-run with sudo",
                dir.display()
            )
        });
    }

    // Refuse to install something that cannot run: far better to abort now than
    // to leave the user with a broken command.
    match std::process::Command::new(&staged).arg("--version").output() {
        Ok(out) if out.status.success() => {}
        Ok(out) => {
            let _ = std::fs::remove_file(&staged);
            bail!(
                "the downloaded binary failed to run ({}), keeping the current one",
                out.status
            );
        }
        Err(e) => {
            let _ = std::fs::remove_file(&staged);
            bail!("the downloaded binary could not be executed ({e}), keeping the current one");
        }
    }

    if let Err(e) = std::fs::rename(&staged, exe) {
        let _ = std::fs::remove_file(&staged);
        return Err(e).with_context(|| {
            format!(
                "replacing {} — if it is a system directory, re-run with sudo",
                exe.display()
            )
        });
    }
    Ok(())
}

/// Downloads and installs the newest published build.
pub fn run(source: &str, force: bool) -> Result<()> {
    let source = parse_source(source)?;
    let client = client()?;
    let exe = std::env::current_exe()
        .context("locating the running binary")?
        .canonicalize()
        .context("resolving the path of the running binary")?;

    println!("current version {VERSION} ({TARGET})");

    let (label, url) = match &source {
        Source::GitHub(repo) => {
            println!("checking {repo} for a newer release…");
            let (tag, assets) = github_release(&client, repo)?;
            if !force && !is_newer(VERSION, &tag) {
                println!("already up to date (latest release is {tag})");
                return Ok(());
            }
            let asset = asset_for_target(&assets, TARGET).ok_or_else(|| {
                anyhow!(
                    "release {tag} has no asset for {TARGET}; available: {}",
                    if assets.is_empty() {
                        "none".to_string()
                    } else {
                        assets
                            .iter()
                            .map(|(n, _)| n.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                )
            })?;
            (tag, asset.1.clone())
        }
        Source::Url(url) => ("the configured URL".to_string(), url.clone()),
    };

    println!("downloading {url}");
    let bytes = client
        .get(&url)
        .send()
        .context("downloading the new binary")?
        .error_for_status()
        .context("the download was rejected")?
        .bytes()
        .context("reading the downloaded binary")?;

    if !looks_like_executable(&bytes) {
        bail!(
            "that download is not an executable for {TARGET} ({} bytes) — check the update source",
            bytes.len()
        );
    }

    replace_binary(&exe, &bytes)?;
    println!(
        "updated to {label} — {} replaced ({:.1} MB)",
        exe.display(),
        bytes.len() as f64 / 1_048_576.0
    );
    Ok(())
}

/// Path used in messages when no source has been configured.
pub fn unconfigured_hint() -> String {
    format!(
        "no update source configured.\n\n\
         Set one once, then --update will use it:\n\
         \x20 valheim-mods-tui --update-source <owner/repo>      # GitHub releases\n\
         \x20 valheim-mods-tui --update-source <https://…/bin>   # any static host\n\n\
         This build is {VERSION} for {TARGET}."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_are_recognised_by_shape() {
        assert_eq!(
            parse_source("landsprutte/valheim-mods-tui").unwrap(),
            Source::GitHub("landsprutte/valheim-mods-tui".into())
        );
        assert_eq!(
            parse_source("https://files.example/vmt").unwrap(),
            Source::Url("https://files.example/vmt".into())
        );
        assert!(parse_source("not a source").is_err());
        assert!(parse_source("too/many/parts").is_err());
    }

    #[test]
    fn version_comparison_only_moves_forward() {
        assert!(is_newer("0.1.0", "0.2.0"));
        assert!(is_newer("0.1.0", "v0.1.1"));
        assert!(is_newer("0.9.0", "0.10.0"), "numeric, not lexical");
        assert!(!is_newer("0.2.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "v0.1.0"), "a v prefix is not a new version");
    }

    #[test]
    fn the_asset_matching_this_platform_is_chosen() {
        let assets = vec![
            ("vmt-aarch64-unknown-linux-musl".to_string(), "a".to_string()),
            ("vmt-x86_64-unknown-linux-musl".to_string(), "b".to_string()),
            ("vmt-aarch64-apple-darwin".to_string(), "c".to_string()),
        ];
        assert_eq!(
            asset_for_target(&assets, "x86_64-unknown-linux-musl").unwrap().1,
            "b"
        );
        assert_eq!(
            asset_for_target(&assets, "aarch64-apple-darwin").unwrap().1,
            "c"
        );
        // A gnu build still matches the musl asset for the same arch and OS.
        assert_eq!(
            asset_for_target(&assets, "aarch64-unknown-linux-gnu").unwrap().1,
            "a"
        );
    }

    /// Guards the contract with .github/workflows/release.yml, which names each
    /// asset "valheim-mods-tui-<triple>". If that naming drifts, --update
    /// silently stops finding builds.
    #[test]
    fn release_workflow_asset_names_resolve() {
        let published: Vec<(String, String)> = [
            "x86_64-unknown-linux-musl",
            "aarch64-unknown-linux-musl",
            "aarch64-apple-darwin",
        ]
        .iter()
        .map(|t| (format!("valheim-mods-tui-{t}"), format!("url/{t}")))
        .collect();

        for t in [
            "x86_64-unknown-linux-musl",
            "aarch64-unknown-linux-musl",
            "aarch64-apple-darwin",
        ] {
            let hit = asset_for_target(&published, t)
                .unwrap_or_else(|| panic!("no asset resolved for {t}"));
            assert_eq!(hit.0, format!("valheim-mods-tui-{t}"));
        }
        // The triple this build targets must resolve too.
        assert!(
            asset_for_target(&published, TARGET).is_some(),
            "no release asset would match this build's target {TARGET}"
        );
    }

    #[test]
    fn an_arch_mismatch_is_never_accepted() {
        let only_x86 = vec![("vmt-x86_64-unknown-linux-musl".to_string(), "b".to_string())];
        assert!(asset_for_target(&only_x86, "aarch64-unknown-linux-musl").is_none());
    }

    #[test]
    fn non_executable_downloads_are_rejected() {
        assert!(!looks_like_executable(b"<!DOCTYPE html><html>rate limited"));
        assert!(!looks_like_executable(b""));
        assert!(!looks_like_executable(b"\x7fEL"));
        #[cfg(target_os = "linux")]
        assert!(looks_like_executable(b"\x7fELF\x02\x01\x01"));
        #[cfg(target_os = "macos")]
        assert!(looks_like_executable(&[0xcf, 0xfa, 0xed, 0xfe, 0x0c]));
        // A binary for the wrong platform must not pass either.
        #[cfg(target_os = "linux")]
        assert!(!looks_like_executable(&[0xcf, 0xfa, 0xed, 0xfe, 0x0c]));
    }
}
