//! Thunderstore API access, slimmed model, and on-disk cache.

use anyhow::{Context, Result};
use serde::de::{IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Every package published to the Valheim community, all versions included.
const PACKAGE_API: &str = "https://thunderstore.io/c/valheim/api/v1/package/";

/// Valheim left early access as 1.0 alongside the Deep North update.
pub const V1_LAUNCH_DATE: &str = "2026-09-09";
/// The category authors tag when they mark a mod as current for 1.0.
pub const V1_CATEGORY: &str = "Deep North Update";

/// Cache lifetime before we re-fetch the package list.
const CACHE_TTL_SECS: u64 = 6 * 60 * 60;
const CACHE_FORMAT: u32 = 1;

/// A single mod, reduced to its latest version and the fields the TUI needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mod {
    /// "Owner-Name", the key dependencies refer to.
    pub full_name: String,
    pub name: String,
    pub owner: String,
    pub description: String,
    pub version: String,
    pub download_url: String,
    pub package_url: String,
    pub downloads: u64,
    pub rating: i64,
    pub file_size: u64,
    pub date_updated: String,
    pub categories: Vec<String>,
    /// Dependencies as "Owner-Name-Version" strings.
    pub dependencies: Vec<String>,
    pub deprecated: bool,
}

impl Mod {
    /// The author tagged this mod for the 1.0 / Deep North game version.
    pub fn tagged_for_v1(&self) -> bool {
        self.categories.iter().any(|c| c == V1_CATEGORY)
    }

    /// The mod saw a release on or after the 1.0 launch.
    pub fn updated_since_v1(&self) -> bool {
        self.date_updated.as_str() >= V1_LAUNCH_DATE
    }

    /// Thunderstore has no authoritative compatibility field, so a mod counts as
    /// 1.0-ready when it is still maintained and carries either positive signal.
    pub fn supports_v1(&self) -> bool {
        !self.deprecated && (self.tagged_for_v1() || self.updated_since_v1())
    }

    /// Short marker shown in the list explaining why the mod qualified.
    pub fn signal(&self) -> &'static str {
        match (self.tagged_for_v1(), self.updated_since_v1()) {
            (true, true) => "tagged + updated",
            (true, false) => "tagged",
            (false, true) => "updated",
            (false, false) => "-",
        }
    }

    pub fn size_human(&self) -> String {
        let b = self.file_size as f64;
        if b >= 1_048_576.0 {
            format!("{:.1} MB", b / 1_048_576.0)
        } else {
            format!("{:.0} KB", b / 1024.0)
        }
    }
}

// --- Raw API shapes -------------------------------------------------------

#[derive(Deserialize)]
struct RawPackage {
    full_name: String,
    name: String,
    owner: String,
    package_url: String,
    date_updated: String,
    rating_score: i64,
    is_deprecated: bool,
    categories: Vec<String>,
    /// Thunderstore returns versions newest-first; we keep only the first.
    versions: FirstVersion,
}

#[derive(Deserialize)]
struct RawVersion {
    description: String,
    version_number: String,
    dependencies: Vec<String>,
    download_url: String,
    downloads: u64,
    file_size: u64,
}

/// Deserializes a version array while retaining only its first element.
///
/// The full payload is ~160 MB because it carries every historical version of
/// every package. Discarding the tail during parsing instead of after keeps
/// peak memory low enough to run comfortably on a small VPS.
struct FirstVersion(Option<RawVersion>);

impl<'de> Deserialize<'de> for FirstVersion {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = FirstVersion;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an array of package versions")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<FirstVersion, A::Error> {
                let first = seq.next_element::<RawVersion>()?;
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(FirstVersion(first))
            }
        }
        d.deserialize_seq(V)
    }
}

// --- Cache ----------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct Cache {
    format: u32,
    fetched_at: u64,
    mods: Vec<Mod>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn cache_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("valheim-mods-tui")
        .join("packages.json")
}

/// Age of the cache in seconds, if one exists.
pub fn cache_age(path: &Path) -> Option<u64> {
    let raw = std::fs::read(path).ok()?;
    let cache: Cache = serde_json::from_slice(&raw).ok()?;
    Some(now_secs().saturating_sub(cache.fetched_at))
}

fn read_cache(path: &Path) -> Option<Vec<Mod>> {
    let raw = std::fs::read(path).ok()?;
    let cache: Cache = serde_json::from_slice(&raw).ok()?;
    if cache.format != CACHE_FORMAT || now_secs().saturating_sub(cache.fetched_at) > CACHE_TTL_SECS
    {
        return None;
    }
    Some(cache.mods)
}

fn write_cache(path: &Path, mods: &[Mod]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let cache = Cache {
        format: CACHE_FORMAT,
        fetched_at: now_secs(),
        mods: mods.to_vec(),
    };
    // Write-then-rename so a crash mid-write cannot leave a truncated cache.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&cache)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Loads the package list, preferring a fresh cache unless `force` is set.
pub fn load(force: bool) -> Result<Vec<Mod>> {
    let path = cache_path();
    if !force {
        if let Some(mods) = read_cache(&path) {
            return Ok(mods);
        }
    }
    let mods = fetch()?;
    // A cache write failure is not fatal; we already have the data in hand.
    let _ = write_cache(&path, &mods);
    Ok(mods)
}

/// Downloads and parses the full Valheim package index.
fn fetch() -> Result<Vec<Mod>> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("valheim-mods-tui")
        .gzip(true)
        // The catalogue is large, so the overall budget is generous; a dead
        // host still fails fast on the connect timeout rather than hanging.
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(20 * 60))
        .build()?;

    let resp = client
        .get(PACKAGE_API)
        .send()
        .context("requesting the Thunderstore package list")?
        .error_for_status()
        .context("Thunderstore returned an error status")?;

    // Parse straight off the socket so the 160 MB body is never held whole.
    // The buffer matters: serde_json pulls a byte at a time, so reading the
    // response unbuffered turns the parse into millions of tiny reads.
    let reader = std::io::BufReader::with_capacity(1 << 20, resp);
    let raw: Vec<RawPackage> =
        serde_json::from_reader(reader).context("parsing the Thunderstore package list")?;

    let mut mods: Vec<Mod> = raw
        .into_iter()
        .filter_map(|p| {
            let v = p.versions.0?;
            Some(Mod {
                full_name: p.full_name,
                name: p.name,
                owner: p.owner,
                description: v.description,
                version: v.version_number,
                download_url: v.download_url,
                package_url: p.package_url,
                downloads: v.downloads,
                rating: p.rating_score,
                file_size: v.file_size,
                date_updated: p.date_updated,
                categories: p.categories,
                dependencies: v.dependencies,
                deprecated: p.is_deprecated,
            })
        })
        .collect();

    mods.sort_by(|a, b| a.full_name.cmp(&b.full_name));
    Ok(mods)
}
