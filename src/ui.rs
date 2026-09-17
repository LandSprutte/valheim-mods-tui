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
        Modal::Confirm { plan, missing } => confirm_modal(f, app, plan, missing),
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
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn list(f: &mut Frame, area: Rect, app: &mut App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::new().fg(DIM))
        .title(Span::styled(" mods ", Style::new().fg(ACCENT)));
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
        .map(|row| {
            let mut spans = Vec::new();

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
                    spans.push(Span::styled(
                        m.name.clone(),
                        Style::new()
                            .fg(if row.depth == 0 { Color::White } else { Color::Gray })
                            .bold(),
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
            .highlight_style(Style::new().bg(Color::Rgb(41, 46, 66))),
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
        field("updated", &m.date_updated[..10.min(m.date_updated.len())]),
        field("downloads", downloads(m.downloads)),
        field("rating", m.rating.to_string()),
        field("size", m.size_human()),
        field("1.0 signal", m.signal()),
    ];

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
        "j/k move  h/l collapse/expand  space select  enter install  / search  f filter  s sort  r refresh  ? help  q quit"
    };
    f.render_widget(
        Paragraph::new(Span::styled(format!(" {text}"), Style::new().fg(DIM))),
        area,
    );
}

fn centered(f: &Frame, w: u16, h: u16) -> Rect {
    let a = f.area();
    let width = w.min(a.width.saturating_sub(4));
    let height = h.min(a.height.saturating_sub(4));
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
        ("/", "search name, author and description"),
        ("f", "cycle the 1.0 compatibility filter"),
        ("s", "cycle sort: downloads, rating, updated, name"),
        ("r", "re-fetch the catalogue from Thunderstore"),
        ("q / esc", "quit"),
    ];
    let mut lines = vec![Line::raw("")];
    for (k, v) in rows {
        lines.push(Line::from(vec![
            Span::styled(format!("  {k:<18}"), Style::new().fg(ACCENT).bold()),
            Span::styled(v.to_string(), Style::new().fg(Color::Gray)),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "  press any key to close",
        Style::new().fg(DIM).italic(),
    )));

    let area = centered(f, 66, lines.len() as u16 + 2);
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

fn confirm_modal(f: &mut Frame, app: &App, plan: &[usize], missing: &[String]) {
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
        Line::from(vec![
            Span::styled("  into  ", Style::new().fg(DIM)),
            Span::styled(target, Style::new().fg(Color::White)),
        ]),
        Line::raw(""),
    ];

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
    lines.push(Line::from(vec![
        Span::styled("  enter/y ", Style::new().fg(Color::Black).bg(GOOD).bold()),
        Span::styled(" install    ", Style::new().fg(DIM)),
        Span::styled(" any other key ", Style::new().fg(Color::Black).bg(DIM)),
        Span::styled(" cancel", Style::new().fg(DIM)),
    ]));

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

fn downloads(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M ↓", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.0}k ↓", n as f64 / 1_000.0)
    } else {
        format!("{n} ↓")
    }
}
