//! Ratatui rendering.

use crate::app::{App, Modal};
use crate::index::Index;
use ratatui::prelude::*;
use ratatui::widgets::{
    Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap,
};

const ACCENT: Color = Color::Rgb(122, 162, 247);
const DIM: Color = Color::Rgb(128, 135, 152);
const GOOD: Color = Color::Rgb(158, 206, 106);
const WARN: Color = Color::Rgb(224, 175, 104);
/// Background of the row under the cursor.
const CURSOR_BG: Color = Color::Rgb(54, 66, 106);

pub fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(5),    // body
        Constraint::Length(1), // status
        Constraint::Length(1), // keys
    ])
    .split(f.area());

    header(f, chunks[0], app);

    let body = Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(chunks[1]);
    list(f, body[0], app);
    detail(f, body[1], app);

    status(f, chunks[2], app);
    keys(f, chunks[3], app);

    match &app.modal {
        Modal::Help => help_modal(f),
        Modal::Confirm {
            plan,
            missing,
            editing,
        } => confirm_modal(f, app, plan, missing, editing.as_deref()),
        Modal::Export {
            mods,
            count,
            bepinex,
            written,
            target,
            applied,
        } => export_modal(
            f,
            mods,
            *count,
            bepinex.as_deref(),
            written,
            target.as_deref(),
            applied.as_ref(),
        ),
        Modal::None => {}
    }
}

