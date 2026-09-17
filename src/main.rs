//! A terminal browser for Valheim 1.0 mods on Thunderstore.

mod app;
mod config;
mod index;
mod install;
mod thunderstore;
mod ui;
#[cfg(test)]
mod ui_tests;

use anyhow::Result;
use app::App;
use config::{Config, Layout};
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use std::io::stdout;
use std::path::PathBuf;
use std::time::Duration;

const USAGE: &str = "\
valheim-mods-tui — browse and install Valheim 1.0 mods from Thunderstore

usage:
  valheim-mods-tui [--install-dir <path>]

options:
  --install-dir <path>   Valheim server root, its BepInEx directory, or the
                         plugins directory. Saved for subsequent runs.
  --print-config         Show the resolved paths and exit.
  -h, --help             Show this help.

The install directory is remembered in the config file, so it only needs to be
passed once. VALHEIM_INSTALL_DIR overrides it for a single run.
";

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut cli_dir: Option<PathBuf> = None;
    let mut print_config = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--install-dir" => {
                cli_dir = args.next().map(PathBuf::from);
                if cli_dir.is_none() {
                    eprintln!("--install-dir needs a path");
                    std::process::exit(2);
                }
            }
            "--print-config" => print_config = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other => {
                eprintln!("unknown argument: {other}\n\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    let mut cfg = Config::load();
    // A path given on the command line becomes the new default.
    if let Some(dir) = cli_dir {
        cfg.install_dir = Some(dir);
        if let Err(e) = cfg.save() {
            eprintln!("warning: could not save config: {e:#}");
        }
    }

    let chosen = std::env::var("VALHEIM_INSTALL_DIR")
        .ok()
        .map(PathBuf::from)
        .or_else(|| cfg.install_dir.clone());

    let mut layout = None;
    let mut hint = String::from("none configured");
    if let Some(dir) = &chosen {
        match Layout::resolve(dir) {
            Ok(l) => {
                hint = l.plugins().display().to_string();
                layout = Some(l);
            }
            Err(e) => hint = format!("{e:#}"),
        }
    }

    if print_config {
        println!("config file:  {}", config::config_path().display());
        println!("cache file:   {}", thunderstore::cache_path().display());
        match &layout {
            Some(l) => {
                println!("server root:  {}", l.root.display());
                println!("plugins dir:  {}", l.plugins().display());
                println!("bepinex:      {}", if l.bepinex_present() { "present" } else { "not installed yet" });
            }
            None => println!("install dir:  {hint}"),
        }
        return Ok(());
    }

    run(layout, hint)
}

fn run(layout: Option<Layout>, hint: String) -> Result<()> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout()))?;

    let result = event_loop(&mut term, layout, hint);

    // Restore the terminal even if the loop failed, so an error never leaves
    // the user with a broken shell.
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    term.show_cursor()?;
    result
}

fn event_loop<B: ratatui::backend::Backend>(
    term: &mut ratatui::Terminal<B>,
    layout: Option<Layout>,
    hint: String,
) -> Result<()> {
    let mut app = App::new(layout, hint);

    loop {
        term.draw(|f| ui::draw(f, &mut app))?;

        // Drain background progress before waiting on input.
        while let Ok(msg) = app.rx.try_recv() {
            app.handle_msg(msg);
        }

        // Short poll keeps the download log ticking without busy-waiting.
        if event::poll(Duration::from_millis(120))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => app.on_key(key),
                Event::Resize(_, _) => {}
                _ => {}
            }
        }

        if app.quit {
            return Ok(());
        }
    }
}
