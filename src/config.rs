//! Persisted settings and Valheim install-path resolution.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Valheim server root, its BepInEx directory, or the plugins directory —
    /// all three spellings are accepted and normalised on use.
    pub install_dir: Option<PathBuf>,
    /// Where `--update` fetches new builds: "owner/repo" or a URL.
    #[serde(default)]
    pub update_source: Option<String>,
    /// docker-compose.yml or .env that receives the MODS list.
    #[serde(default)]
    pub compose_file: Option<PathBuf>,
    /// Which compose service to edit; defaults to the first with an
    /// `environment:` block.
    #[serde(default)]
    pub compose_service: Option<String>,
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("valheim-mods-tui")
        .join("config.json")
}

impl Config {
    pub fn load() -> Self {
        std::fs::read(config_path())
            .ok()
            .and_then(|raw| serde_json::from_slice(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}

/// Resolved destination directories for an install.
#[derive(Debug, Clone)]
pub struct Layout {
    /// Game/server root — where BepInEx itself unpacks.
    pub root: PathBuf,
    /// The BepInEx directory holding plugins, config, patchers.
    pub bepinex: PathBuf,
}

impl Layout {
    /// Accepts the server root, the BepInEx dir, or the plugins dir.
    pub fn resolve(path: &Path) -> Result<Layout> {
        let path = path.to_path_buf();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        let (root, bepinex) = if name == "plugins" {
            let bepinex = path
                .parent()
                .map(Path::to_path_buf)
                .ok_or_else(|| anyhow::anyhow!("plugins directory has no parent"))?;
            let root = bepinex.parent().map(Path::to_path_buf).unwrap_or_default();
            (root, bepinex)
        } else if name == "bepinex" {
            let root = path.parent().map(Path::to_path_buf).unwrap_or_default();
            (root, path)
        } else {
            let bepinex = path.join("BepInEx");
            (path, bepinex)
        };

        if !root.is_dir() {
            bail!("install directory does not exist: {}", root.display());
        }
        Ok(Layout { root, bepinex })
    }

    pub fn plugins(&self) -> PathBuf {
        self.bepinex.join("plugins")
    }

    /// True once BepInEx has been unpacked into the root.
    pub fn bepinex_present(&self) -> bool {
        self.bepinex.is_dir()
    }
}