fn header(f: &mut Frame, area: Rect, app: &App) {
    let roots = app.rows.iter().filter(|r| r.depth == 0).count();
    let line = Line::from(vec![
        Span::styled(
            " Valheim 1.0 Mods ",
            Style::new().fg(Color::Black).bg(ACCENT).bold(),
        ),
        Span::raw(" "),
        Span::styled(format!("{roots} mods"), Style::new().fg(Color::White)),
        Span::styled("  ·  filter ", Style::new().fg(DIM)),
        Span::styled(app.filter.label(), Style::new().fg(ACCENT)),
        Span::styled("  ·  sort ", Style::new().fg(DIM)),
        Span::styled(app.sort.label(), Style::new().fg(ACCENT)),
        Span::styled("  ·  ", Style::new().fg(DIM)),
        Span::styled(
            format!("{} selected", app.selected.len()),
            Style::new().fg(if app.selected.is_empty() { DIM } else { GOOD }),
        ),
        Span::styled("  ·  ", Style::new().fg(DIM)),
        Span::styled(
            if app.installed_only {
                format!("{} installed (only these)", app.installed.len())
            } else {
                format!("{} installed", app.installed.len())
            },
            Style::new().fg(if app.installed.is_empty() { DIM } else { GOOD }),
        ),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn list(f: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(DIM))
        .title(Span::styled(
            if app.rows.is_empty() {
                " mods ".to_string()
            } else {
                format!(" mods  {}/{} ", app.cursor + 1, app.rows.len())
            },
            Style::new().fg(ACCENT),
        ));
    let inner = block.inner(area);
    app.page = inner.height.max(1) as usize;

    if app.loading {
        f.render_widget(
            Paragraph::new("\n  fetching the Thunderstore catalogue…")
                .style(Style::new().fg(DIM))
                .block(block),
            area,
        );
        return;
    }

    let items: Vec<ListItem> = app
        .rows
        .iter()
        .enumerate()
        .map(|(n, row)| {
            // A solid bar in the gutter marks the cursor, so the current row
            // stays obvious even where the background highlight is washed out
            // by the terminal's colour scheme.
            let on_cursor = n == app.cursor;
            let mut spans = vec![Span::styled(
                if on_cursor { "▌" } else { " " },
                Style::new().fg(ACCENT),
            )];

            match row.mod_idx {
                Some(i) => {
                    let m = app.index.get(i);
                    let picked = app.selected.contains(&m.full_name);
                    spans.push(Span::styled(
                        if picked { "[x] " } else { "[ ] " },
                        Style::new().fg(if picked { GOOD } else { DIM }),
                    ));
                    spans.push(Span::styled(row.prefix.clone(), Style::new().fg(DIM)));
                    spans.push(Span::styled(
                        if !row.has_children {
                            "  "
                        } else if row.expanded {
                            "▾ "
                        } else {
                            "▸ "
                        },
                        Style::new().fg(ACCENT),
                    ));
                    // A status glyph in front of the name: present, or present
                    // at a different version to the latest release.
                    let state = app.installed_state(&m.full_name);
                    let outdated = app.is_outdated(&m.full_name, &m.version);
                    spans.push(match (state.is_some(), outdated) {
                        (true, true) => Span::styled("▲ ", Style::new().fg(WARN)),
                        (true, false) => Span::styled("● ", Style::new().fg(GOOD)),
                        _ => Span::raw("  "),
                    });

                    let name_colour = if on_cursor {
                        ACCENT
                    } else if row.depth == 0 {
                        Color::White
                    } else {
                        Color::Gray
                    };
                    spans.push(Span::styled(
                        m.name.clone(),
                        Style::new().fg(name_colour).bold(),
                    ));
                    spans.push(Span::styled(
                        format!(" {}", m.version),
                        Style::new().fg(DIM),
                    ));
                    if row.depth == 0 {
                        spans.push(Span::styled(
                            format!("  {}", downloads(m.downloads)),
                            Style::new().fg(DIM),
                        ));
                        spans.push(Span::styled(
                            format!("  {}", m.signal()),
                            Style::new().fg(if m.tagged_for_v1() { GOOD } else { WARN }),
                        ));
                    }
                    if row.depth == 0 {
                        if let Some(have) = state.and_then(|i| i.version.as_deref()) {
                            if have != m.version {
                                spans.push(Span::styled(
                                    format!("  have {have}"),
                                    Style::new().fg(WARN),
                                ));
                            }
                        }
                    }
                    if row.cycle {
                        spans.push(Span::styled(
                            "  ↻ already above",
                            Style::new().fg(WARN),
                        ));
                    }
                }
                None => {
                    let name = row.missing.clone().unwrap_or_default();
                    spans.push(Span::styled("[!] ", Style::new().fg(WARN)));
                    spans.push(Span::styled(row.prefix.clone(), Style::new().fg(DIM)));
                    spans.push(Span::styled("  ", Style::new()));
                    spans.push(Span::styled(name, Style::new().fg(WARN)));
                    spans.push(Span::styled(
                        "  not on Thunderstore",
                        Style::new().fg(WARN).italic(),
                    ));
                }
            }
            ListItem::new(Line::from(spans))
        })
        .collect();

    let mut state = ListState::default();
    if !app.rows.is_empty() {
        state.select(Some(app.cursor));
    }
    f.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_style(Style::new().bg(CURSOR_BG)),
        area,
        &mut state,
    );
}

fn detail(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(DIM))
        .title(Span::styled(" details ", Style::new().fg(ACCENT)));

    let Some(m) = app.current_mod() else {
        let msg = app
            .current()
            .and_then(|r| r.missing.clone())
            .map(|d| format!("\n  {d}\n\n  This dependency is not published on\n  Thunderstore and cannot be installed\n  automatically."))
            .unwrap_or_else(|| "\n  nothing here".into());
        f.render_widget(
            Paragraph::new(msg).style(Style::new().fg(WARN)).block(block),
            area,
        );
        return;
    };

    let mut lines = vec![
        Line::from(Span::styled(
            m.name.clone(),
            Style::new().fg(Color::White).bold(),
        )),
        Line::from(Span::styled(
            format!("by {}  ·  v{}", m.owner, m.version),
            Style::new().fg(DIM),
        )),
        Line::raw(""),
        Line::from(Span::raw(m.description.clone())),
        Line::raw(""),
        field("downloads", downloads(m.downloads)),
        field("rating", m.rating.to_string()),
        field("size", m.size_human()),
    ];

    // State on disk comes first: it is the thing you act on.
    match app.installed_state(&m.full_name) {
        Some(have) => {
            // An unreadable version is not evidence of being out of date, so it
            // must not be dressed up as one.
            let stale = have
                .version
                .as_deref()
                .is_some_and(|v| v != m.version.as_str());
            lines.push(Line::from(vec![
                Span::styled(
                    if stale { "▲ installed  " } else { "● installed  " },
                    Style::new().fg(if stale { WARN } else { GOOD }),
                ),
                Span::styled(
                    match &have.version {
                        Some(v) => format!("{v} in BepInEx/{}", have.location),
                        None => format!("in BepInEx/{}", have.location),
                    },
                    Style::new().fg(Color::Gray),
                ),
            ]));
            if stale {
                lines.push(Line::from(Span::styled(
                    format!("  latest is {} — enter reinstalls", m.version),
                    Style::new().fg(WARN),
                )));
            }
        }
        None => lines.push(Line::from(Span::styled(
            "○ not installed here",
            Style::new().fg(DIM),
        ))),
    }
    lines.push(Line::raw(""));

    // Spell the compatibility signals out. "tagged"/"updated" is shorthand in
    // the list; here it says what was actually checked and what it is worth.
    let released = &m.date_updated[..10.min(m.date_updated.len())];
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "works with Valheim 1.0?",
        Style::new().fg(ACCENT).bold(),
    )));
    lines.push(check(
        m.tagged_for_v1(),
        "tagged by the author for Deep North",
        "not tagged by the author for Deep North",
    ));
    lines.push(check(
        m.updated_since_v1(),
        format!("released {released}, after 1.0"),
        format!("released {released}, before 1.0"),
    ));
    lines.push(Line::from(Span::styled(
        "Thunderstore publishes no verified-compatibility flag, so these are the only signals there are.",
        Style::new().fg(DIM).italic(),
    )));

    if !m.categories.is_empty() {
        lines.push(field("categories", m.categories.join(", ")));
    }

    lines.push(Line::raw(""));
    let deps = &m.dependencies;
    lines.push(Line::from(Span::styled(
        format!("dependencies ({})", deps.len()),
        Style::new().fg(ACCENT).bold(),
    )));
    if deps.is_empty() {
        lines.push(Line::from(Span::styled("  none", Style::new().fg(DIM))));
    }
    for d in deps.iter().take(14) {
        let known = app.index.lookup(Index::dep_key(d)).is_some();
        lines.push(Line::from(vec![
            Span::styled("  • ", Style::new().fg(DIM)),
            Span::styled(
                Index::dep_key(d).to_string(),
                Style::new().fg(if known { Color::Gray } else { WARN }),
            ),
            Span::styled(
                format!(" {}", Index::dep_version(d)),
                Style::new().fg(DIM),
            ),
        ]));
    }
    if deps.len() > 14 {
        lines.push(Line::from(Span::styled(
            format!("  … and {} more", deps.len() - 14),
            Style::new().fg(DIM),
        )));
    }

    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: true }).block(block),
        area,
    );
}

