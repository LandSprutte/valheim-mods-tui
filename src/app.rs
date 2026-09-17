//! Application state, input handling, and background work.

use crate::config::Layout;
use crate::index::{build_rows, Filter, Index, Row, Sort};
use crate::install;
use crate::installed::{self, Installed};
use crate::thunderstore::{self, Mod};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender};

/// Messages sent from background workers to the UI thread.
pub enum Msg {
    Catalog(Result<Vec<Mod>, String>),
    Log(String),
    InstallDone(Result<String, String>),
}

/// Which modal, if any, is on screen.
#[derive(PartialEq, Eq)]
pub enum Modal {
    None,
    Help,
    Confirm {
        plan: Vec<usize>,
        missing: Vec<String>,
        /// Set while the destination is being typed.
        editing: Option<String>,
    },
    /// The selection rendered as a Thunderstore dependency list, for server
    /// images that install mods themselves.
    Export {
        mods: String,
        count: usize,
        bepinex: Option<String>,
        written: Result<String, String>,
        /// A docker-compose.yml or .env that MODS can be written into.
        target: Option<std::path::PathBuf>,
        /// Outcome of writing to that target, once attempted.
        applied: Option<Result<String, String>>,
    },
}

pub struct App {
    pub index: Index,
    pub filter: Filter,
    pub sort: Sort,
    pub query: String,
    pub searching: bool,
    pub rows: Vec<Row>,
    pub expanded: HashSet<String>,
    /// Chosen mods, keyed by full name so a mod stays selected wherever it
    /// appears in the tree.
    pub selected: HashSet<String>,
    pub cursor: usize,
    pub status: String,
    pub log: Vec<String>,
    pub busy: bool,
    pub loading: bool,
    pub modal: Modal,
    pub layout: Option<Layout>,
    pub install_dir_hint: String,
    /// Where the exported mod list is written; the working directory by default.
    pub export_dir: Option<std::path::PathBuf>,
    /// Mods already present in the configured install, by full name.
    pub installed: HashMap<String, Installed>,
    /// Restrict the list to mods that are already installed.
    pub installed_only: bool,
    pub quit: bool,
    tx: Sender<Msg>,
    pub rx: Receiver<Msg>,
    /// Viewport height of the list, updated each render for paging.
    pub page: usize,
}

impl App {
    pub fn new(layout: Option<Layout>, install_dir_hint: String) -> Self {
        let mut app = App::blank(layout, install_dir_hint);
        app.rescan_installed();
        app.spawn_load(false);
        app
    }

