use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Padding, Paragraph, Row, Table, Wrap},
};

use crate::{
    action::Action,
    app::{App, ConnectionState, FinderItem, FinderKind, Focus, InspectorSection, Mode, QueryTab},
};

pub fn mouse_action(column: u16, row: u16, width: u16, height: u16, app: &App) -> Action {
    if app.close_tab_dialog.is_some() {
        let area = close_dialog_area(Rect::new(0, 0, width, height));
        let (cancel, confirm) = close_dialog_buttons(area);
        if contains(confirm, column, row) {
            return Action::ConfirmCloseQueryTab;
        }
        if contains(cancel, column, row) {
            return Action::OverlayCancel;
        }
        return Action::Noop;
    }
    if app.overlay_active() {
        return Action::Noop;
    }
    let frame_area = Rect::new(0, 0, width, height);
    let outer = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(1),
    ])
    .split(frame_area);
    let content =
        Layout::vertical([Constraint::Percentage(58), Constraint::Percentage(42)]).split(outer[1]);
    let top = Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(content[0]);
    let tabs_area = editor_areas(top[1]).0;
    if row == tabs_area.y && column >= tabs_area.x && column < tabs_area.right() {
        let mut start = tabs_area.x;
        for (index, tab) in app.query_tabs.iter().enumerate() {
            let label = query_tab_label(app, index, tab);
            let end = start.saturating_add(label.chars().count() as u16);
            if column >= start && column < end {
                return Action::FocusQueryTab(index);
            }
            start = end.saturating_add(1);
        }
        return Action::FocusEditor;
    }
    if row >= top[0].y && row < top[0].bottom() {
        if column >= top[0].x && column < top[0].right() {
            if row > top[0].y {
                let visible_height = top[0].height.saturating_sub(2) as usize;
                let scroll = app
                    .explorer_selection
                    .saturating_sub(visible_height.saturating_sub(1));
                Action::ClickExplorerNode(scroll + row.saturating_sub(top[0].y + 1) as usize)
            } else {
                Action::FocusExplorer
            }
        } else {
            Action::FocusEditor
        }
    } else if row >= content[1].y {
        Action::FocusResults
    } else {
        Action::Noop
    }
}

pub fn draw(frame: &mut Frame, app: &App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(1),
        ])
        .split(frame.area());

    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            " PGIDE",
            Style::default().add_modifier(Modifier::BOLD),
        )]))
        .block(Block::default().borders(Borders::ALL)),
        outer[0],
    );

    let content = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(outer[1]);
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(content[0]);

    draw_explorer(frame, top[0], app);
    draw_editor(frame, top[1], app);
    draw_results(frame, content[1], app);
    draw_status(frame, outer[2], app);
    draw_completion(frame, top[1], app);
    if app.finder.is_some() {
        draw_finder(frame, app);
    }
    if app.save_dialog.is_some() {
        draw_save_dialog(frame, app);
    }
    if app.close_tab_dialog.is_some() {
        draw_close_tab_dialog(frame, app);
    }
    if app.help_visible {
        draw_help(frame);
    }
}

fn draw_explorer(frame: &mut Frame, area: Rect, app: &App) {
    let text = match app.connection {
        ConnectionState::Disconnected => vec![Line::from(" No connection")],
        ConnectionState::Connecting => vec![Line::from(" Connecting…")],
        ConnectionState::Connected => app
            .explorer_entries()
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let marker = if entry.expandable {
                    if entry.open { "▼" } else { "▶" }
                } else {
                    "•"
                };
                let style = if app.focus == Focus::Explorer && index == app.explorer_selection {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else if entry.depth == 2 {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default()
                };
                Line::styled(
                    format!(" {}{marker} {} ", "  ".repeat(entry.depth), entry.label),
                    style,
                )
            })
            .collect(),
    };
    let visible_height = area.height.saturating_sub(2) as usize;
    let scroll = app
        .explorer_selection
        .saturating_sub(visible_height.saturating_sub(1)) as u16;
    frame.render_widget(
        Paragraph::new(text)
            .scroll((scroll, 0))
            .block(panel_block("Explorer", app.focus == Focus::Explorer)),
        area,
    );
}

