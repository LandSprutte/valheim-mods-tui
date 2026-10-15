//! Working out which BepInEx loader applies to a given Valheim install.
//!
//! The Thunderstore BepInEx pack ships every platform's loader at once: a
//! `winhttp.dll` for Windows, and `doorstop_libs` plus shell scripts for Linux
//! and macOS. Which one is live depends on the *game build* in the folder, not
//! on the machine doing the installing — a Windows build running under Proton
//! or Crossover on a Mac still needs `winhttp.dll`.

use anyhow::{Context, Result};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Build {
    Windows,
    Mac,
    Linux,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Loader {
    pub build: Build,
    /// The macOS application bundle, when this is a native Mac build.
    pub app: Option<String>,
    /// The file that actually hooks the game.
    pub hook: &'static str,
    /// How the user has to start the game for mods to load.
    pub launch: String,
}

/// The shell script the Unix builds launch through.
const GAME_SCRIPT: &str = "start_game_bepinex.sh";
const SERVER_SCRIPT: &str = "start_server_bepinex.sh";
/// Finds the macOS application bundle in an install directory.
pub fn find_app(root: &Path) -> Option<String> {
    let entries = std::fs::read_dir(root).ok()?;
    let mut apps: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let is_app = name.to_lowercase().ends_with(".app") && e.path().is_dir();
            is_app.then_some(name)
        })
        .collect();
    apps.sort();
    // Prefer a bundle that looks like the game over any helper app.
    apps.iter()
        .find(|n| n.to_lowercase().starts_with("valheim"))
        .or_else(|| apps.first())
        .cloned()
}

/// Determines which loader a Valheim directory needs.
pub fn detect(root: &Path) -> Loader {
    let has = |n: &str| root.join(n).exists();
    let server = has("valheim_server.exe") || has("valheim_server.x86_64");

    // A Windows build wins even on a Mac or Linux host: that means Proton,
    // Wine or Crossover, where the Windows loader is the correct one.
    let build = if has("valheim.exe") || has("valheim_server.exe") {
        Build::Windows
    } else if find_app(root).is_some() {
        Build::Mac
    } else if has("valheim.x86_64") || has("valheim_server.x86_64") {
        Build::Linux
    } else {
        Build::Unknown
    };

    let app = (build == Build::Mac).then(|| find_app(root)).flatten();

    let (hook, launch) = match (build, server) {
        (Build::Windows, false) => (
            "winhttp.dll",
            "start Valheim as usual — winhttp.dll loads BepInEx".to_string(),
        ),
        (Build::Windows, true) => (
            "winhttp.dll",
            "start the server as usual — winhttp.dll loads BepInEx".to_string(),
        ),
        (Build::Mac, _) => (
            "doorstop_libs/libdoorstop_x64.dylib",
            format!("launch with ./{GAME_SCRIPT} (winhttp.dll is unused on a Mac build)"),
        ),
        (Build::Linux, false) => (
            "doorstop_libs/libdoorstop_x64.so",
            format!("launch with ./{GAME_SCRIPT}"),
        ),
        (Build::Linux, true) => (
            "doorstop_libs/libdoorstop_x64.so",
            format!("launch the server via ./{SERVER_SCRIPT}, or export its DOORSTOP_/LD_PRELOAD vars"),
        ),
        (Build::Unknown, _) => (
            "unknown",
            "no Valheim executable found here — check the folder".to_string(),
        ),
    };

    Loader {
        build,
        app,
        hook,
        launch: launch.to_string(),
    }
}

/// Points the launch script at the macOS app bundle.
///
/// The pack hardcodes the Linux executable name, so on a native Mac build the
/// script aborts with "Please set executable_name" until this is corrected.
/// Returns the name written, or `None` when nothing needed changing.
pub fn configure_mac_script(root: &Path, app: &str) -> Result<Option<String>> {
    let script = root.join(GAME_SCRIPT);
    if !script.is_file() {
        return Ok(None);
    }
    let src = std::fs::read_to_string(&script)
        .with_context(|| format!("reading {}", script.display()))?;

    let wanted = format!("executable_name=\"{app}\"");
    let mut changed = false;
    let mut out: Vec<String> = Vec::with_capacity(src.lines().count());
    for line in src.lines() {
        // Only the unindented setting at the top of the file is the user's
        // config. The script body also assigns executable_name (from "$1"),
        // and rewriting that would break its argument handling — so an
        // indented or commented assignment is left strictly alone, and only
        // the first match is touched.
        let is_setting = !changed
            && line.starts_with("executable_name=")
            && !line.starts_with('#');
        if is_setting {
            if line.trim_end() == wanted {
                return Ok(None);
            }
            out.push(wanted.clone());
            changed = true;
        } else {
            out.push(line.to_string());
        }
    }
    if !changed {
        return Ok(None);
    }

    let mut body = out.join("\n");
    if src.ends_with('\n') {
        body.push('\n');
    }
    std::fs::write(&script, body).with_context(|| format!("writing {}", script.display()))?;
    Ok(Some(app.to_string()))
}

