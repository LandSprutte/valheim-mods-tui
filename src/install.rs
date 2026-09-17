//! Downloading Thunderstore archives and unpacking them into BepInEx.

use crate::config::Layout;
use crate::thunderstore::Mod;
use anyhow::{bail, Context, Result};
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

/// Where an archive entry belongs once unpacked.
enum Target {
    /// Relative to the server root, used by the BepInEx framework package.
    Root(PathBuf),
    /// Relative to the BepInEx directory.
    BepInEx(PathBuf),
    Skip,
}

pub struct Report {
    pub written: usize,
    pub skipped_config: usize,
}

/// Downloads a mod and unpacks it into the configured Valheim install.
pub fn install(
    client: &reqwest::blocking::Client,
    m: &Mod,
    layout: &Layout,
    log: &dyn Fn(String),
) -> Result<Report> {
    log(format!("downloading {} {}", m.full_name, m.version));

    let bytes = client
        .get(&m.download_url)
        .send()
        .with_context(|| format!("downloading {}", m.full_name))?
        .error_for_status()
        .with_context(|| format!("download rejected for {}", m.full_name))?
        .bytes()
        .with_context(|| format!("reading the archive for {}", m.full_name))?;

    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
        .with_context(|| format!("{} is not a readable zip archive", m.full_name))?;

    // The BepInEx pack is a framework, not a plugin: its payload unpacks over
    // the server root rather than into BepInEx/plugins.
    let framework_prefix = framework_prefix(&mut zip);
    let mut report = Report {
        written: 0,
        skipped_config: 0,
    };

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        // `mangled_name` alone still permits absolute paths on some archives,
        // so the path is validated explicitly before anything is written.
        let name = entry.name().replace('\\', "/");
        let rel = match safe_relative(&name) {
            Some(p) => p,
            None => {
                log(format!("  skipped unsafe archive path: {name}"));
                continue;
            }
        };

        let target = route(&rel, &m.full_name, framework_prefix.as_deref());
        let dest = match target {
            Target::Skip => continue,
            Target::Root(p) => layout.root.join(p),
            Target::BepInEx(p) => layout.bepinex.join(p),
        };

        // Existing configs hold the user's tuning; a reinstall must not clobber
        // them, so they are left alone and reported.
        let is_config = dest.starts_with(layout.bepinex.join("config"));
        if is_config && dest.exists() {
            report.skipped_config += 1;
            continue;
        }

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        // Trust the archive's declared size only up to a sane bound, so a
        // malformed or hostile header cannot trigger a huge allocation.
        let hint = entry.size().min(8 * 1024 * 1024) as usize;
        let mut buf = Vec::with_capacity(hint);
        let mode = entry.unix_mode();
        entry.read_to_end(&mut buf)?;
        std::fs::write(&dest, &buf).with_context(|| format!("writing {}", dest.display()))?;

        #[cfg(unix)]
        if let Some(mode) = executable_mode(mode, &dest) {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(mode));
        }
        #[cfg(not(unix))]
        let _ = mode;

        report.written += 1;
    }

    // Nothing written is only an error when nothing was deliberately kept
    // either — a config-only package on a reinstall is a legitimate no-op.
    if report.written == 0 && report.skipped_config == 0 {
        bail!("{} contained no installable files", m.full_name);
    }
    Ok(report)
}

/// The mode to apply to an extracted file, or `None` to leave the default.
///
/// Archive permissions are honoured when they carry an executable bit. BepInEx
/// additionally ships its Linux launch scripts as plain 0666, so `.sh` files
/// are made runnable regardless — an unrunnable start script is useless.
#[cfg(unix)]
fn executable_mode(archive_mode: Option<u32>, dest: &Path) -> Option<u32> {
    if let Some(mode) = archive_mode {
        if mode & 0o111 != 0 {
            return Some(mode & 0o777);
        }
    }
    let is_script = dest
        .extension()
        .map(|e| e.eq_ignore_ascii_case("sh"))
        .unwrap_or(false);
    is_script.then_some(0o755)
}

/// Detects the BepInEx framework pack and returns the prefix to strip.
fn framework_prefix<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>) -> Option<String> {
    for i in 0..zip.len() {
        let name = match zip.by_index(i) {
            Ok(e) => e.name().replace('\\', "/"),
            Err(_) => continue,
        };
        if let Some((first, _)) = name.split_once('/') {
            if first.starts_with("BepInExPack") {
                return Some(format!("{first}/"));
            }
        }
    }
    None
}