/// A pass/fail line for one compatibility signal.
fn check(ok: bool, yes: impl Into<String>, no: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            if ok { "  ✓ " } else { "  ✗ " },
            Style::new().fg(if ok { GOOD } else { WARN }),
        ),
        Span::styled(
            if ok { yes.into() } else { no.into() },
            Style::new().fg(if ok { Color::Gray } else { DIM }),
        ),
    ])
}

fn field(label: &str, value: impl Into<String>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<12}"), Style::new().fg(DIM)),
        Span::styled(value.into(), Style::new().fg(Color::Gray)),
    ])
}

fn status(f: &mut Frame, area: Rect, app: &App) {
    let line = if app.searching {
        Line::from(vec![
            Span::styled(" search ", Style::new().fg(Color::Black).bg(WARN).bold()),
            Span::raw(" "),
            Span::styled(app.query.clone(), Style::new().fg(Color::White)),
            Span::styled("▏", Style::new().fg(ACCENT)),
        ])
    } else {
        let tag = if app.busy { " installing " } else { " status " };
        let colour = if app.busy { WARN } else { ACCENT };
        Line::from(vec![
            Span::styled(tag, Style::new().fg(Color::Black).bg(colour).bold()),
            Span::raw(" "),
            Span::styled(app.status.clone(), Style::new().fg(Color::Gray)),
        ])
    };
    f.render_widget(Paragraph::new(line), area);
}

fn keys(f: &mut Frame, area: Rect, app: &App) {
    let text = if app.searching {
        "type to filter   enter accept   esc clear"
    } else {
        "j/k move  h/l expand  space select  enter install  i installed  e export  / search  f filter  s sort  ? help  q quit"
    };
    f.render_widget(
        Paragraph::new(Span::styled(format!(" {text}"), Style::new().fg(DIM))),
        area,
    );
}

