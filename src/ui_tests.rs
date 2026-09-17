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
fn the_install_destination_can_be_changed_from_the_confirm_modal() {
    let first = std::env::temp_dir().join(format!("vmt-dest-a-{}", std::process::id()));
    let second = std::env::temp_dir().join(format!("vmt-dest-b-{}", std::process::id()));
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();

    let mut app = app();
    app.layout = Some(crate::config::Layout::resolve(&first).unwrap());
    app.on_key(key(KeyCode::Char(' ')));
    app.on_key(key(KeyCode::Enter));

    // The default destination is shown, with a way to change it.
    let out = screen(&mut app);
    assert!(out.contains("press d to install somewhere else"), "{out}");

    app.on_key(key(KeyCode::Char('d')));
    let out = screen(&mut app);
    assert!(out.contains("esc cancels"), "input mode expected:\n{out}");

    // Replace the path with the second directory.
    for _ in 0..first.display().to_string().len() {
        app.on_key(key(KeyCode::Backspace));
    }
    for c in second.display().to_string().chars() {
        app.on_key(key(KeyCode::Char(c)));
    }
    app.on_key(key(KeyCode::Enter));

    assert_eq!(app.layout.as_ref().unwrap().root, second);
    // Still on the confirm step — changing the folder must not install.
    assert!(matches!(app.modal, Modal::Confirm { .. }));
    assert!(!app.busy);

    std::fs::remove_dir_all(&first).ok();
    std::fs::remove_dir_all(&second).ok();
}

#[test]
fn a_bad_destination_is_reported_and_the_old_one_kept() {
    let good = std::env::temp_dir().join(format!("vmt-dest-c-{}", std::process::id()));
    std::fs::create_dir_all(&good).unwrap();
    let mut app = app();
    app.layout = Some(crate::config::Layout::resolve(&good).unwrap());
    app.on_key(key(KeyCode::Char(' ')));
    app.on_key(key(KeyCode::Enter));
    app.on_key(key(KeyCode::Char('d')));

    for _ in 0..good.display().to_string().len() {
        app.on_key(key(KeyCode::Backspace));
    }
    for c in "/no/such/place/anywhere".chars() {
        app.on_key(key(KeyCode::Char(c)));
    }
    app.on_key(key(KeyCode::Enter));

    assert!(app.status.contains("does not exist"), "got: {}", app.status);
    assert_eq!(app.layout.as_ref().unwrap().root, good);
    std::fs::remove_dir_all(&good).ok();
}