/// Rejects absolute paths and parent-directory escapes (zip-slip).
fn safe_relative(name: &str) -> Option<PathBuf> {
    let path = Path::new(name);
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Maps an archive entry to its destination, following Thunderstore's
/// conventional package layout.
fn route(rel: &Path, full_name: &str, framework: Option<&str>) -> Target {
    let unix = rel.to_string_lossy().replace('\\', "/");

    if let Some(prefix) = framework {
        return match unix.strip_prefix(prefix) {
            // The pack's own metadata is not part of the game install.
            Some(rest) if !rest.is_empty() => Target::Root(PathBuf::from(rest)),
            _ => Target::Skip,
        };
    }

    let (head, rest) = match unix.split_once('/') {
        Some((h, r)) => (h.to_lowercase(), r.to_string()),
        None => (String::new(), unix.clone()),
    };

    // Directories BepInEx owns are merged in place; everything else is scoped
    // to a per-mod folder so installs stay separable.
    match head.as_str() {
        "plugins" => Target::BepInEx(Path::new("plugins").join(full_name).join(rest)),
        "patchers" => Target::BepInEx(Path::new("patchers").join(full_name).join(rest)),
        "monomod" => Target::BepInEx(Path::new("monomod").join(full_name).join(rest)),
        "core" => Target::BepInEx(Path::new("core").join(rest)),
        "config" => Target::BepInEx(Path::new("config").join(rest)),
        _ => Target::BepInEx(Path::new("plugins").join(full_name).join(&unix)),
    }
}

pub fn http_client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent("valheim-mods-tui")
        .connect_timeout(std::time::Duration::from_secs(30))
        .timeout(std::time::Duration::from_secs(15 * 60))
        .build()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(name: &str) -> Target {
        route(&PathBuf::from(name), "Author-Mod", None)
    }

    fn bepinex_of(t: Target) -> String {
        match t {
            Target::BepInEx(p) => p.to_string_lossy().replace('\\', "/"),
            Target::Root(p) => panic!("expected BepInEx target, got root {}", p.display()),
            Target::Skip => panic!("expected BepInEx target, got skip"),
        }
    }

    #[test]
    fn plugins_folder_is_scoped_per_mod() {
        assert_eq!(
            bepinex_of(r("plugins/Mod.dll")),
            "plugins/Author-Mod/Mod.dll"
        );
    }

    #[test]
    fn loose_dll_at_archive_root_lands_in_the_mod_folder() {
        assert_eq!(bepinex_of(r("Mod.dll")), "plugins/Author-Mod/Mod.dll");
    }

    #[test]
    fn nested_assets_keep_their_structure() {
        assert_eq!(
            bepinex_of(r("plugins/assets/icon.png")),
            "plugins/Author-Mod/assets/icon.png"
        );
    }

    #[test]
    fn bepinex_owned_directories_merge_in_place() {
        assert_eq!(bepinex_of(r("config/mod.cfg")), "config/mod.cfg");
        assert_eq!(bepinex_of(r("core/lib.dll")), "core/lib.dll");
        assert_eq!(
            bepinex_of(r("patchers/patch.dll")),
            "patchers/Author-Mod/patch.dll"
        );
    }

    #[test]
    fn framework_payload_unpacks_over_the_server_root() {
        let t = route(
            &PathBuf::from("BepInExPack_Valheim/winhttp.dll"),
            "denikson-BepInExPack_Valheim",
            Some("BepInExPack_Valheim/"),
        );
        match t {
            Target::Root(p) => assert_eq!(p.to_string_lossy(), "winhttp.dll"),
            _ => panic!("framework files belong at the server root"),
        }
    }

    #[test]
    fn framework_metadata_outside_the_pack_is_dropped() {
        let t = route(
            &PathBuf::from("manifest.json"),
            "denikson-BepInExPack_Valheim",
            Some("BepInExPack_Valheim/"),
        );
        assert!(matches!(t, Target::Skip));
    }

    #[cfg(unix)]
    #[test]
    fn launch_scripts_are_made_executable() {
        // BepInEx ships these as 0666, so the extractor has to add the bit.
        assert_eq!(
            executable_mode(Some(0o666), Path::new("start_server_bepinex.sh")),
            Some(0o755)
        );
        // An archive that already marks a file executable is honoured as-is.
        assert_eq!(executable_mode(Some(0o750), Path::new("tool")), Some(0o750));
        // Everything else keeps the filesystem default.
        assert_eq!(executable_mode(Some(0o666), Path::new("Mod.dll")), None);
        assert_eq!(executable_mode(None, Path::new("Mod.dll")), None);
    }

    #[test]
    fn zip_slip_paths_are_rejected() {
        assert!(safe_relative("../../etc/passwd").is_none());
        assert!(safe_relative("/etc/passwd").is_none());
        assert!(safe_relative("plugins/../../escape.dll").is_none());
        assert!(safe_relative("").is_none());
        assert!(safe_relative("plugins/ok.dll").is_some());
    }
}

