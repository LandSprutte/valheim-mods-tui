//! Detecting which mods are already present in a Valheim install.

use crate::config::Layout;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// A mod found on disk.
#[derive(Debug, Clone)]
pub struct Installed {
    /// Version from the package manifest, when one was written.
    pub version: Option<String>,
    /// Which BepInEx directory it sits in.
    pub location: &'static str,
}

#[derive(Deserialize)]
struct Manifest {
    version_number: Option<String>,
}

/// The directories mods are installed into, each scoped one folder per mod.
const SCANNED: [&str; 3] = ["plugins", "patchers", "monomod"];

/// Scans a Valheim install for mods, keyed by Thunderstore full name.
///
/// Folder names are the key, which is how this tool installs them. Mods placed
/// by other managers may use a different naming scheme and simply will not be
/// recognised.
pub fn scan(layout: &Layout) -> HashMap<String, Installed> {
    let mut found = HashMap::new();

    for dir in SCANNED {
        let root = layout.bepinex.join(dir);
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            // A mod can ship both a plugin and a patcher; the first wins so the
            // reported location matches where its code actually lives.
            found.entry(name).or_insert_with(|| Installed {
                version: read_version(&entry.path()),
                location: dir,
            });
        }
    }

    // BepInEx has no folder of its own; its presence is the core assembly.
    if layout.bepinex.join("core").join("BepInEx.dll").is_file() {
        found
            .entry("denikson-BepInExPack_Valheim".to_string())
            .or_insert(Installed {
                version: None,
                location: "core",
            });
    }

    found
}

fn read_version(dir: &Path) -> Option<String> {
    let raw = std::fs::read(dir.join("manifest.json")).ok()?;
    // Some Thunderstore manifests are written with a UTF-8 BOM, which serde_json
    // rejects outright; without stripping it those mods report no version.
    let raw = raw.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&raw);
    let manifest: Manifest = serde_json::from_slice(raw).ok()?;
    manifest.version_number
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(tag: &str) -> Layout {
        let root = std::env::temp_dir().join(format!("vmt-installed-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let bepinex = root.join("BepInEx");

        let jotunn = bepinex.join("plugins").join("ValheimModding-Jotunn");
        std::fs::create_dir_all(&jotunn).unwrap();
        std::fs::write(
            jotunn.join("manifest.json"),
            br#"{"name":"Jotunn","version_number":"2.30.0"}"#,
        )
        .unwrap();

        // A mod installed without a manifest still counts as present.
        std::fs::create_dir_all(bepinex.join("plugins").join("Someone-NoManifest")).unwrap();
        // And a patcher lives in its own directory.
        std::fs::create_dir_all(bepinex.join("patchers").join("ASharpPen-This_Goes_Here")).unwrap();
        // A loose file beside the folders must not be mistaken for a mod.
        std::fs::write(bepinex.join("plugins").join("stray.dll"), b"x").unwrap();

        Layout {
            root: root.clone(),
            bepinex,
        }
    }

    #[test]
    fn mods_are_found_with_their_versions() {
        let layout = fixture("basic");
        let found = scan(&layout);

        assert_eq!(
            found["ValheimModding-Jotunn"].version.as_deref(),
            Some("2.30.0")
        );
        assert_eq!(found["ValheimModding-Jotunn"].location, "plugins");
        assert_eq!(found["ASharpPen-This_Goes_Here"].location, "patchers");
        assert!(found.contains_key("Someone-NoManifest"));
        assert_eq!(found["Someone-NoManifest"].version, None);
        assert!(!found.contains_key("stray.dll"), "files are not mods");
        assert_eq!(found.len(), 3);

        std::fs::remove_dir_all(&layout.root).ok();
    }

    /// Thunderstore manifests are not consistently BOM-free; Jotunn ships one.
    #[test]
    fn a_manifest_with_a_utf8_bom_still_yields_its_version() {
        let layout = fixture("bom");
        let dir = layout.bepinex.join("plugins").join("Owner-Bommed");
        std::fs::create_dir_all(&dir).unwrap();
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(br#"{"name":"Bommed","version_number":"1.4.2"}"#);
        std::fs::write(dir.join("manifest.json"), bytes).unwrap();

        assert_eq!(
            scan(&layout)["Owner-Bommed"].version.as_deref(),
            Some("1.4.2")
        );
        std::fs::remove_dir_all(&layout.root).ok();
    }

    #[test]
    fn bepinex_itself_counts_as_installed() {
        let layout = fixture("core");
        assert!(!scan(&layout).contains_key("denikson-BepInExPack_Valheim"));

        std::fs::create_dir_all(layout.bepinex.join("core")).unwrap();
        std::fs::write(layout.bepinex.join("core").join("BepInEx.dll"), b"x").unwrap();
        assert!(scan(&layout).contains_key("denikson-BepInExPack_Valheim"));

        std::fs::remove_dir_all(&layout.root).ok();
    }

    #[test]
    fn an_install_that_does_not_exist_yet_scans_clean() {
        let layout = Layout {
            root: std::env::temp_dir().join("vmt-nope"),
            bepinex: std::env::temp_dir().join("vmt-nope").join("BepInEx"),
        };
        assert!(scan(&layout).is_empty());
    }
}
