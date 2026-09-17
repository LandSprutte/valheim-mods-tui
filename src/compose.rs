//! Writing a mod list into a .env file or a docker-compose service.
//!
//! Edits are line-targeted rather than a YAML round-trip, because rewriting the
//! document would discard the comments and formatting of a file the user owns.

use anyhow::{bail, Result};
use std::path::Path;

/// How a target file expresses variables.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Kind {
    /// KEY=value lines.
    Env,
    /// A docker-compose service's `environment:` block.
    Compose,
}

/// Guesses the format from a file name.
pub fn kind_of(path: &Path) -> Kind {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if name.ends_with(".yml") || name.ends_with(".yaml") {
        Kind::Compose
    } else {
        Kind::Env
    }
}

/// Files worth offering, in preference order, found next to the user.
pub fn discover(dir: &Path) -> Option<std::path::PathBuf> {
    [
        "docker-compose.yml",
        "docker-compose.yaml",
        "compose.yml",
        "compose.yaml",
        ".env",
    ]
    .iter()
    .map(|n| dir.join(n))
    .find(|p| p.is_file())
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// Sets each key in a KEY=value file, replacing existing lines in place.
pub fn update_env(src: &str, vars: &[(&str, String)]) -> String {
    let mut lines: Vec<String> = src.lines().map(str::to_string).collect();

    for (key, value) in vars {
        let prefix = format!("{key}=");
        // A commented-out assignment is left alone; it is the user's note.
        match lines
            .iter()
            .position(|l| l.trim_start().starts_with(&prefix))
        {
            Some(i) => lines[i] = format!("{key}={value}"),
            None => lines.push(format!("{key}={value}")),
        }
    }

    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Sets each key inside a compose service's `environment:` block.
///
/// Both mapping (`KEY: value`) and list (`- KEY=value`) styles are supported;
/// whichever the block already uses is preserved.
pub fn update_compose(src: &str, service: Option<&str>, vars: &[(&str, String)]) -> Result<String> {
    let mut lines: Vec<String> = src.lines().map(str::to_string).collect();
    let env_line = find_environment(&lines, service)?;
    let env_indent = indent_of(&lines[env_line]);

    // The block runs until a line at or above the `environment:` indent.
    let mut end = env_line + 1;
    while end < lines.len() {
        let line = &lines[end];
        if line.trim().is_empty() {
            end += 1;
            continue;
        }
        if indent_of(line) <= env_indent {
            break;
        }
        end += 1;
    }
    // Trailing blank lines belong after the block, not inside it.
    while end > env_line + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }

    let body = &lines[env_line + 1..end];
    let list_style = body.iter().any(|l| l.trim_start().starts_with("- "));
    let child_indent = body
        .iter()
        .find(|l| !l.trim().is_empty())
        .map(|l| indent_of(l))
        .unwrap_or(env_indent + 2);
    let pad = " ".repeat(child_indent);

    let render = |key: &str, value: &str| {
        if list_style {
            format!("{pad}- {key}={value}")
        } else {
            format!("{pad}{key}: \"{value}\"")
        }
    };

    for (key, value) in vars {
        let mapping_prefix = format!("{key}:");
        let list_prefix = format!("- {key}=");
        let found = (env_line + 1..end).find(|&i| {
            let t = lines[i].trim_start();
            t.starts_with(&mapping_prefix) || t.starts_with(&list_prefix)
        });
        match found {
            Some(i) => lines[i] = render(key, value),
            None => {
                lines.insert(end, render(key, value));
                end += 1;
            }
        }
    }

    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

/// Locates the `environment:` key, optionally within a named service.
fn find_environment(lines: &[String], service: Option<&str>) -> Result<usize> {
    let is_env = |l: &str| {
        let t = l.trim_end();
        t.trim_start() == "environment:" || t.trim_start().starts_with("environment: ")
    };

    let Some(service) = service else {
        return lines
            .iter()
            .position(|l| is_env(l))
            .ok_or_else(|| anyhow::anyhow!(
                "no `environment:` block found — add one to the service first, \
                 so nothing has to be guessed about the file's layout"
            ));
    };

    let header = format!("{service}:");
    let start = lines
        .iter()
        .position(|l| l.trim_start() == header)
        .ok_or_else(|| anyhow::anyhow!("no service named {service:?} in that file"))?;
    let service_indent = indent_of(&lines[start]);

    for (i, line) in lines.iter().enumerate().skip(start + 1) {
        if !line.trim().is_empty() && indent_of(line) <= service_indent {
            break;
        }
        if is_env(line) {
            return Ok(i);
        }
    }
    bail!("service {service:?} has no `environment:` block — add one first")
}

/// Applies the variables to a file, keeping a `.bak` copy of the original.
pub fn write_to_file(
    path: &Path,
    service: Option<&str>,
    vars: &[(&str, String)],
) -> Result<std::path::PathBuf> {
    let src = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?;

    let updated = match kind_of(path) {
        Kind::Env => update_env(&src, vars),
        Kind::Compose => update_compose(&src, service, vars)?,
    };

    // Keep the original recoverable; this edits a file the user maintains.
    let backup = path.with_extension(format!(
        "{}bak",
        path.extension()
            .map(|e| format!("{}.", e.to_string_lossy()))
            .unwrap_or_default()
    ));
    std::fs::copy(path, &backup)
        .map_err(|e| anyhow::anyhow!("backing up to {}: {e}", backup.display()))?;
    std::fs::write(path, updated)
        .map_err(|e| anyhow::anyhow!("writing {}: {e}", path.display()))?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> Vec<(&'static str, String)> {
        vec![
            ("MODS", "A-B-1.0.0,C-D-2.0.0".to_string()),
            ("BEPINEXPACK_VERSION", "5.4.2350".to_string()),
        ]
    }

    #[test]
    fn env_files_get_keys_replaced_not_duplicated() {
        let src = "SERVER_NAME=mine\nMODS=old-1.0.0\nPASSWORD=secret\n";
        let out = update_env(src, &vars());
        assert!(out.contains("MODS=A-B-1.0.0,C-D-2.0.0"));
        assert!(!out.contains("old-1.0.0"));
        assert_eq!(out.matches("MODS=").count(), 1);
        // Unrelated settings survive untouched.
        assert!(out.contains("SERVER_NAME=mine"));
        assert!(out.contains("PASSWORD=secret"));
        // A key that was absent is appended.
        assert!(out.contains("BEPINEXPACK_VERSION=5.4.2350"));
    }

    #[test]
    fn a_commented_assignment_is_not_treated_as_the_value() {
        let src = "# MODS=do-not-touch\nSERVER_NAME=mine\n";
        let out = update_env(src, &[("MODS", "new-1.0.0".into())]);
        assert!(out.contains("# MODS=do-not-touch"), "comment must survive");
        assert!(out.contains("\nMODS=new-1.0.0"));
    }

    #[test]
    fn compose_mapping_style_is_updated_in_place() {
        let src = "\
services:
  valheim:
    image: indifferentbroccoli/valheim-server-docker
    environment:
      SERVER_NAME: \"mine\"
      MODS: \"stale-0.0.1\"
    volumes:
      - ./valheim/server-files:/valheim
";
        let out = update_compose(src, Some("valheim"), &vars()).unwrap();
        assert!(out.contains("      MODS: \"A-B-1.0.0,C-D-2.0.0\""), "{out}");
        assert!(!out.contains("stale-0.0.1"));
        assert!(out.contains("      BEPINEXPACK_VERSION: \"5.4.2350\""), "{out}");
        // The rest of the document is untouched, volumes included.
        assert!(out.contains("      SERVER_NAME: \"mine\""));
        assert!(out.contains("      - ./valheim/server-files:/valheim"));
        assert_eq!(out.matches("MODS").count(), 1);
    }

    #[test]
    fn compose_list_style_keeps_its_own_style() {
        let src = "\
services:
  valheim:
    environment:
      - SERVER_NAME=mine
      - MODS=stale
    ports:
      - 2456:2456/udp
";
        let out = update_compose(src, Some("valheim"), &vars()).unwrap();
        assert!(out.contains("      - MODS=A-B-1.0.0,C-D-2.0.0"), "{out}");
        assert!(out.contains("      - BEPINEXPACK_VERSION=5.4.2350"), "{out}");
        assert!(!out.contains("MODS: "), "mapping style must not be introduced");
        assert!(out.contains("      - 2456:2456/udp"));
    }

    #[test]
    fn comments_and_blank_lines_survive_the_edit() {
        let src = "\
# my server
services:
  valheim:
    environment:
      # tuning
      SERVER_NAME: \"mine\"

    restart: unless-stopped
";
        let out = update_compose(src, Some("valheim"), &vars()).unwrap();
        assert!(out.contains("# my server"));
        assert!(out.contains("      # tuning"));
        assert!(out.contains("    restart: unless-stopped"));
        // The new key lands inside the block, above the blank line and the
        // sibling key that follows it.
        let mods = out.find("MODS").unwrap();
        assert!(mods < out.find("restart:").unwrap(), "leaked out of the block:\n{out}");
    }

    #[test]
    fn a_missing_key_is_inserted_into_an_existing_block() {
        let src = "services:\n  valheim:\n    environment:\n      SERVER_NAME: \"mine\"\n";
        let out = update_compose(src, Some("valheim"), &vars()).unwrap();
        assert!(out.contains("      MODS: \"A-B-1.0.0,C-D-2.0.0\""), "{out}");
        assert!(out.contains("      SERVER_NAME: \"mine\""));
    }

    #[test]
    fn the_right_service_is_edited_when_several_exist() {
        let src = "\
services:
  db:
    environment:
      MODS: \"not-this-one\"
  valheim:
    environment:
      MODS: \"this-one\"
";
        let out = update_compose(src, Some("valheim"), &[("MODS", "new".into())]).unwrap();
        assert!(out.contains("MODS: \"not-this-one\""), "db must be untouched:\n{out}");
        assert!(out.contains("MODS: \"new\""));
        assert!(!out.contains("\"this-one\""), "the valheim value should be gone:\n{out}");
    }

    #[test]
    fn a_file_with_no_environment_block_is_refused() {
        let src = "services:\n  valheim:\n    image: x\n";
        let err = update_compose(src, Some("valheim"), &vars()).unwrap_err();
        assert!(err.to_string().contains("no `environment:` block"), "{err}");
    }

    #[test]
    fn an_unknown_service_is_refused_rather_than_guessed() {
        let src = "services:\n  valheim:\n    environment:\n      A: \"b\"\n";
        let err = update_compose(src, Some("other"), &vars()).unwrap_err();
        assert!(err.to_string().contains("no service named"), "{err}");
    }

    #[test]
    fn file_kind_follows_the_extension() {
        assert_eq!(kind_of(Path::new("docker-compose.yml")), Kind::Compose);
        assert_eq!(kind_of(Path::new("compose.yaml")), Kind::Compose);
        assert_eq!(kind_of(Path::new(".env")), Kind::Env);
        assert_eq!(kind_of(Path::new("stack.env")), Kind::Env);
    }
}