fn centered(f: &Frame, w: u16, h: u16) -> Rect {
    let a = f.area();
    let width = w.min(a.width.saturating_sub(4));
    let height = h.min(a.height.saturating_sub(2));
    Rect {
        x: a.x + (a.width.saturating_sub(width)) / 2,
        y: a.y + (a.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

fn help_modal(f: &mut Frame) {
    let rows = [
        ("j / k / ↓ / ↑", "move up and down the list"),
        ("l / → / tab", "expand a mod's dependency tree"),
        ("h / ←", "collapse, or jump to the parent mod"),
        ("g / G", "jump to the top or bottom"),
        ("ctrl-d / ctrl-u", "half-page down and up"),
        ("space", "select or deselect the mod"),
        ("c", "clear the whole selection"),
        ("enter", "install selection plus dependencies"),
        ("e", "export the selection as a MODS= list"),
        ("i", "show only mods already installed here"),
        ("/", "search name, author and description"),
        ("f", "cycle the 1.0 compatibility filter"),
        ("s", "cycle sort: downloads, rating, updated, name"),
        ("r", "re-fetch the catalogue from Thunderstore"),
        ("q / esc", "quit"),
    ];
    let mut lines = Vec::new();
    for (k, v) in rows {
        lines.push(Line::from(vec![
            Span::styled(format!("  {k:<18}"), Style::new().fg(ACCENT).bold()),
            Span::styled(v.to_string(), Style::new().fg(Color::Gray)),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(vec![
        Span::styled("  ● ", Style::new().fg(GOOD)),
        Span::styled("installed   ", Style::new().fg(Color::Gray)),
        Span::styled("▲ ", Style::new().fg(WARN)),
        Span::styled("installed at another version   ", Style::new().fg(Color::Gray)),
        Span::styled("○ ", Style::new().fg(DIM)),
        Span::styled("not installed", Style::new().fg(Color::Gray)),
    ]));
    for (k, v) in [
        ("tagged", "the author tagged it for the Deep North update"),
        ("updated", "it has a release dated after the 1.0 launch"),
        ("tagged + updated", "both — see the details pane for dates"),
    ] {
        lines.push(Line::from(vec![
            Span::styled(format!("  {k:<18}"), Style::new().fg(GOOD)),
            Span::styled(v.to_string(), Style::new().fg(Color::Gray)),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "  press any key to close",
        Style::new().fg(DIM).italic(),
    )));

    let area = centered(f, 74, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(ACCENT))
                .title(Span::styled(" keys ", Style::new().fg(ACCENT).bold())),
        ),
        area,
    );
}

fn confirm_modal(
    f: &mut Frame,
    app: &App,
    plan: &[usize],
    missing: &[String],
    editing: Option<&str>,
) {
    let direct = app.selected.len().max(1);
    let target = app
        .layout
        .as_ref()
        .map(|l| l.plugins().display().to_string())
        .unwrap_or_default();

    let mut lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled("  installing ", Style::new().fg(Color::Gray)),
            Span::styled(format!("{} mods", plan.len()), Style::new().fg(GOOD).bold()),
            Span::styled(
                format!(
                    " ({direct} chosen + {})",
                    plural(plan.len().saturating_sub(direct), "dependency", "dependencies")
                ),
                Style::new().fg(DIM),
            ),
        ]),
        Line::raw(""),
    ];

    match editing {
        Some(buf) => {
            lines.push(Line::from(vec![
                Span::styled("  into  ", Style::new().fg(DIM)),
                Span::styled(buf.to_string(), Style::new().fg(Color::White)),
                Span::styled("▏", Style::new().fg(ACCENT)),
            ]));
            lines.push(Line::from(Span::styled(
                "  server root, BepInEx dir or plugins dir — enter accepts, esc cancels",
                Style::new().fg(DIM).italic(),
            )));
        }
        None => {
            lines.push(Line::from(vec![
                Span::styled("  into  ", Style::new().fg(DIM)),
                Span::styled(target, Style::new().fg(Color::White)),
            ]));
            lines.push(Line::from(Span::styled(
                "         press d to install somewhere else",
                Style::new().fg(DIM).italic(),
            )));
        }
    }
    lines.push(Line::raw(""));

    for &i in plan.iter().take(10) {
        let m = app.index.get(i);
        lines.push(Line::from(vec![
            Span::styled("    • ", Style::new().fg(DIM)),
            Span::styled(m.full_name.clone(), Style::new().fg(Color::Gray)),
            Span::styled(format!(" {}", m.version), Style::new().fg(DIM)),
        ]));
    }
    if plan.len() > 10 {
        lines.push(Line::from(Span::styled(
            format!("    … and {} more", plan.len() - 10),
            Style::new().fg(DIM),
        )));
    }

    if let Some(layout) = &app.layout {
        if !layout.bepinex_present() {
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(
                "  note: no BepInEx directory here yet — it will be created.",
                Style::new().fg(WARN),
            )));
        }
    }

    if !missing.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            format!(
                "  {} not on Thunderstore, and will be skipped:",
                plural(missing.len(), "dependency is", "dependencies are")
            ),
            Style::new().fg(WARN),
        )));
        for d in missing.iter().take(4) {
            lines.push(Line::from(Span::styled(
                format!("    ! {d}"),
                Style::new().fg(WARN),
            )));
        }
    }

    lines.push(Line::raw(""));
    if editing.is_none() {
        lines.push(Line::from(vec![
            Span::styled("  enter/y ", Style::new().fg(Color::Black).bg(GOOD).bold()),
            Span::styled(" install   ", Style::new().fg(DIM)),
            Span::styled(" d ", Style::new().fg(Color::Black).bg(ACCENT).bold()),
            Span::styled(" change folder   ", Style::new().fg(DIM)),
            Span::styled(" any other key ", Style::new().fg(Color::Black).bg(DIM)),
            Span::styled(" cancel", Style::new().fg(DIM)),
        ]));
    }

    let area = centered(f, 78, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(GOOD))
                .title(Span::styled(" confirm install ", Style::new().fg(GOOD).bold())),
        ),
        area,
    );
}