fn draw_editor(frame: &mut Frame, area: Rect, app: &App) {
    let before = &app.query[..app.cursor];
    let cursor_line = before.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let cursor_column = before
        .rsplit('\n')
        .next()
        .unwrap_or_default()
        .chars()
        .count()
        + 1;
    frame.render_widget(
        panel_block(
            &format!(
                "SQL  {}  Ln {cursor_line}, Col {cursor_column}",
                app.mode.label()
            ),
            app.focus == Focus::Editor,
        ),
        area,
    );
    let (tabs_area, sql_area) = editor_areas(area);
    let tabs = app
        .query_tabs
        .iter()
        .enumerate()
        .flat_map(|(index, tab)| {
            let active = index == app.active_query_tab;
            let label = query_tab_label(app, index, tab);
            let style = if active {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::LightBlue)
            };
            [Span::styled(label, style), Span::raw(" ")]
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(Line::from(tabs)), tabs_area);
    frame.render_widget(
        Paragraph::new(crate::sql::highlight::lines(&app.query)),
        sql_area,
    );
    if app.focus == Focus::Editor && !app.help_visible && !app.overlay_active() {
        let prefix = &app.query[..app.cursor];
        let row = prefix
            .chars()
            .filter(|character| *character == '\n')
            .count() as u16;
        let column = prefix
            .rsplit('\n')
            .next()
            .unwrap_or_default()
            .chars()
            .count() as u16;
        let x = sql_area
            .x
            .saturating_add(column)
            .min(sql_area.right().saturating_sub(1));
        let y = sql_area
            .y
            .saturating_add(row)
            .min(sql_area.bottom().saturating_sub(1));
        frame.set_cursor_position((x, y));
    }
}

fn query_tab_label(app: &App, index: usize, tab: &QueryTab) -> String {
    let name = tab
        .name
        .as_deref()
        .map_or_else(|| format!("scratch-{}", index + 1), str::to_owned);
    let modified = if index == app.active_query_tab {
        app.active_query_is_modified()
    } else {
        tab.is_modified()
    };
    format!(" {name}{} ", if modified { " ●" } else { "" })
}

fn editor_areas(area: Rect) -> (Rect, Rect) {
    let inner = area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let sections = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(inner);
    (sections[0], sections[1])
}

