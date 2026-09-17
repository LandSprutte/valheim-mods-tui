//! Application state, input handling, and background work.

use crate::config::Layout;
use crate::index::{build_rows, Filter, Index, Row, Sort};
use crate::install;
use crate::thunderstore::{self, Mod};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashSet;
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
    Confirm { plan: Vec<usize>, missing: Vec<String> },
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
    pub quit: bool,
    tx: Sender<Msg>,
    pub rx: Receiver<Msg>,
    /// Viewport height of the list, updated each render for paging.
    pub page: usize,
}

impl App {
    pub fn new(layout: Option<Layout>, install_dir_hint: String) -> Self {
        let mut app = App::blank(layout, install_dir_hint);
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

    pub fn rebuild(&mut self) {
        let roots = self.index.roots(self.filter, &self.query, self.sort);
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
            Modal::Confirm { .. } => {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                        if let Modal::Confirm { plan, .. } =
                            std::mem::replace(&mut self.modal, Modal::None)
                        {
                            self.start_install(plan);
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
        self.modal = Modal::Confirm { plan, missing };
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

/// Human-readable age for the cached catalogue.
fn ago(secs: u64) -> String {
    match secs {
        0..=90 => "just fetched".into(),
        s if s < 3600 => format!("{}m old", s / 60),
        s => format!("{}h old", s / 3600),
    }
}