    /// State with no catalogue loaded and no background fetch started.
    pub(crate) fn blank(layout: Option<Layout>, install_dir_hint: String) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        App {
            index: Index::new(Vec::new()),
            filter: Filter::V1Ready,
            sort: Sort::Downloads,
            query: String::new(),
            searching: false,
            rows: Vec::new(),
            expanded: HashSet::new(),
            selected: HashSet::new(),
            cursor: 0,
            status: "loading catalogue from Thunderstore…".into(),
            log: Vec::new(),
            busy: false,
            loading: true,
            modal: Modal::None,
            layout,
            install_dir_hint,
            export_dir: None,
            installed: HashMap::new(),
            installed_only: false,
            quit: false,
            tx,
            rx,
            page: 20,
        }
    }

    /// Fetches the catalogue off-thread so the UI stays responsive.
    fn spawn_load(&mut self, force: bool) {
        self.loading = true;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let res = thunderstore::load(force).map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Catalog(res));
        });
    }

    /// Re-reads the install directory. Cheap enough to run after every install.
    pub fn rescan_installed(&mut self) {
        self.installed = match &self.layout {
            Some(l) => installed::scan(l),
            None => HashMap::new(),
        };
    }

    /// The installed record for a mod, if it is present on disk.
    pub fn installed_state(&self, full_name: &str) -> Option<&Installed> {
        self.installed.get(full_name)
    }

    /// True when the installed copy is a different version to the latest.
    pub fn is_outdated(&self, full_name: &str, latest: &str) -> bool {
        match self.installed.get(full_name).and_then(|i| i.version.as_deref()) {
            Some(v) => v != latest,
            None => false,
        }
    }

    pub fn rebuild(&mut self) {
        // Showing what is installed ignores the compatibility filter: a mod on
        // disk is worth seeing whether or not it signals 1.0 support.
        let filter = if self.installed_only {
            Filter::All
        } else {
            self.filter
        };
        let mut roots = self.index.roots(filter, &self.query, self.sort);
        if self.installed_only {
            roots.retain(|&i| self.installed.contains_key(&self.index.get(i).full_name));
        }
        self.rows = build_rows(&self.index, &roots, &self.expanded);
        if self.cursor >= self.rows.len() {
            self.cursor = self.rows.len().saturating_sub(1);
        }
    }

    pub fn current(&self) -> Option<&Row> {
        self.rows.get(self.cursor)
    }

    pub fn current_mod(&self) -> Option<&Mod> {
        self.current()
            .and_then(|r| r.mod_idx)
            .map(|i| self.index.get(i))
    }

    pub fn handle_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Catalog(Ok(mods)) => {
                let total = mods.len();
                self.index = Index::new(mods);
                self.loading = false;
                self.rebuild();
                let shown = self
                    .rows
                    .iter()
                    .filter(|r| r.depth == 0)
                    .count();
                let age = thunderstore::cache_age(&thunderstore::cache_path())
                    .map(|secs| format!("  ·  catalogue {}", ago(secs)))
                    .unwrap_or_default();
                self.status = format!(
                    "{shown} mods ready for Valheim 1.0 (of {total} published){age}"
                );
            }
            Msg::Catalog(Err(e)) => {
                self.loading = false;
                self.status = format!("failed to load catalogue: {e}");
            }
            Msg::Log(line) => {
                self.status = line.clone();
                self.log.push(line);
                if self.log.len() > 200 {
                    self.log.remove(0);
                }
            }
            Msg::InstallDone(res) => {
                self.busy = false;
                match res {
                    Ok(summary) => {
                        self.status = summary.clone();
                        self.log.push(summary);
                        // Reflect the new files immediately in the list.
                        self.rescan_installed();
                        self.rebuild();
                    }
                    Err(e) => {
                        self.status = format!("install failed: {e}");
                        self.log.push(self.status.clone());
                    }
                }
            }
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        // Modals swallow input so a stray keypress cannot act on the list
        // behind them.
        match &self.modal {
            Modal::Help => {
                self.modal = Modal::None;
                return;
            }
            Modal::Export { target, .. } => {
                // `w` commits the list to the compose or env file.
                if matches!(key.code, KeyCode::Char('w')) && target.is_some() {
                    self.write_mods_to_target();
                } else {
                    self.modal = Modal::None;
                }
                return;
            }
            Modal::Confirm { editing, .. } => {
                // While the path is being typed every key belongs to the input.
                if editing.is_some() {
                    self.destination_key(key);
                    return;
                }
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                        if let Modal::Confirm { plan, .. } =
                            std::mem::replace(&mut self.modal, Modal::None)
                        {
                            self.start_install(plan);
                        }
                    }
                    KeyCode::Char('d') => {
                        let current = self
                            .layout
                            .as_ref()
                            .map(|l| l.root.display().to_string())
                            .unwrap_or_default();
                        if let Modal::Confirm { editing, .. } = &mut self.modal {
                            *editing = Some(current);
                        }
                    }
                    _ => self.modal = Modal::None,
                }
                return;
            }
            Modal::None => {}
        }

        if self.searching {
            self.search_key(key);
            return;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if ctrl => self.quit = true,
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('?') => self.modal = Modal::Help,

            KeyCode::Char('j') | KeyCode::Down => self.move_cursor(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_cursor(-1),
            KeyCode::Char('d') if ctrl => self.move_cursor(self.page as isize / 2),
            KeyCode::Char('u') if ctrl => self.move_cursor(-(self.page as isize) / 2),
            KeyCode::PageDown => self.move_cursor(self.page as isize),
            KeyCode::PageUp => self.move_cursor(-(self.page as isize)),
            KeyCode::Char('g') | KeyCode::Home => self.cursor = 0,
            KeyCode::Char('G') | KeyCode::End => {
                self.cursor = self.rows.len().saturating_sub(1)
            }

            KeyCode::Char('l') | KeyCode::Right => self.expand(),
            KeyCode::Char('h') | KeyCode::Left => self.collapse(),
            KeyCode::Tab => self.toggle_expand(),

            KeyCode::Char(' ') => self.toggle_select(),
            KeyCode::Char('c') => {
                self.selected.clear();
                self.status = "selection cleared".into();
            }
            KeyCode::Enter => self.confirm_install(),
            KeyCode::Char('e') => self.export_mod_list(),
            KeyCode::Char('i') => {
                self.installed_only = !self.installed_only;
                self.cursor = 0;
                self.rebuild();
                self.status = if self.installed_only {
                    format!("showing the {} mods installed here", self.installed.len())
                } else {
                    "showing all mods".into()
                };
            }

            KeyCode::Char('/') => {
                self.searching = true;
                self.query.clear();
            }
            KeyCode::Char('f') => {
                self.filter = self.filter.next();
                self.cursor = 0;
                self.rebuild();
                self.status = format!("filter: {} — {}", self.filter.label(), self.filter.describe());
            }
            KeyCode::Char('s') => {
                self.sort = self.sort.next();
                self.cursor = 0;
                self.rebuild();
                self.status = format!("sorted by {}", self.sort.label());
            }
            // Ignored while a fetch is already in flight.
            KeyCode::Char('r') if !self.loading => {
                self.status = "refreshing catalogue…".into();
                self.spawn_load(true);
            }
            _ => {}
        }
    }

    /// Text input for the install destination inside the confirm modal.
    fn destination_key(&mut self, key: KeyEvent) {
        let Modal::Confirm { editing, .. } = &mut self.modal else {
            return;
        };
        let Some(buf) = editing else { return };
        match key.code {
            KeyCode::Esc => *editing = None,
            KeyCode::Backspace => {
                buf.pop();
            }
            KeyCode::Char(c) => buf.push(c),
            KeyCode::Enter => {
                let typed = buf.trim().to_string();
                *editing = None;
                self.set_destination(&typed);
            }
            _ => {}
        }
    }

    /// Applies a new install destination and remembers it for next time.
    fn set_destination(&mut self, path: &str) {
        if path.is_empty() {
            return;
        }
        match Layout::resolve(std::path::Path::new(path)) {
            Ok(layout) => {
                self.install_dir_hint = layout.plugins().display().to_string();
                self.layout = Some(layout);
                self.rescan_installed();
                self.rebuild();

                let mut cfg = crate::config::Config::load();
                cfg.install_dir = Some(std::path::PathBuf::from(path));
                self.status = match cfg.save() {
                    Ok(()) => format!("installing into {path} (saved for next time)"),
                    Err(e) => format!("installing into {path} (could not save: {e:#})"),
                };
            }
            Err(e) => self.status = format!("{e:#}"),
        }
    }

    fn search_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.searching = false;
                self.query.clear();
                self.rebuild();
            }
            KeyCode::Enter => self.searching = false,
            KeyCode::Backspace => {
                self.query.pop();
                self.cursor = 0;
                self.rebuild();
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                self.cursor = 0;
                self.rebuild();
            }
            _ => {}
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        let max = self.rows.len() as isize - 1;
        self.cursor = (self.cursor as isize + delta).clamp(0, max) as usize;
    }

    fn expand(&mut self) {
        if let Some(row) = self.current() {
            if row.has_children && !row.expanded {
                let key = row.key.clone();
                self.expanded.insert(key);
                self.rebuild();
            } else if row.has_children {
                // Already open: step onto the first dependency.
                self.move_cursor(1);
            }
        }
    }

    fn collapse(&mut self) {
        let Some(row) = self.current() else { return };
        if row.expanded {
            let key = row.key.clone();
            self.expanded.remove(&key);
            self.rebuild();
        } else if row.depth > 0 {
            // Jump to the parent so h walks back up the tree.
            let depth = row.depth;
            if let Some(parent) = (0..self.cursor)
                .rev()
                .find(|&i| self.rows[i].depth < depth)
            {
                self.cursor = parent;
            }
        }
    }

    fn toggle_expand(&mut self) {
        let Some(row) = self.current() else { return };
        if !row.has_children {
            return;
        }
        let key = row.key.clone();
        if row.expanded {
            self.expanded.remove(&key);
        } else {
            self.expanded.insert(key);
        }
        self.rebuild();
    }

    fn toggle_select(&mut self) {
        let Some(m) = self.current_mod() else {
            self.status = "that dependency is not published on Thunderstore".into();
            return;
        };
        let key = m.full_name.clone();
        let name = m.name.clone();
        if self.selected.remove(&key) {
            self.status = format!("deselected {name}");
        } else {
            self.selected.insert(key);
            self.status = format!("selected {name} ({} total)", self.selected.len());
        }
    }

    /// Mods explicitly chosen, or the one under the cursor when none are.
    fn chosen(&self) -> Vec<usize> {
        if self.selected.is_empty() {
            return self
                .current()
                .and_then(|r| r.mod_idx)
                .into_iter()
                .collect();
        }
        let mut v: Vec<usize> = self
            .selected
            .iter()
            .filter_map(|k| self.index.lookup(k))
            .collect();
        v.sort_unstable();
        v
    }

    fn confirm_install(&mut self) {
        if self.busy || self.loading {
            return;
        }
        if self.layout.is_none() {
            self.status = format!(
                "no install directory set — pass --install-dir <valheim server root> ({})",
                self.install_dir_hint
            );
            return;
        }
        let chosen = self.chosen();
        if chosen.is_empty() {
            self.status = "nothing selected".into();
            return;
        }
        let (plan, missing) = self.index.install_plan(&chosen);
        self.modal = Modal::Confirm {
            plan,
            missing,
            editing: None,
        };
    }

    /// Renders the selection as Thunderstore dependency strings and writes them
    /// to a file, for server images configured with a mod list rather than
    /// pre-placed plugin files.
    fn export_mod_list(&mut self) {
        let chosen = self.chosen();
        if chosen.is_empty() {
            self.status = "nothing selected to export".into();
            return;
        }
        let (plan, _) = self.index.install_plan(&chosen);

        // BepInEx itself is the loader, which those images install separately
        // from their own version setting; listing it as a mod invites a clash.
        let mut bepinex = None;
        let mut mods = Vec::new();
        for &i in &plan {
            let m = self.index.get(i);
            let dep = format!("{}-{}", m.full_name, m.version);
            if m.full_name.starts_with("denikson-BepInExPack") {
                bepinex = Some(m.version.clone());
            } else {
                mods.push(dep);
            }
        }
        let count = mods.len();
        let joined = mods.join(",");

        let written = write_mod_list(&joined, bepinex.as_deref(), self.export_dir.as_deref())
            .map(|p| p.display().to_string())
            .map_err(|e| format!("{e:#}"));
        self.status = match &written {
            Ok(p) => format!("exported {count} mods to {p}"),
            Err(e) => format!("could not write the mod list: {e}"),
        };
        // Offer an explicit target file when one is configured or sitting in
        // the working directory.
        let target = crate::config::Config::load()
            .compose_file
            .filter(|p| p.is_file())
            .or_else(|| {
                // Look beside the exported file, which is the working directory
                // unless a caller overrode it.
                self.export_dir
                    .clone()
                    .or_else(|| std::env::current_dir().ok())
                    .as_deref()
                    .and_then(crate::compose::discover)
            });

        self.modal = Modal::Export {
            mods: joined,
            count,
            bepinex,
            written,
            target,
            applied: None,
        };
    }

    /// Writes MODS (and the loader version) into the chosen compose or env file.
    fn write_mods_to_target(&mut self) {
        let Modal::Export {
            mods,
            bepinex,
            target,
            applied,
            ..
        } = &mut self.modal
        else {
            return;
        };
        let Some(path) = target.clone() else { return };

        let mut vars: Vec<(&str, String)> = vec![("MODS", mods.clone())];
        if let Some(v) = bepinex.clone() {
            vars.push(("BEPINEXPACK_VERSION", v));
        }
        let service = crate::config::Config::load().compose_service;

        let result = crate::compose::write_to_file(&path, service.as_deref(), &vars)
            .map(|backup| format!("{} (backup: {})", path.display(), backup.display()))
            .map_err(|e| format!("{e:#}"));
        self.status = match &result {
            Ok(w) => format!("wrote MODS into {w}"),
            Err(e) => format!("could not update the file: {e}"),
        };
        *applied = Some(result);
    }

    fn start_install(&mut self, plan: Vec<usize>) {
        let Some(layout) = self.layout.clone() else { return };
        self.busy = true;
        let tx = self.tx.clone();
        let mods: Vec<Mod> = plan.iter().map(|&i| self.index.get(i).clone()).collect();

        std::thread::spawn(move || {
            let log = |line: String| {
                let _ = tx.send(Msg::Log(line));
            };
            let client = match install::http_client() {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(Msg::InstallDone(Err(format!("{e:#}"))));
                    return;
                }
            };

            let total = mods.len();
            let mut installed = 0usize;
            let mut kept_configs = 0usize;
            for (n, m) in mods.iter().enumerate() {
                log(format!("[{}/{}] {}", n + 1, total, m.full_name));
                match install::install(&client, m, &layout, &log) {
                    Ok(r) => {
                        installed += 1;
                        kept_configs += r.skipped_config;
                        log(format!(
                            "  installed {} {} ({} files)",
                            m.name, m.version, r.written
                        ));
                    }
                    Err(e) => {
                        let _ = tx.send(Msg::InstallDone(Err(format!(
                            "{} — {e:#} ({installed} of {total} installed)",
                            m.full_name
                        ))));
                        return;
                    }
                }
            }
            let kept = if kept_configs > 0 {
                format!(", kept {kept_configs} existing config files")
            } else {
                String::new()
            };
            let _ = tx.send(Msg::InstallDone(Ok(format!(
                "installed {installed} mods into {}{kept} — restart the server to load them",
                layout.plugins().display()
            ))));
        });
    }
}

