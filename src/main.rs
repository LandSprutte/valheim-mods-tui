//! A terminal browser for Valheim 1.0 mods on Thunderstore.

mod app;
mod compose;
mod config;
mod detect;
mod index;
mod install;
mod installed;
mod thunderstore;
mod update;
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
  valheim-mods-tui --update

options:
  --install-dir <path>   Valheim server root, its BepInEx directory, or the
                         plugins directory. Saved for subsequent runs.
  --update               Replace this binary with the newest published build.
  --update-source <src>  Where updates come from: a GitHub \"owner/repo\" whose
                         releases carry per-platform assets, or a direct URL.
                         Saved, so --update alone works afterwards.
  --force                With --update, reinstall even if the version matches.
  --compose <file>       docker-compose.yml or .env that the exported MODS
                         list is written into. Saved for subsequent runs.
  --compose-service <s>  Which compose service to edit (default: the first with
                         an environment block).
  --print-config         Show the resolved paths and exit.
  -V, --version          Print the version and target triple.
  -h, --help             Show this help.

Paths and the update source are remembered in the config file, so each only
needs to be passed once. VALHEIM_INSTALL_DIR overrides the install directory
for a single run.
";

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mut cli_dir: Option<PathBuf> = None;
    let mut print_config = false;
    let mut do_update = false;
    let mut force = false;
    let mut cli_update_source: Option<String> = None;
    let mut cli_compose: Option<PathBuf> = None;
    let mut cli_service: Option<String> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--install-dir" => {
                cli_dir = args.next().map(PathBuf::from);
                if cli_dir.is_none() {
                    eprintln!("--install-dir needs a path");
                    std::process::exit(2);
                }
            }
            "--update" => do_update = true,
            "--force" => force = true,
            "--update-source" => {
                cli_update_source = args.next();
                if cli_update_source.is_none() {
                    eprintln!("--update-source needs a GitHub \"owner/repo\" or a URL");
                    std::process::exit(2);
                }
            }
            "--compose" => {
                cli_compose = args.next().map(PathBuf::from);
                if cli_compose.is_none() {
                    eprintln!("--compose needs a path to a docker-compose.yml or .env");
                    std::process::exit(2);
                }
            }
            "--compose-service" => {
                cli_service = args.next();
                if cli_service.is_none() {
                    eprintln!("--compose-service needs a service name");
                    std::process::exit(2);
                }
            }
            "--print-config" => print_config = true,
            "-V" | "--version" => {
                println!("valheim-mods-tui {} ({})", update::VERSION, update::TARGET);
                return Ok(());
            }
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
    // Values given on the command line become the new defaults.
    let mut dirty = false;
    if let Some(dir) = cli_dir {
        cfg.install_dir = Some(dir);
        dirty = true;
    }
    if let Some(path) = cli_compose {
        cfg.compose_file = Some(path);
        dirty = true;
    }
    if let Some(service) = cli_service {
        cfg.compose_service = Some(service);
        dirty = true;
    }
    if let Some(src) = cli_update_source {
        // Validated before saving so a typo cannot be persisted.
        update::parse_source(&src)?;
        cfg.update_source = Some(src);
        dirty = true;
    }
    if dirty {
        if let Err(e) = cfg.save() {
            eprintln!("warning: could not save config: {e:#}");
        }
    }

    if do_update {
        let source = std::env::var("VALHEIM_MODS_TUI_UPDATE_SOURCE")
            .ok()
            .or_else(|| cfg.update_source.clone());
        return match source {
            Some(src) => update::run(&src, force),
            None => {
                eprintln!("{}", update::unconfigured_hint());
                std::process::exit(2);
            }
        };
    }

    let chosen = std::env::var("VALHEIM_INSTALL_DIR")
        .ok()
        .map(PathBuf::from)
        .or_else(|| cfg.install_dir.clone());

    let mut layout = None;
    // Empty means nothing was configured at all, as opposed to configured but
    // unusable — the two need different advice.
    let mut hint = String::new();
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
        println!("version:      {} ({})", update::VERSION, update::TARGET);
        println!("config file:  {}", config::config_path().display());
        println!(
            "update src:   {}",
            cfg.update_source.as_deref().unwrap_or("not configured")
        );
        println!(
            "compose file: {}",
            cfg.compose_file
                .as_deref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "not configured".into())
        );
        println!("cache file:   {}", thunderstore::cache_path().display());
        match &layout {
            Some(l) => {
                println!("server root:  {}", l.root.display());
                println!("plugins dir:  {}", l.plugins().display());
                println!("bepinex:      {}", if l.bepinex_present() { "present" } else { "not installed yet" });
            }
            None => println!(
                "install dir:  {}",
                if hint.is_empty() { "not configured" } else { &hint }
            ),
        }
        let found = detect::detect();
        if found.is_empty() {
            println!("detected:     nothing in the usual places for this system");
        } else {
            println!("detected:");
            for install in found {
                println!(
                    "  {} {}  ({}{})",
                    if install.has_bepinex { "●" } else { "○" },
                    install.path.display(),
                    install.label,
                    if install.has_bepinex {
                        ""
                    } else {
                        ", no BepInEx"
                    }
                );
            }
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