fn draw_results(frame: &mut Frame, area: Rect, app: &App) {
    if app.inspector_loading {
        frame.render_widget(
            Paragraph::new("Loading table metadata…")
                .block(panel_block("Table Inspector", app.focus == Focus::Results)),
            area,
        );
        return;
    }
    if app.inspector.is_some() {
        draw_table_inspector(frame, area, app);
        return;
    }
    let position = app.result.as_ref().map_or_else(String::new, |result| {
        format!(
            "  row {}/{}  col {}/{}",
            app.result_row.saturating_add(1).min(result.rows.len()),
            result.rows.len(),
            app.result_column
                .saturating_add(1)
                .min(result.columns.len()),
            result.columns.len()
        )
    });
    let block = panel_block(&format!("Results{position}"), app.focus == Focus::Results);
    if let Some(error) = &app.error {
        frame.render_widget(
            Paragraph::new(error.as_str())
                .style(Style::default().fg(Color::Red))
                .block(block),
            area,
        );
    } else if app.query_running {
        frame.render_widget(Paragraph::new("Running…").block(block), area);
    } else if let Some(result) = &app.result {
        let start_column = app.result_column.saturating_sub(1);
        let available = area.width.saturating_sub(7) as usize;
        let mut used = 0;
        let mut end_column = start_column;
        let mut widths = vec![Constraint::Length(5)];
        while end_column < result.columns.len() {
            let content_width = result
                .rows
                .iter()
                .filter_map(|row| row.get(end_column))
                .map(|value| value.chars().count())
                .max()
                .unwrap_or(0);
            let width = result.columns[end_column]
                .chars()
                .count()
                .max(content_width)
                .clamp(6, 24)
                + 2;
            if used + width > available && end_column > start_column {
                break;
            }
            used += width;
            widths.push(Constraint::Length(width as u16));
            end_column += 1;
        }
        let header = Row::new(
            std::iter::once(Cell::from("#")).chain(
                result.columns[start_column..end_column]
                    .iter()
                    .map(|value| Cell::from(value.as_str())),
            ),
        )
        .style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
        let visible_height = area.height.saturating_sub(3) as usize;
        let start_row = app
            .result_row
            .saturating_sub(visible_height.saturating_sub(1));
        let rows = result
            .rows
            .iter()
            .enumerate()
            .skip(start_row)
            .take(visible_height)
            .map(|(row_index, row)| {
                let number = Cell::from((row_index + 1).to_string())
                    .style(Style::default().fg(Color::DarkGray));
                let cells =
                    row[start_column..end_column]
                        .iter()
                        .enumerate()
                        .map(|(offset, value)| {
                            let column_index = start_column + offset;
                            let style = if row_index == app.result_row
                                && column_index == app.result_column
                            {
                                Style::default()
                                    .fg(Color::Black)
                                    .bg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD)
                            } else if row_index == app.result_row {
                                Style::default().bg(Color::Rgb(45, 48, 60))
                            } else if value == "NULL" {
                                Style::default()
                                    .fg(Color::DarkGray)
                                    .add_modifier(Modifier::ITALIC)
                            } else {
                                Style::default()
                            };
                            Cell::from(value.as_str()).style(style)
                        });
                Row::new(std::iter::once(number).chain(cells))
            });
        frame.render_widget(
            Table::new(rows, widths)
                .header(header)
                .block(block)
                .column_spacing(2),
            area,
        );
    } else {
        frame.render_widget(
            Paragraph::new("No results yet  ·  Press Space then r to run SQL")
                .style(Style::default().fg(Color::DarkGray))
                .block(block),
            area,
        );
    }
}