/// Applies any platform fix-ups the install needs, describing what it did.
pub fn configure(root: &Path) -> (Loader, Option<String>) {
    let loader = detect(root);
    let note = match (&loader.build, &loader.app) {
        (Build::Mac, Some(app)) => match configure_mac_script(root, app) {
            Ok(Some(name)) => Some(format!("pointed {GAME_SCRIPT} at {name}")),
            Ok(None) => None,
            Err(e) => Some(format!("could not adjust {GAME_SCRIPT}: {e:#}")),
        },
        _ => None,
    };
    (loader, note)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// The value the pack ships, correct only for Linux.
    const SHIPPED_EXECUTABLE: &str = "valheim.x86_64";

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vmt-loader-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The script as the Thunderstore pack ships it.
    fn shipped_script(dir: &Path) {
        std::fs::write(
            dir.join(GAME_SCRIPT),
            "#!/bin/sh\n\
             # LINUX: name of Unity executable\n\
             # MACOS: name of the .app directory\n\
             executable_name=\"valheim.x86_64\"\n\
             enabled=\"1\"\n",
        )
        .unwrap();
    }

    #[test]
    fn a_native_mac_build_uses_the_dylib_not_winhttp() {
        let d = scratch("mac");
        std::fs::create_dir_all(d.join("valheim.app")).unwrap();
        // The pack installs every platform's loader side by side.
        std::fs::write(d.join("winhttp.dll"), b"x").unwrap();

        let l = detect(&d);
        assert_eq!(l.build, Build::Mac);
        assert_eq!(l.app.as_deref(), Some("valheim.app"));
        assert!(l.hook.ends_with("dylib"), "got {}", l.hook);
        assert!(l.launch.contains("start_game_bepinex.sh"), "{}", l.launch);
        assert!(l.launch.contains("winhttp.dll is unused"), "{}", l.launch);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_windows_build_wins_even_on_a_mac_host() {
        // Valheim under Proton or Crossover is a Windows build in a Mac's
        // Steam folder, and genuinely needs winhttp.dll.
        let d = scratch("proton");
        std::fs::write(d.join("valheim.exe"), b"x").unwrap();
        std::fs::create_dir_all(d.join("valheim.app")).unwrap();

        let l = detect(&d);
        assert_eq!(l.build, Build::Windows);
        assert_eq!(l.hook, "winhttp.dll");
        assert!(l.launch.contains("as usual"), "{}", l.launch);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_linux_server_is_told_about_the_server_script() {
        let d = scratch("linuxsrv");
        std::fs::write(d.join("valheim_server.x86_64"), b"x").unwrap();

        let l = detect(&d);
        assert_eq!(l.build, Build::Linux);
        assert!(l.hook.ends_with(".so"));
        assert!(l.launch.contains("start_server_bepinex.sh"), "{}", l.launch);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_folder_with_no_game_is_reported_as_unknown() {
        let d = scratch("empty");
        assert_eq!(detect(&d).build, Build::Unknown);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn the_mac_launch_script_is_pointed_at_the_app_bundle() {
        let d = scratch("patch");
        std::fs::create_dir_all(d.join("valheim.app")).unwrap();
        shipped_script(&d);

        let (loader, note) = configure(&d);
        assert_eq!(loader.build, Build::Mac);
        assert!(note.unwrap().contains("valheim.app"));

        let script = std::fs::read_to_string(d.join(GAME_SCRIPT)).unwrap();
        assert!(script.contains("executable_name=\"valheim.app\""), "{script}");
        assert!(!script.contains(&format!("executable_name=\"{SHIPPED_EXECUTABLE}\"")));
        // The documentation comments above the setting must survive.
        assert!(script.contains("# MACOS: name of the .app directory"), "{script}");
        assert!(script.contains("enabled=\"1\""));
        std::fs::remove_dir_all(&d).ok();
    }

    /// The real script assigns executable_name a second time, indented, inside
    /// its argument handling. Rewriting that line breaks `./script <exe>`.
    #[test]
    fn only_the_top_level_setting_is_rewritten() {
        let d = scratch("body");
        std::fs::create_dir_all(d.join("valheim.app")).unwrap();
        std::fs::write(
            d.join(GAME_SCRIPT),
            "#!/bin/sh\n\
             # MACOS: name of the .app directory\n\
             executable_name=\"valheim.x86_64\"\n\
             \n\
             # Handle first param being executable name\n\
             if [ -x \"$1\" ] ; then\n\
             \x20   executable_name=\"$1\"\n\
             \x20   shift\n\
             fi\n",
        )
        .unwrap();

        configure_mac_script(&d, "valheim.app").unwrap().unwrap();
        let script = std::fs::read_to_string(d.join(GAME_SCRIPT)).unwrap();

        assert!(script.contains("executable_name=\"valheim.app\""));
        assert!(
            script.contains("    executable_name=\"$1\""),
            "the script body must be untouched:\n{script}"
        );
        assert_eq!(
            script.matches("valheim.app").count(),
            1,
            "exactly one line should change:\n{script}"
        );
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn rewriting_the_script_is_idempotent() {
        let d = scratch("idem");
        std::fs::create_dir_all(d.join("valheim.app")).unwrap();
        shipped_script(&d);

        assert!(configure_mac_script(&d, "valheim.app").unwrap().is_some());
        // A second pass has nothing to do.
        assert!(configure_mac_script(&d, "valheim.app").unwrap().is_none());
        let script = std::fs::read_to_string(d.join(GAME_SCRIPT)).unwrap();
        assert_eq!(script.matches("executable_name=").count(), 1);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn linux_and_windows_installs_are_left_alone() {
        let d = scratch("nopatch");
        std::fs::write(d.join("valheim.x86_64"), b"x").unwrap();
        shipped_script(&d);

        let (_, note) = configure(&d);
        assert!(note.is_none(), "a Linux install needs no rewrite");
        let script = std::fs::read_to_string(d.join(GAME_SCRIPT)).unwrap();
        assert!(script.contains(&format!("executable_name=\"{SHIPPED_EXECUTABLE}\"")));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_missing_script_is_not_an_error() {
        let d = scratch("noscript");
        std::fs::create_dir_all(d.join("valheim.app")).unwrap();
        assert!(configure_mac_script(&d, "valheim.app").unwrap().is_none());
        std::fs::remove_dir_all(&d).ok();
    }
}
