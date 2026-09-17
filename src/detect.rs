//! Finding Valheim installations, so a misconfigured path is caught at startup.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Mac,
    Linux,
}

pub fn current_os() -> Os {
    if cfg!(target_os = "windows") {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::Mac
    } else {
        Os::Linux
    }
}

/// A directory that looks like a Valheim install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    pub path: PathBuf,
    /// What the directory appears to be: a server, the game, or bare BepInEx.
    pub label: &'static str,
    pub has_bepinex: bool,
}

/// Files that identify a Valheim directory, most specific first.
const MARKERS: [(&str, &str); 6] = [
    ("valheim_server.x86_64", "dedicated server"),
    ("valheim_server.exe", "dedicated server"),
    ("valheim_server_Data", "dedicated server"),
    ("valheim.exe", "game install"),
    ("valheim.x86_64", "game install"),
    ("valheim_Data", "game install"),
];

/// Decides whether a directory is a Valheim install, and what kind.
pub fn classify(path: &Path) -> Option<Install> {
    if !path.is_dir() {
        return None;
    }
    let has_bepinex = path.join("BepInEx").is_dir();

    for (marker, label) in MARKERS {
        if path.join(marker).exists() {
            return Some(Install {
                path: path.to_path_buf(),
                label,
                has_bepinex,
            });
        }
    }
    // A directory holding only BepInEx is still a usable target: mods live
    // there even when the game files sit elsewhere.
    has_bepinex.then(|| Install {
        path: path.to_path_buf(),
        label: "BepInEx directory",
        has_bepinex: true,
    })
}

/// Paths worth checking for a given platform, before touching the filesystem.
pub fn candidate_roots(home: &Path, os: Os) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();

    let steam_roots: Vec<PathBuf> = match os {
        Os::Windows => vec![
            PathBuf::from(r"C:\Program Files (x86)\Steam"),
            PathBuf::from(r"C:\Program Files\Steam"),
            home.join("Steam"),
        ],
        Os::Mac => vec![home.join("Library/Application Support/Steam")],
        Os::Linux => vec![
            home.join(".steam/steam"),
            home.join(".local/share/Steam"),
            home.join(".steam/root"),
            home.join("Steam"),
            home.join("snap/steam/common/.local/share/Steam"),
        ],
    };

    // Steam keeps games under steamapps/common, and may have extra libraries
    // on other drives listed in libraryfolders.vdf.
    let mut libraries = steam_roots.clone();
    for root in &steam_roots {
        libraries.extend(library_folders(&root.join("steamapps/libraryfolders.vdf")));
    }
    for library in libraries {
        let common = library.join("steamapps/common");
        roots.push(common.join("Valheim"));
        roots.push(common.join("Valheim dedicated server"));
    }

    // Server installs rarely live under Steam's default tree.
    match os {
        Os::Linux => roots.extend(
            [
                "/opt/valheim",
                "/opt/valheim/server",
                "/srv/valheim",
                "/home/steam/valheim",
                "/home/steam/valheim-server",
                "/root/valheim",
                "/root/valheim/server",
            ]
            .iter()
            .map(PathBuf::from),
        ),
        Os::Mac | Os::Windows => {}
    }
    roots.push(home.join("valheim"));
    roots.push(home.join("valheim-server"));

    roots.dedup();
    roots
}

/// Extra Steam library paths declared in a libraryfolders.vdf.
pub fn library_folders(vdf: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(vdf) else {
        return Vec::new();
    };
    parse_library_folders(&text)
}

/// Pulls the `"path"` values out of a Steam libraryfolders.vdf.
pub fn parse_library_folders(text: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("\"path\"") {
            continue;
        }
        // The value is the second quoted field on the line.
        if let Some(value) = trimmed.split('"').nth(3) {
            if !value.is_empty() {
                // VDF escapes backslashes, which matters on Windows paths.
                out.push(PathBuf::from(value.replace("\\\\", "\\")));
            }
        }
    }
    out
}