/// Renders a count with the right singular or plural noun.
fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Shows the dependency-string list and where it was saved.
#[allow(clippy::too_many_arguments)]
fn export_modal(
    f: &mut Frame,
    mods: &str,
    count: usize,
    bepinex: Option<&str>,
    written: &Result<String, String>,
    target: Option<&std::path::Path>,
    applied: Option<&Result<String, String>>,
) {
    let width = 86u16;
    let wrap_at = width.saturating_sub(8) as usize;

    let mut lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled("  ", Style::new()),
            Span::styled(
                format!("{count} mods"),
                Style::new().fg(GOOD).bold(),
            ),
            Span::styled(
                " as Thunderstore dependency strings, for server images",
                Style::new().fg(Color::Gray),
            ),
        ]),
        Line::from(Span::styled(
            "  that install mods themselves (a MODS= list) rather than from files.",
            Style::new().fg(Color::Gray),
        )),
        Line::raw(""),
    ];

    if let Some(v) = bepinex {
        lines.push(Line::from(vec![
            Span::styled("  BEPINEXPACK_VERSION=", Style::new().fg(ACCENT)),
            Span::styled(v.to_string(), Style::new().fg(Color::White)),
        ]));
        lines.push(Line::raw(""));
    }

    lines.push(Line::from(Span::styled(
        "  MODS=",
        Style::new().fg(ACCENT),
    )));
    // Wrapped by hand so the list stays readable and copyable on screen.
    for chunk in wrap_chunks(mods, wrap_at) {
        lines.push(Line::from(Span::styled(
            format!("    {chunk}"),
            Style::new().fg(Color::White),
        )));
    }

    lines.push(Line::raw(""));
    match written {
        Ok(path) => lines.push(Line::from(vec![
            Span::styled("  saved to  ", Style::new().fg(DIM)),
            Span::styled(path.clone(), Style::new().fg(Color::Gray)),
        ])),
        Err(e) => lines.push(Line::from(Span::styled(
            format!("  could not save the file: {e}"),
            Style::new().fg(WARN),
        ))),
    }
    // Offer to put the list straight into the compose or env file.
    match (applied, target) {
        (Some(Ok(where_)), _) => {
            lines.push(Line::from(Span::styled(
                format!("  ✓ written into {where_}"),
                Style::new().fg(GOOD),
            )));
            lines.push(Line::from(Span::styled(
                "  recreate the container to apply: docker compose up -d --force-recreate",
                Style::new().fg(DIM).italic(),
            )));
        }
        (Some(Err(e)), _) => lines.push(Line::from(Span::styled(
            format!("  ✗ {e}"),
            Style::new().fg(WARN),
        ))),
        (None, Some(path)) => {
            lines.push(Line::from(vec![
                Span::styled("  w ", Style::new().fg(Color::Black).bg(ACCENT).bold()),
                Span::styled(
                    format!(" write MODS into {}", path.display()),
                    Style::new().fg(Color::Gray),
                ),
            ]));
            lines.push(Line::from(Span::styled(
                "      the original is kept as a .bak alongside it",
                Style::new().fg(DIM).italic(),
            )));
        }
        (None, None) => lines.push(Line::from(Span::styled(
            "  no docker-compose.yml or .env here — pass --compose <file> to write one",
            Style::new().fg(DIM).italic(),
        ))),
    }

    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "  press any other key to close",
        Style::new().fg(DIM).italic(),
    )));

    let area = centered(f, width, lines.len() as u16 + 2);
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::new().fg(ACCENT))
                .title(Span::styled(" mod list ", Style::new().fg(ACCENT).bold())),
        ),
        area,
    );
}

/// Splits a long comma-separated list into display-width chunks, breaking
/// after commas so no dependency string is cut in half.
fn wrap_chunks(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for part in text.split_inclusive(',') {
        if !line.is_empty() && line.len() + part.len() > width {
            out.push(std::mem::take(&mut line));
        }
        line.push_str(part);
    }
    if !line.is_empty() {
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn downloads(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M ↓", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.0}k ↓", n as f64 / 1_000.0)
    } else {
        format!("{n} ↓")
    }
}