fn draw_table_inspector(frame: &mut Frame, area: Rect, app: &App) {
    let details = app.inspector.as_ref().expect("inspector checked by caller");
    let title = format!("Table Inspector  {}.{}", details.schema, details.name);
    frame.render_widget(panel_block(&title, app.focus == Focus::Results), area);
    let inner = area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });
    let sections = Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).split(inner);
    let tabs = [
        (InspectorSection::Overview, " Overview "),
        (InspectorSection::Columns, " Columns "),
        (InspectorSection::Constraints, " Constraints "),
        (InspectorSection::Indexes, " Indexes "),
    ];
    let tab_line = Line::from(
        tabs.into_iter()
            .flat_map(|(section, label)| {
                let style = if app.inspector_section == section {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Cyan)
                };
                [Span::styled(label, style), Span::raw(" ")]
            })
            .collect::<Vec<_>>(),
    );
    frame.render_widget(
        Paragraph::new(tab_line)
            .block(Block::default().title(" [ / ] switch · Esc close · p preview ")),
        sections[0],
    );

    match app.inspector_section {
        InspectorSection::Overview => {
            let overview = vec![
                Line::from(vec![
                    Span::styled("Qualified name  ", Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        format!("{}.{}", details.schema, details.name),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(""),
                metric_line(
                    "Estimated rows",
                    if details.estimated_rows < 0 {
                        "not analyzed".into()
                    } else {
                        details.estimated_rows.to_string()
                    },
                ),
                metric_line("Columns", details.columns.len().to_string()),
                metric_line("Constraints", details.constraints.len().to_string()),
                metric_line("Indexes", details.indexes.len().to_string()),
                Line::from(""),
                metric_line("Table size", details.table_size.clone()),
                metric_line("Index size", details.indexes_size.clone()),
                metric_line("Total size", details.total_size.clone()),
            ];
            frame.render_widget(
                Paragraph::new(overview).block(
                    Block::default()
                        .title(" PostgreSQL Storage ")
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::DarkGray))
                        .padding(Padding::uniform(1)),
                ),
                sections[1],
            );
        }
        InspectorSection::Columns => {
            let header = Row::new(["#", "Column", "Type", "Null", "Key", "Default", "Comment"])
                .style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                );
            let rows = details.columns.iter().enumerate().map(|(index, column)| {
                Row::new([
                    (index + 1).to_string(),
                    column.name.clone(),
                    column.data_type.clone(),
                    if column.nullable {
                        "YES".into()
                    } else {
                        "NO".into()
                    },
                    column.key.clone().unwrap_or_default(),
                    column.default.clone().unwrap_or_else(|| "—".into()),
                    column.comment.clone().unwrap_or_default(),
                ])
            });
            frame.render_widget(
                Table::new(
                    rows,
                    [
                        Constraint::Length(4),
                        Constraint::Length(22),
                        Constraint::Length(22),
                        Constraint::Length(6),
                        Constraint::Length(10),
                        Constraint::Percentage(35),
                        Constraint::Percentage(25),
                    ],
                )
                .header(header)
                .column_spacing(1),
                sections[1],
            );
        }
        InspectorSection::Constraints => {
            let header = Row::new(["Type", "Name", "Definition"]).style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );
            let rows = details.constraints.iter().map(|constraint| {
                Row::new([
                    constraint.kind.clone(),
                    constraint.name.clone(),
                    constraint.definition.clone(),
                ])
            });
            frame.render_widget(
                Table::new(
                    rows,
                    [
                        Constraint::Length(16),
                        Constraint::Length(28),
                        Constraint::Min(30),
                    ],
                )
                .header(header)
                .column_spacing(2),
                sections[1],
            );
        }
        InspectorSection::Indexes => {
            let header = Row::new(["Index", "PostgreSQL definition"]).style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            );
            let rows = details
                .indexes
                .iter()
                .map(|index| Row::new([index.name.clone(), index.definition.clone()]));
            frame.render_widget(
                Table::new(rows, [Constraint::Length(32), Constraint::Min(40)])
                    .header(header)
                    .column_spacing(2),
                sections[1],
            );
        }
    }
}

fn metric_line(label: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<18}"), Style::default().fg(Color::DarkGray)),
        Span::styled(value, Style::default().fg(Color::LightBlue)),
    ])
}