/// Writes the dependency list as an env file next to the user, preferring the
/// working directory and falling back to the config directory.
fn write_mod_list(
    mods: &str,
    bepinex: Option<&str>,
    preferred: Option<&std::path::Path>,
) -> anyhow::Result<std::path::PathBuf> {
    let mut body = String::new();
    body.push_str("# Generated by valheim-mods-tui.\n");
    body.push_str("# For server images that install mods themselves, put these\n");
    body.push_str("# in the environment of your docker-compose service.\n");
    if let Some(v) = bepinex {
        body.push_str(&format!("BEPINEXPACK_VERSION={v}\n"));
    }
    body.push_str(&format!("MODS={mods}\n"));

    let candidates = [
        preferred.map(|d| d.join("valheim-mods.env")),
        std::env::current_dir().ok().map(|d| d.join("valheim-mods.env")),
        crate::config::config_path()
            .parent()
            .map(|d| d.join("valheim-mods.env")),
    ];
    let mut last = None;
    for path in candidates.into_iter().flatten() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(&path, &body) {
            Ok(()) => return Ok(path),
            Err(e) => last = Some(anyhow::anyhow!("{}: {e}", path.display())),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no writable location for the mod list")))
}

/// Human-readable age for the cached catalogue.
fn ago(secs: u64) -> String {
    match secs {
        0..=90 => "just fetched".into(),
        s if s < 3600 => format!("{}m old", s / 60),
        s => format!("{}h old", s / 3600),
    }
}