/// End-to-end checks against live Thunderstore downloads.
///
/// Excluded from the default run because they need network access; execute
/// with `cargo test -- --ignored --nocapture`.
#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::config::Layout;
    use crate::thunderstore::Mod;

    /// Resolves a package's current release so the test does not pin versions.
    fn latest(client: &reqwest::blocking::Client, owner: &str, name: &str) -> Mod {
        let url = format!("https://thunderstore.io/api/experimental/package/{owner}/{name}/");
        let body: serde_json::Value = client.get(&url).send().unwrap().json().unwrap();
        let latest = &body["latest"];
        Mod {
            full_name: format!("{owner}-{name}"),
            name: name.into(),
            owner: owner.into(),
            description: String::new(),
            version: latest["version_number"].as_str().unwrap().into(),
            download_url: latest["download_url"].as_str().unwrap().into(),
            package_url: String::new(),
            downloads: 0,
            rating: 0,
            file_size: 0,
            date_updated: String::new(),
            categories: vec![],
            dependencies: vec![],
            deprecated: false,
        }
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("vmt-live-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn exists(root: &Path, rel: &str) -> bool {
        root.join(rel).exists()
    }

    #[test]
    #[ignore = "requires network access"]
    fn real_packages_unpack_into_a_working_bepinex_layout() {
        let client = http_client().unwrap();
        let root = scratch("layout");
        let layout = Layout::resolve(&root).unwrap();
        let log = |line: String| println!("{line}");

        // The framework unpacks over the server root and creates BepInEx.
        let bepinex = latest(&client, "denikson", "BepInExPack_Valheim");
        install(&client, &bepinex, &layout, &log).unwrap();
        assert!(exists(&root, "BepInEx/core/BepInEx.dll"), "BepInEx core missing");
        assert!(exists(&root, "BepInEx/config/BepInEx.cfg"), "BepInEx config missing");
        assert!(
            exists(&root, "winhttp.dll") || exists(&root, "doorstop_config.ini"),
            "doorstop loader missing from the server root"
        );
        assert!(!exists(&root, "manifest.json"), "package metadata leaked to root");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let script = root.join("start_server_bepinex.sh");
            if script.exists() {
                let mode = std::fs::metadata(&script).unwrap().permissions().mode();
                assert!(mode & 0o111 != 0, "launch script must stay executable");
            }
        }
        assert!(layout.bepinex_present());

        // A plugins/-style archive.
        let jotunn = latest(&client, "ValheimModding", "Jotunn");
        install(&client, &jotunn, &layout, &log).unwrap();
        assert!(
            exists(&root, "BepInEx/plugins/ValheimModding-Jotunn/Jotunn.dll"),
            "Jotunn.dll not in its scoped plugin folder"
        );

        // A bare-DLL-at-root archive.
        let dust = latest(&client, "Azumatt", "NoBuildDust");
        install(&client, &dust, &layout, &log).unwrap();
        assert!(
            exists(&root, "BepInEx/plugins/Azumatt-NoBuildDust/NoBuildDust.dll"),
            "loose DLL not placed in its scoped plugin folder"
        );

        // A patchers/-style archive.
        let tgh = latest(&client, "ASharpPen", "This_Goes_Here");
        install(&client, &tgh, &layout, &log).unwrap();
        assert!(
            exists(&root, "BepInEx/patchers/ASharpPen-This_Goes_Here/Valheim.ThisGoesHere.dll"),
            "patcher not placed under BepInEx/patchers"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    #[ignore = "requires network access"]
    fn reinstalling_preserves_an_edited_config() {
        let client = http_client().unwrap();
        let root = scratch("config");
        let layout = Layout::resolve(&root).unwrap();
        let log = |_: String| {};

        let bepinex = latest(&client, "denikson", "BepInExPack_Valheim");
        install(&client, &bepinex, &layout, &log).unwrap();

        let cfg = root.join("BepInEx/config/BepInEx.cfg");
        std::fs::write(&cfg, "# tuned by hand\n").unwrap();

        let report = install(&client, &bepinex, &layout, &log).unwrap();
        assert_eq!(
            std::fs::read_to_string(&cfg).unwrap(),
            "# tuned by hand\n",
            "a reinstall must not overwrite user config"
        );
        assert!(report.skipped_config > 0);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    #[ignore = "requires network access"]
    fn the_catalogue_parses_and_yields_v1_mods() {
        let mods = crate::thunderstore::load(true).unwrap();
        assert!(mods.len() > 5_000, "unexpectedly small catalogue");

        let ready: Vec<_> = mods.iter().filter(|m| m.supports_v1()).collect();
        assert!(ready.len() > 500, "1.0 filter kept too few mods");
        assert!(ready.iter().all(|m| !m.deprecated));

        // Dependencies must resolve against the index for the tree to be useful.
        let index = crate::index::Index::new(mods.clone());
        let total: usize = ready.iter().map(|m| m.dependencies.len()).sum();
        let unresolved = ready
            .iter()
            .flat_map(|m| m.dependencies.iter())
            .filter(|d| index.lookup(crate::index::Index::dep_key(d)).is_none())
            .count();
        println!("{} mods ready, {total} dependency refs, {unresolved} unresolved", ready.len());
        assert!(
            unresolved * 100 < total,
            "more than 1% of dependencies failed to resolve"
        );
    }
}