fn draw_completion(frame: &mut Frame, editor_area: Rect, app: &App) {
    if app.mode != Mode::Insert || app.focus != Focus::Editor || app.help_visible {
        return;
    }
    let candidates = crate::sql::completion::candidates(
        &app.query,
        app.cursor,
        &app.completion_items,
        &app.relation_items,
    );
    if candidates.is_empty() {
        return;
    }
    let editor_area = editor_areas(editor_area).1;
    let selected_completion = app.completion_index.min(candidates.len().saturating_sub(1));
    let before = &app.query[..app.cursor];
    let row = before.bytes().filter(|byte| *byte == b'\n').count() as u16;
    let column = before
        .rsplit('\n')
        .next()
        .unwrap_or_default()
        .chars()
        .count() as u16;
    let width = candidates
        .iter()
        .map(|item| item.len())
        .max()
        .unwrap_or(10)
        .max(18) as u16
        + 4;
    let height = candidates.len() as u16 + 2;
    let x = editor_area
        .x
        .saturating_add(column + 1)
        .min(editor_area.right().saturating_sub(width));
    let y = editor_area
        .y
        .saturating_add(row + 2)
        .min(editor_area.bottom().saturating_sub(height));
    let popup = Rect::new(x, y, width.min(editor_area.width), height);
    frame.render_widget(Clear, popup);
    let lines = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            Line::styled(
                format!(" {candidate}"),
                if index == selected_completion {
                    Style::default().fg(Color::Black).bg(Color::Yellow)
                } else {
                    Style::default()
                },
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .title(" Completion · Ctrl-n/p select · Enter accept ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan)),
        ),
        popup,
    );
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let connection = match app.connection {
        ConnectionState::Disconnected => "DISCONNECTED".to_owned(),
        ConnectionState::Connecting => "CONNECTING".to_owned(),
        ConnectionState::Connected => app
            .database_name
            .clone()
            .unwrap_or_else(|| "CONNECTED".into()),
    };
    let details = app
        .result
        .as_ref()
        .map(|result| {
            format!(
                " │ {} rows │ {}ms",
                result.rows.len(),
                app.elapsed().unwrap_or_default().as_millis()
            )
        })
        .unwrap_or_default();
    let hint = if app.key_sequence == Some(' ') {
        "LEADER  n New query  •  r Run  •  f Find  •  ? Help"
    } else if app.key_sequence == Some('f') {
        "LEADER f…  f Saved queries  •  h History  •  s Save as  •  Esc Cancel"
    } else if app.key_sequence == Some('b') {
        "LEADER b…  d Close current query tab  •  Esc Cancel"
    } else if app.key_sequence == Some('d') {
        "d…  d Delete current line  •  Esc Cancel"
    } else if app.key_sequence == Some('g') {
        "g…  g First line  •  t Next tab  •  T Previous tab  •  Esc Cancel"
    } else if app.mode == Mode::Insert {
        "Esc Normal  •  type to edit"
    } else {
        "1 Explorer  2 SQL  3 Results  •  Ctrl-n New  •  Ctrl-s Save  •  Space ff/fh Find"
    };
    let message = app
        .status_message
        .as_deref()
        .map_or_else(String::new, |message| format!(" │ {message}"));
    let left = format!(
        " {} │ {}{}{}",
        app.mode.label(),
        connection,
        details,
        message
    );
    let gap = (area.width as usize).saturating_sub(left.chars().count() + hint.chars().count() + 1);
    frame.render_widget(
        Paragraph::new(format!("{left}{}{hint} ", " ".repeat(gap)))
            .style(Style::default().fg(Color::Black).bg(Color::Cyan)),
        area,
    );
}

fn draw_save_dialog(frame: &mut Frame, app: &App) {
    let Some(dialog) = &app.save_dialog else {
        return;
    };
    let area = centered_fixed(68, 8, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(if dialog.save_as {
            " Save query as "
        } else {
            " Save query "
        })
        .title_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .padding(Padding::uniform(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(inner);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Cyan)),
            Span::raw(&dialog.input),
        ])),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new("Stored for this database and mirrored to .pgide/queries/")
            .style(Style::default().fg(Color::DarkGray)),
        rows[1],
    );
    frame.render_widget(
        Paragraph::new("Enter save  •  Esc cancel").style(Style::default().fg(Color::LightBlue)),
        rows[2],
    );
    frame.set_cursor_position((
        rows[0]
            .x
            .saturating_add(2 + dialog.input.chars().count() as u16),
        rows[0].y,
    ));
}

fn draw_close_tab_dialog(frame: &mut Frame, app: &App) {
    let Some(dialog) = &app.close_tab_dialog else {
        return;
    };
    let Some(tab) = app.query_tabs.get(dialog.tab_index) else {
        return;
    };
    let area = close_dialog_area(frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(" Close query tab? ")
        .title_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .padding(Padding::uniform(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let name = tab.name.as_deref().unwrap_or("Unnamed scratch buffer");
    let warning = if tab.saved_query_id.is_some() {
        if tab.is_modified() {
            "Unsaved edits will be discarded. The saved query and .sql file will remain."
        } else {
            "Only this editor tab will close. The saved query and .sql file will remain."
        }
    } else {
        "This unnamed buffer will be discarded. No saved query or file will be deleted."
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(name, Style::default().add_modifier(Modifier::BOLD)),
            Line::from(""),
            Line::styled(warning, Style::default().fg(Color::Yellow)),
        ])
        .wrap(Wrap { trim: false }),
        inner,
    );
    let (cancel, confirm) = close_dialog_buttons(area);
    frame.render_widget(
        Paragraph::new(" [ Cancel ] ").style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        cancel,
    );
    frame.render_widget(
        Paragraph::new(" [ Close tab ] ").style(
            Style::default()
                .fg(Color::White)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
        ),
        confirm,
    );
}

fn close_dialog_area(area: Rect) -> Rect {
    centered_fixed(76, 10, area)
}

fn close_dialog_buttons(area: Rect) -> (Rect, Rect) {
    let y = area.bottom().saturating_sub(3);
    (
        Rect::new(area.x.saturating_add(2), y, 12, 1),
        Rect::new(area.right().saturating_sub(17), y, 15, 1),
    )
}

fn contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x && column < area.right() && row >= area.y && row < area.bottom()
}

