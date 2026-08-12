use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Padding, Paragraph, Row, Table, Wrap},
};

use crate::{
    action::Action,
    app::{App, ConnectionState, Focus, Mode},
};

pub fn mouse_action(column: u16, row: u16, width: u16, height: u16) -> Action {
    let content_top: u16 = 3;
    let content_height = height.saturating_sub(4);
    let top_height = content_height.saturating_mul(58) / 100;
    if row < content_top.saturating_add(top_height) {
        if column < width.saturating_mul(30) / 100 {
            if row >= content_top.saturating_add(1) {
                Action::ClickExplorerNode(row.saturating_sub(content_top + 1) as usize)
            } else {
                Action::FocusExplorer
            }
        } else {
            Action::FocusEditor
        }
    } else {
        Action::FocusResults
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
        Paragraph::new(crate::sql::highlight::lines(&app.query)).block(panel_block(
            &format!(
                "SQL  {}  Ln {cursor_line}, Col {cursor_column}",
                app.mode.label()
            ),
            app.focus == Focus::Editor,
        )),
        area,
    );
    if app.focus == Focus::Editor && !app.help_visible {
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
        let x = area
            .x
            .saturating_add(1)
            .saturating_add(column)
            .min(area.right().saturating_sub(2));
        let y = area
            .y
            .saturating_add(1)
            .saturating_add(row)
            .min(area.bottom().saturating_sub(2));
        frame.set_cursor_position((x, y));
    }
}

fn draw_results(frame: &mut Frame, area: Rect, app: &App) {
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
        "LEADER  r Run  ? Help"
    } else if app.key_sequence == Some('d') {
        "d…  d Delete current line  •  Esc Cancel"
    } else if app.key_sequence == Some('g') {
        "g…  g First line  •  Esc Cancel"
    } else if app.mode == Mode::Insert {
        "Esc Normal  •  type to edit"
    } else {
        "1 Explorer  2 SQL  3 Results  •  Ctrl-Enter/Space r Run  •  i/o/dd Edit  •  ? Help"
    };
    let left = format!(" {} │ {}{}", app.mode.label(), connection, details);
    let gap = (area.width as usize).saturating_sub(left.chars().count() + hint.chars().count() + 1);
    frame.render_widget(
        Paragraph::new(format!("{left}{}{hint} ", " ".repeat(gap)))
            .style(Style::default().fg(Color::Black).bg(Color::Cyan)),
        area,
    );
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