/// Scans the platform's likely locations for Valheim installs.
///
/// Installs that already have BepInEx sort first, since those are almost always
/// the one the user means.
pub fn detect() -> Vec<Install> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let mut found: Vec<Install> = candidate_roots(&home, current_os())
        .iter()
        .filter_map(|p| classify(p))
        .collect();

    found.sort_by_key(|i| (!i.has_bepinex, i.path.clone()));
    found.dedup_by(|a, b| a.path == b.path);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vmt-detect-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_dedicated_server_directory_is_recognised() {
        let d = scratch("server");
        std::fs::write(d.join("valheim_server.x86_64"), b"x").unwrap();
        let found = classify(&d).unwrap();
        assert_eq!(found.label, "dedicated server");
        assert!(!found.has_bepinex);

        std::fs::create_dir_all(d.join("BepInEx")).unwrap();
        assert!(classify(&d).unwrap().has_bepinex);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_game_install_is_recognised() {
        let d = scratch("game");
        std::fs::create_dir_all(d.join("valheim_Data")).unwrap();
        assert_eq!(classify(&d).unwrap().label, "game install");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_bare_bepinex_folder_still_counts_as_a_target() {
        let d = scratch("bare");
        std::fs::create_dir_all(d.join("BepInEx")).unwrap();
        let found = classify(&d).unwrap();
        assert_eq!(found.label, "BepInEx directory");
        assert!(found.has_bepinex);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn an_unrelated_directory_is_not_a_valheim_install() {
        let d = scratch("empty");
        assert!(classify(&d).is_none());
        std::fs::write(d.join("readme.txt"), b"x").unwrap();
        assert!(classify(&d).is_none());
        assert!(classify(&d.join("missing")).is_none());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn each_platform_looks_where_that_platform_keeps_steam() {
        let home = Path::new("/home/me");
        let as_text = |os| {
            candidate_roots(home, os)
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
        };

        let linux = as_text(Os::Linux);
        assert!(linux.iter().any(|p| p.contains(".steam/steam/steamapps/common/Valheim")));
        assert!(linux.iter().any(|p| p.contains(".local/share/Steam/steamapps/common/Valheim")));
        assert!(linux.iter().any(|p| p == "/opt/valheim"));
        assert!(linux.iter().any(|p| p.contains("Valheim dedicated server")));

        let mac = as_text(Os::Mac);
        assert!(mac
            .iter()
            .any(|p| p.contains("Library/Application Support/Steam/steamapps/common/Valheim")));

        let windows = as_text(Os::Windows);
        assert!(windows.iter().any(|p| p.contains("Program Files (x86)")));
        assert!(windows.iter().any(|p| p.contains("Valheim")));

        // Linux server conventions must not leak onto the other platforms.
        assert!(!mac.iter().any(|p| p == "/opt/valheim"));
        assert!(!windows.iter().any(|p| p == "/opt/valheim"));
    }

    #[test]
    fn steam_libraries_on_other_drives_are_read_from_the_vdf() {
        let vdf = r#"
"libraryfolders"
{
    "0"
    {
        "path"		"C:\\Program Files (x86)\\Steam"
        "label"		""
    }
    "1"
    {
        "path"		"D:\\SteamLibrary"
    }
}
"#;
        let paths = parse_library_folders(vdf);
        assert_eq!(paths.len(), 2);
        assert!(paths[1].display().to_string().contains("SteamLibrary"));
        // The label field must not be mistaken for a path.
        assert!(!paths.iter().any(|p| p.as_os_str().is_empty()));
    }

    #[test]
    fn a_missing_or_junk_vdf_yields_nothing() {
        assert!(library_folders(Path::new("/definitely/not/here.vdf")).is_empty());
        assert!(parse_library_folders("not a vdf at all").is_empty());
    }
}