fn draw_finder(frame: &mut Frame, app: &App) {
    let Some(finder) = &app.finder else {
        return;
    };
    let matches = app.finder_matches();
    let title = match finder.kind {
        FinderKind::SavedQueries => " Saved queries · <leader>ff ",
        FinderKind::History => " Query history · <leader>fh ",
    };
    let area = centered_rect(82, 76, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(title)
        .title_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .padding(Padding::uniform(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let sections = Layout::vertical([
        Constraint::Length(2),
        Constraint::Percentage(42),
        Constraint::Min(4),
        Constraint::Length(1),
    ])
    .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Find  ", Style::default().fg(Color::Cyan)),
            Span::raw(&finder.input),
        ]))
        .block(Block::default().borders(Borders::BOTTOM)),
        sections[0],
    );

    let visible_rows = sections[1].height.max(1) as usize;
    let start = finder
        .selected
        .saturating_sub(visible_rows.saturating_sub(1));
    let lines = matches
        .iter()
        .enumerate()
        .skip(start)
        .take(visible_rows)
        .map(|(match_index, item_index)| {
            let item = &finder.items[*item_index];
            Line::styled(
                format!(" {}", item.label()),
                if match_index == finder.selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                },
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(if lines.is_empty() {
            vec![Line::styled(
                " No matching queries",
                Style::default().fg(Color::DarkGray),
            )]
        } else {
            lines
        })
        .block(Block::default().title(format!(" {} matches ", matches.len()))),
        sections[1],
    );

    let selected_item = matches
        .get(finder.selected)
        .and_then(|index| finder.items.get(*index));
    let preview_title = selected_item.map_or_else(
        || " SQL preview ".into(),
        |item| match item {
            FinderItem::Saved(saved) => format!(
                " SQL preview · {} · {} ",
                saved.database_name,
                std::path::Path::new(&saved.file_path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("query.sql")
            ),
            FinderItem::History(entry) => {
                let status = if entry.success { "success" } else { "failed" };
                format!(" SQL preview · {status} ")
            }
        },
    );
    let preview = selected_item.map_or_else(Vec::new, |item| {
        let mut lines = crate::sql::highlight::lines(item.sql());
        if let FinderItem::History(entry) = item
            && let Some(error) = &entry.error
        {
            lines.push(Line::from(""));
            lines.push(Line::styled(
                format!("PostgreSQL: {error}"),
                Style::default().fg(Color::Red),
            ));
        }
        lines
    });
    frame.render_widget(
        Paragraph::new(preview)
            .wrap(Wrap { trim: false })
            .block(Block::default().title(preview_title).borders(Borders::TOP)),
        sections[2],
    );
    frame.render_widget(
        Paragraph::new("Type to filter  •  Ctrl-n/p or ↑/↓ select  •  Enter open  •  Esc close")
            .style(Style::default().fg(Color::LightBlue)),
        sections[3],
    );
    frame.set_cursor_position((
        sections[0]
            .x
            .saturating_add(6 + finder.input.chars().count() as u16),
        sections[0].y,
    ));
}

fn draw_help(frame: &mut Frame) {
    let area = centered_rect(68, 78, frame.area());
    frame.render_widget(Clear, area);
    let help = vec![
        Line::styled(
            "GETTING STARTED",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Line::from("PGIDE has NORMAL and INSERT modes, like Neovim."),
        Line::from(""),
        key_line("i", "Enter INSERT mode and edit SQL"),
        key_line("Esc", "Return to NORMAL mode / close this window"),
        key_line("Ctrl-Enter / Space r", "Run statement under cursor"),
        key_line("Ctrl-s", "Save or update the current query"),
        key_line(
            "Ctrl-n / Space n",
            "New SQL buffer (from NORMAL editor mode)",
        ),
        key_line("Space b d", "Close current tab after confirmation"),
        key_line("Space f f", "Find saved queries for this database"),
        key_line("Space f h", "Search query history"),
        key_line("Space f s", "Save current SQL as a new query"),
        key_line("? / F1", "Open or close this cheat sheet"),
        key_line("q", "Quit from NORMAL mode"),
        key_line("Ctrl-c", "Quit from anywhere"),
        Line::from(""),
        Line::styled(
            "MOVEMENT",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        key_line("h j k l", "Move left, down, up, right in NORMAL mode"),
        key_line("o", "Open a new line below and enter INSERT mode"),
        key_line("O", "Open a new line above and enter INSERT mode"),
        key_line("dd", "Delete the current SQL line"),
        key_line("x", "Delete character under cursor"),
        key_line("a / A", "Append after cursor / at end of line"),
        key_line("I", "Insert at first non-blank character"),
        key_line("0 / ^ / $", "Line start / first non-blank / line end"),
        key_line("w / b / e", "Next word / previous word / word end"),
        key_line("gg / G", "First line / last line"),
        key_line("gt / gT", "Next / previous SQL buffer tab"),
        key_line("Arrow keys", "Move in either mode"),
        key_line("Tab / S-Tab", "Cycle Explorer → SQL → Results panels"),
        key_line("1 / 2 / 3", "Jump directly to Explorer / SQL / Results"),
        key_line("Ctrl-h", "Focus the Explorer on the left"),
        key_line("Ctrl-l", "Focus the SQL editor on the right"),
        key_line("Ctrl-j", "Focus Results below"),
        key_line("Ctrl-k", "Focus SQL above"),
        key_line(
            "Mouse click",
            "Focus any panel; click Explorer node to toggle",
        ),
        key_line("Enter", "Expand/collapse selected Explorer node"),
        key_line("h j k l", "Move the selected cell in Results"),
        key_line("Tab (INSERT)", "Accept highlighted SQL completion"),
        key_line("Ctrl-n / Ctrl-p", "Select next / previous completion"),
        key_line(
            "Enter (INSERT)",
            "Accept completion, otherwise insert newline",
        ),
        key_line("Enter (NORMAL)", "Expand/collapse Explorer nodes"),
        Line::from(""),
        Line::styled(
            "RUN YOUR FIRST QUERY",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Line::from("1. Press i and type or paste SQL"),
        Line::from("2. Press Esc"),
        Line::from("3. Press Ctrl-Enter (or Space, then r) to run the statement at cursor"),
        Line::from(""),
        Line::styled(
            "Press ? or Esc to close",
            Style::default().fg(Color::Yellow),
        ),
    ];
    frame.render_widget(
        Paragraph::new(help).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(" PGIDE CHEAT SHEET ")
                .title_style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .padding(Padding::uniform(1)),
        ),
        area,
    );
}

fn panel_block(title: &str, active: bool) -> Block<'static> {
    let label = if active {
        format!(" {title} [ACTIVE] ")
    } else {
        format!(" {title} ")
    };
    Block::default()
        .title(label)
        .borders(Borders::ALL)
        .border_style(if active {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        })
}

fn key_line<'a>(key: &'a str, description: &'a str) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!("{key:<12}"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(description),
    ])
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1])[1]
}

fn centered_fixed(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}
