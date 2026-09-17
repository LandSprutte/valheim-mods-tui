//! Rendering and interaction tests driven through a headless backend.

use crate::app::{App, Modal, Msg};
use crate::thunderstore::Mod;
use crate::ui;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn m(full_name: &str, deps: &[&str], downloads: u64) -> Mod {
    let (owner, name) = full_name.split_once('-').unwrap();
    Mod {
        full_name: full_name.into(),
        name: name.into(),
        owner: owner.into(),
        description: format!("{name} does a thing"),
        version: "1.0.0".into(),
        download_url: String::new(),
        package_url: String::new(),
        downloads,
        rating: 10,
        file_size: 2048,
        date_updated: "2026-09-12T00:00:00Z".into(),
        categories: vec!["Mods".into()],
        dependencies: deps.iter().map(|d| format!("{d}-1.0.0")).collect(),
        deprecated: false,
    }
}

fn app() -> App {
    let mut app = App::blank(None, "unset".into());
    app.handle_msg(Msg::Catalog(Ok(vec![
        m("Dev-Torchless", &["Dev-Corelib"], 900),
        m("Dev-Corelib", &[], 500),
        m("Dev-Bigpack", &["Dev-Corelib", "Dev-Ghost"], 100),
    ])));
    app
}

fn screen(app: &mut App) -> String {
    let mut term = Terminal::new(TestBackend::new(110, 30)).unwrap();
    term.draw(|f| ui::draw(f, app)).unwrap();
    let buf = term.backend().buffer().clone();
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn the_list_renders_mods_sorted_by_downloads() {
    let mut app = app();
    let out = screen(&mut app);
    assert!(out.contains("Torchless"), "mod names must be listed:\n{out}");
    assert!(out.contains("Valheim 1.0 Mods"));
    assert!(out.contains("3 mods"));
    let torch = out.find("Torchless").unwrap();
    let core = out.find("Corelib").unwrap();
    assert!(torch < core, "higher-download mods sort first");
}

#[test]
fn arrow_keys_and_hjkl_move_the_same_cursor() {
    let mut app = app();
    assert_eq!(app.cursor, 0);
    app.on_key(key(KeyCode::Char('j')));
    assert_eq!(app.cursor, 1);
    app.on_key(key(KeyCode::Down));
    assert_eq!(app.cursor, 2);
    app.on_key(key(KeyCode::Char('k')));
    assert_eq!(app.cursor, 1);
    app.on_key(key(KeyCode::Up));
    assert_eq!(app.cursor, 0);
    // The cursor must not run off either end of the list.
    app.on_key(key(KeyCode::Char('k')));
    assert_eq!(app.cursor, 0);
    app.on_key(key(KeyCode::Char('G')));
    assert_eq!(app.cursor, app.rows.len() - 1);
    app.on_key(key(KeyCode::Char('j')));
    assert_eq!(app.cursor, app.rows.len() - 1);
}

#[test]
fn l_expands_the_dependency_tree_and_h_collapses_it() {
    let mut app = app();
    let before = app.rows.len();

    app.on_key(key(KeyCode::Char('l')));
    assert_eq!(app.rows.len(), before + 1, "the dependency should appear");
    assert_eq!(app.rows[1].depth, 1);

    let out = screen(&mut app);
    assert!(out.contains("└─"), "tree connector expected:\n{out}");
    assert!(out.contains("▾"), "expanded marker expected");

    app.on_key(key(KeyCode::Char('h')));
    assert_eq!(app.rows.len(), before, "the subtree should collapse again");
}

#[test]
fn h_walks_up_to_the_parent_when_already_collapsed() {
    let mut app = app();
    app.on_key(key(KeyCode::Char('l'))); // expand root
    app.on_key(key(KeyCode::Char('j'))); // step onto the dependency
    assert_eq!(app.cursor, 1);
    app.on_key(key(KeyCode::Char('h')));
    assert_eq!(app.cursor, 0, "h on a leaf returns to its parent");
}

#[test]
fn space_toggles_selection_and_shows_a_checkbox() {
    let mut app = app();
    app.on_key(key(KeyCode::Char(' ')));
    assert!(app.selected.contains("Dev-Torchless"));

    let out = screen(&mut app);
    assert!(out.contains("[x]"), "selected rows show a filled box:\n{out}");
    assert!(out.contains("1 selected"));

    app.on_key(key(KeyCode::Char(' ')));
    assert!(app.selected.is_empty());
}

#[test]
fn selection_follows_the_mod_across_the_tree() {
    let mut app = app();
    // Select Corelib as a nested dependency of Torchless.
    app.on_key(key(KeyCode::Char('l')));
    app.on_key(key(KeyCode::Char('j')));
    app.on_key(key(KeyCode::Char(' ')));
    assert!(app.selected.contains("Dev-Corelib"));

    // Its own root row must now read as selected too.
    app.on_key(key(KeyCode::Char('h')));
    app.on_key(key(KeyCode::Char('h')));
    let root_row = app
        .rows
        .iter()
        .position(|r| r.mod_idx.map(|i| app.index.get(i).full_name == "Dev-Corelib").unwrap_or(false))
        .unwrap();
    assert!(app.selected.contains(&app.index.get(app.rows[root_row].mod_idx.unwrap()).full_name));
}

#[test]
fn enter_opens_a_confirmation_listing_dependencies() {
    let mut app = app();
    // An install directory is required before anything can be written.
    app.on_key(key(KeyCode::Enter));
    assert!(matches!(app.modal, Modal::None));
    assert!(app.status.contains("--install-dir"), "got: {}", app.status);

    let dir = std::env::temp_dir().join(format!("vmt-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    app.layout = Some(crate::config::Layout::resolve(&dir).unwrap());

    app.on_key(key(KeyCode::Char(' '))); // select Torchless
    app.on_key(key(KeyCode::Enter));

    match &app.modal {
        Modal::Confirm { plan, .. } => {
            let names: Vec<&str> =
                plan.iter().map(|&i| app.index.get(i).full_name.as_str()).collect();
            assert_eq!(names, ["Dev-Corelib", "Dev-Torchless"]);
        }
        _ => panic!("enter should raise a confirmation"),
    }

    let out = screen(&mut app);
    assert!(out.contains("confirm install"), "{out}");
    assert!(out.contains("2 mods"));

    // Any key other than enter/y backs out without installing.
    app.on_key(key(KeyCode::Char('n')));
    assert!(matches!(app.modal, Modal::None));
    assert!(!app.busy);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn unpublished_dependencies_are_shown_but_not_selectable() {
    let mut app = app();
    app.on_key(key(KeyCode::Char('G'))); // Bigpack, lowest downloads
    app.on_key(key(KeyCode::Char('l')));
    app.on_key(key(KeyCode::Char('j')));
    app.on_key(key(KeyCode::Char('j'))); // onto Dev-Ghost
    assert!(app.current().unwrap().mod_idx.is_none());

    app.on_key(key(KeyCode::Char(' ')));
    assert!(app.selected.is_empty());
    assert!(app.status.contains("not published"), "got: {}", app.status);

    let out = screen(&mut app);
    assert!(out.contains("not on Thunderstore"), "{out}");
}

#[test]
fn search_filters_the_list_and_escape_restores_it() {
    let mut app = app();
    let all = app.rows.len();

    app.on_key(key(KeyCode::Char('/')));
    for c in "torch".chars() {
        app.on_key(key(KeyCode::Char(c)));
    }
    assert_eq!(app.rows.len(), 1);
    let out = screen(&mut app);
    assert!(out.contains("search"), "{out}");

    app.on_key(key(KeyCode::Esc));
    assert_eq!(app.rows.len(), all);
    assert!(!app.searching);
}

#[test]
fn typing_in_search_never_triggers_an_install() {
    let mut app = app();
    app.on_key(key(KeyCode::Char('/')));
    // 'q' would quit and ' ' would select outside of search mode.
    for c in "q f s".chars() {
        app.on_key(key(KeyCode::Char(c)));
    }
    assert!(!app.quit);
    assert!(app.selected.is_empty());
    assert_eq!(app.query, "q f s");
}

#[test]
fn the_help_modal_opens_and_any_key_closes_it() {
    let mut app = app();
    app.on_key(key(KeyCode::Char('?')));
    let out = screen(&mut app);
    assert!(out.contains("keys"), "{out}");
    assert!(out.contains("space"));
    app.on_key(key(KeyCode::Char('x')));
    assert!(matches!(app.modal, Modal::None));
}

#[test]
fn the_filter_cycles_through_every_compatibility_view() {
    let mut app = app();
    let mut seen = Vec::new();
    for _ in 0..4 {
        seen.push(app.filter.label());
        app.on_key(key(KeyCode::Char('f')));
    }
    assert_eq!(seen, ["1.0 ready", "tagged only", "updated only", "all mods"]);
    assert_eq!(app.filter.label(), "1.0 ready", "the cycle must return home");
}

#[test]
fn a_narrow_terminal_still_renders() {
    let mut app = app();
    let mut term = Terminal::new(TestBackend::new(40, 12)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    app.on_key(key(KeyCode::Char('?')));
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
}

#[test]
fn an_empty_catalogue_renders_without_panicking() {
    let mut app = App::blank(None, "unset".into());
    app.handle_msg(Msg::Catalog(Ok(vec![])));
    let out = screen(&mut app);
    assert!(out.contains("0 mods"), "{out}");
    // Navigation and selection on an empty list must be harmless.
    app.on_key(key(KeyCode::Char('j')));
    app.on_key(key(KeyCode::Char(' ')));
    app.on_key(key(KeyCode::Enter));
    assert!(!app.quit);
}

#[test]
fn a_failed_catalogue_fetch_is_surfaced_not_swallowed() {
    let mut app = App::blank(None, "unset".into());
    app.handle_msg(Msg::Catalog(Err("connection refused".into())));
    assert!(!app.loading);
    let out = screen(&mut app);
    assert!(out.contains("connection refused"), "{out}");
}