#[test]
fn typing_a_destination_never_triggers_the_install() {
    let dir = std::env::temp_dir().join(format!("vmt-dest-d-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = app();
    app.layout = Some(crate::config::Layout::resolve(&dir).unwrap());
    app.on_key(key(KeyCode::Char(' ')));
    app.on_key(key(KeyCode::Enter));
    app.on_key(key(KeyCode::Char('d')));

    // 'y' would confirm the install outside of input mode.
    for c in "y/tmp".chars() {
        app.on_key(key(KeyCode::Char(c)));
    }
    assert!(!app.busy, "install must not start while typing a path");
    app.on_key(key(KeyCode::Esc));
    assert!(matches!(app.modal, Modal::Confirm { .. }), "esc leaves input, not the modal");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn w_writes_the_mod_list_into_a_compose_file() {
    let dir = std::env::temp_dir().join(format!("vmt-compose-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let compose = dir.join("docker-compose.yml");
    std::fs::write(
        &compose,
        "services:\n  valheim:\n    image: x\n    environment:\n      SERVER_NAME: \"mine\"\n",
    )
    .unwrap();

    let mut app = App::blank(None, "unset".into());
    app.export_dir = Some(dir.clone());
    let mut jotunn = m("ValheimModding-Jotunn", &[], 999);
    jotunn.version = "2.30.0".into();
    app.handle_msg(Msg::Catalog(Ok(vec![jotunn])));

    app.on_key(key(KeyCode::Char(' ')));
    app.on_key(key(KeyCode::Char('e')));

    // The target is only offered, never written without the explicit key.
    match &app.modal {
        Modal::Export { target, applied, .. } => {
            assert!(target.is_some(), "a compose file beside us should be offered");
            assert!(applied.is_none(), "nothing may be written before w");
        }
        _ => panic!("expected the export modal"),
    }
    let before = std::fs::read_to_string(&compose).unwrap();
    assert!(!before.contains("MODS"));

    app.on_key(key(KeyCode::Char('w')));

    let after = std::fs::read_to_string(&compose).unwrap();
    assert!(after.contains("MODS: \"ValheimModding-Jotunn-2.30.0\""), "{after}");
    assert!(after.contains("SERVER_NAME: \"mine\""), "other keys must survive");
    // The original is recoverable.
    let backup = std::fs::read_to_string(dir.join("docker-compose.yml.bak")).unwrap();
    assert_eq!(backup, before);
    assert!(screen(&mut app).contains("written into"), "the result must be shown");

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
fn the_cursor_row_is_marked_in_the_gutter() {
    let mut app = app();
    let out = screen(&mut app);
    let rows: Vec<&str> = out.lines().collect();

    // Exactly one bar, and it sits on the first mod row.
    assert_eq!(out.matches('\u{258c}').count(), 1, "one cursor bar expected:\n{out}");
    let marked = rows.iter().find(|l| l.contains('\u{258c}')).unwrap();
    assert!(marked.contains("Torchless"), "bar must sit on the cursor row: {marked}");

    // And it follows the cursor.
    app.on_key(key(KeyCode::Char('j')));
    let out = screen(&mut app);
    let marked = out.lines().find(|l| l.contains('\u{258c}')).unwrap();
    assert!(marked.contains("Corelib"), "bar did not follow the cursor: {marked}");
}

#[test]
fn the_header_shows_the_cursor_position() {
    let mut app = app();
    assert!(screen(&mut app).contains("1/3"));
    app.on_key(key(KeyCode::Char('G')));
    assert!(screen(&mut app).contains("3/3"));
}

#[test]
fn the_detail_pane_spells_out_what_the_1_0_signals_mean() {
    let mut app = App::blank(None, "unset".into());
    let mut tagged_only = m("Dev-Tagged", &[], 10);
    tagged_only.date_updated = "2024-01-01T00:00:00Z".into();
    tagged_only.categories = vec![crate::thunderstore::V1_CATEGORY.into()];
    app.handle_msg(Msg::Catalog(Ok(vec![tagged_only])));

    let out = screen(&mut app);
    assert!(out.contains("works with Valheim 1.0?"), "{out}");
    // The matched signal is explained in words, not just labelled.
    assert!(out.contains("tagged by the author for Deep North"), "{out}");
    // And so is the one that did not match, with its date.
    assert!(out.contains("released 2024-01-01, before 1.0"), "{out}");
    assert!(out.contains('\u{2713}') && out.contains('\u{2717}'), "tick and cross expected:\n{out}");
}

#[test]
fn the_help_modal_explains_the_1_0_column() {
    let mut app = app();
    app.on_key(key(KeyCode::Char('?')));
    let out = screen(&mut app);
    assert!(out.contains("Deep North update"), "{out}");
    assert!(out.contains("after the 1.0 launch"), "{out}");
    assert!(out.contains("installed at another version"), "{out}");
}

#[test]
fn e_exports_the_selection_as_thunderstore_dependency_strings() {
    let dir = std::env::temp_dir().join(format!("vmt-export-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let mut app = App::blank(None, "unset".into());
    app.export_dir = Some(dir.clone());
    let mut bepinex = m("denikson-BepInExPack_Valheim", &[], 500);
    bepinex.version = "5.4.2350".into();
    let mut jotunn = m("ValheimModding-Jotunn", &["denikson-BepInExPack_Valheim"], 999);
    jotunn.version = "2.30.0".into();
    app.handle_msg(Msg::Catalog(Ok(vec![jotunn, bepinex])));

    // Select Jotunn; its dependency must come along.
    app.on_key(key(KeyCode::Char(' ')));
    app.on_key(key(KeyCode::Char('e')));

    let out = screen(&mut app);
    assert!(out.contains("ValheimModding-Jotunn-2.30.0"), "{out}");
    // BepInEx is the loader, surfaced separately rather than as a mod.
    assert!(out.contains("BEPINEXPACK_VERSION=5.4.2350"), "{out}");
    assert!(!out.contains("MODS=denikson"), "BepInEx must not be in the MODS list:\n{out}");

    let file = std::fs::read_to_string(dir.join("valheim-mods.env")).unwrap();
    assert!(file.contains("MODS=ValheimModding-Jotunn-2.30.0"), "{file}");
    assert!(file.contains("BEPINEXPACK_VERSION=5.4.2350"), "{file}");
    assert!(
        !file.contains("MODS=denikson-BepInExPack_Valheim"),
        "loader leaked into the mod list:\n{file}"
    );

    app.on_key(key(KeyCode::Char('x')));
    assert!(matches!(app.modal, Modal::None));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn exporting_with_nothing_selected_is_refused() {
    let mut app = App::blank(None, "unset".into());
    app.handle_msg(Msg::Catalog(Ok(vec![])));
    app.on_key(key(KeyCode::Char('e')));
    assert!(matches!(app.modal, Modal::None));
    assert!(app.status.contains("nothing selected"), "got: {}", app.status);
}

/// Builds an app whose install directory already contains one of the mods.
fn app_with_install(dir: &std::path::Path, folder: &str, version: Option<&str>) -> App {
    let plugins = dir.join("BepInEx").join("plugins").join(folder);
    std::fs::create_dir_all(&plugins).unwrap();
    if let Some(v) = version {
        std::fs::write(
            plugins.join("manifest.json"),
            format!(r#"{{"name":"x","version_number":"{v}"}}"#),
        )
        .unwrap();
    }
    let mut app = App::blank(
        Some(crate::config::Layout::resolve(dir).unwrap()),
        "set".into(),
    );
    app.rescan_installed();
    app.handle_msg(Msg::Catalog(Ok(vec![
        m("Dev-Torchless", &[], 900),
        m("Dev-Corelib", &[], 500),
    ])));
    app
}

#[test]
fn installed_mods_are_marked_in_the_list_and_details() {
    let dir = std::env::temp_dir().join(format!("vmt-inst-a-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    // m() builds every mod at version 1.0.0, so this copy is current.
    let mut app = app_with_install(&dir, "Dev-Torchless", Some("1.0.0"));

    let out = screen(&mut app);
    assert!(out.contains("● Torchless"), "installed mod must be marked:\n{out}");
    assert!(out.contains("○ not installed") || out.contains("● installed"), "{out}");
    assert!(out.contains("1 installed"), "header should count them:\n{out}");
    assert!(out.contains("● installed  1.0.0 in BepInEx/plugins"), "{out}");

    // The uninstalled one says so plainly.
    app.on_key(key(KeyCode::Char('j')));
    let out = screen(&mut app);
    assert!(out.contains("○ not installed here"), "{out}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_different_installed_version_is_flagged_rather_than_shown_as_current() {
    let dir = std::env::temp_dir().join(format!("vmt-inst-b-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut app = app_with_install(&dir, "Dev-Torchless", Some("0.9.0"));

    let out = screen(&mut app);
    assert!(out.contains("▲ Torchless"), "stale install needs its own mark:\n{out}");
    assert!(out.contains("have 0.9.0"), "{out}");
    assert!(out.contains("▲ installed  0.9.0"), "{out}");
    assert!(out.contains("latest is 1.0.0"), "{out}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn i_narrows_the_list_to_installed_mods() {
    let dir = std::env::temp_dir().join(format!("vmt-inst-c-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut app = app_with_install(&dir, "Dev-Corelib", Some("1.0.0"));
    assert_eq!(app.rows.len(), 2);

    app.on_key(key(KeyCode::Char('i')));
    assert_eq!(app.rows.len(), 1, "only the installed mod should remain");
    assert_eq!(app.index.get(app.rows[0].mod_idx.unwrap()).full_name, "Dev-Corelib");
    assert!(screen(&mut app).contains("only these"), "the filter must be visible");

    app.on_key(key(KeyCode::Char('i')));
    assert_eq!(app.rows.len(), 2, "toggling back restores the full list");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_mod_without_a_manifest_still_counts_as_installed() {
    let dir = std::env::temp_dir().join(format!("vmt-inst-d-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut app = app_with_install(&dir, "Dev-Torchless", None);

    // Unknown version must not be reported as out of date.
    let out = screen(&mut app);
    assert!(out.contains("● Torchless"), "{out}");
    assert!(!out.contains("have "), "no version to compare against:\n{out}");
    assert!(out.contains("● installed  in BepInEx/plugins"), "{out}");
    assert!(!out.contains("latest is"), "unknown version is not staleness:\n{out}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn with_no_install_directory_nothing_is_marked_installed() {
    let mut app = app();
    assert!(app.installed.is_empty());
    let out = screen(&mut app);
    assert!(out.contains("0 installed"), "{out}");
    assert!(!out.contains("●"), "nothing can be installed without a target:\n{out}");
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
fn the_help_modal_fits_a_short_terminal() {
    let mut app = app();
    app.on_key(key(KeyCode::Char('?')));
    let mut term = Terminal::new(TestBackend::new(96, 26)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let buf = term.backend().buffer().clone();
    let out: String = (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");
    // The closing hint is the last line, so seeing it proves nothing clipped.
    assert!(out.contains("press any key to close"), "help modal was cut off:\n{out}");
    assert!(out.contains("not installed"), "{out}");
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
