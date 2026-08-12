use std::{collections::HashSet, time::Duration};

use crate::{
    action::Action,
    database::{DatabaseCatalog, DatabaseEvent, DatabaseService, QueryResult},
    sql::completion,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
}

impl Mode {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Insert => "INSERT",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Explorer,
    Editor,
    Results,
}

#[derive(Debug, Clone)]
pub struct ExplorerEntry {
    pub id: String,
    pub label: String,
    pub depth: usize,
    pub expandable: bool,
    pub open: bool,
}

pub struct App {
    pub mode: Mode,
    pub connection: ConnectionState,
    pub database_name: Option<String>,
    pub query: String,
    pub cursor: usize,
    pub result: Option<QueryResult>,
    pub error: Option<String>,
    pub query_running: bool,
    pub should_quit: bool,
    pub key_sequence: Option<char>,
    pub help_visible: bool,
    pub focus: Focus,
    pub explorer_expanded: bool,
    pub catalog: DatabaseCatalog,
    pub explorer_open: HashSet<String>,
    pub explorer_selection: usize,
    pub result_row: usize,
    pub result_column: usize,
    pub completion_items: Vec<String>,
    pub relation_items: Vec<String>,
    pub completion_index: usize,
    database: DatabaseService,
}

impl App {
    pub fn new(database: DatabaseService) -> Self {
        Self {
            mode: Mode::Normal,
            connection: ConnectionState::Disconnected,
            database_name: None,
            query: "SELECT 1;".into(),
            cursor: 9,
            result: None,
            error: None,
            query_running: false,
            should_quit: false,
            key_sequence: None,
            help_visible: false,
            focus: Focus::Editor,
            explorer_expanded: true,
            catalog: DatabaseCatalog::default(),
            explorer_open: HashSet::from(["database".into()]),
            explorer_selection: 0,
            result_row: 0,
            result_column: 0,
            completion_items: Vec::new(),
            relation_items: Vec::new(),
            completion_index: 0,
            database,
        }
    }

    pub async fn connect(&mut self, url: String) {
        self.connection = ConnectionState::Connecting;
        self.error = None;
        if self.database.connect(url).await.is_err() {
            self.connection = ConnectionState::Disconnected;
            self.error = Some("database worker is unavailable".into());
        }
    }

    pub async fn update(&mut self, action: Action) {
        match action {
            Action::Quit => self.should_quit = true,
            Action::EnterInsertMode => {
                self.mode = Mode::Insert;
                self.focus = Focus::Editor;
            }
            Action::EnterNormalMode => self.mode = Mode::Normal,
            Action::RunQuery
                if self.connection == ConnectionState::Connected && !self.query_running =>
            {
                self.error = None;
                self.result = None;
                self.result_row = 0;
                self.result_column = 0;
                self.query_running = true;
                let statement = crate::sql::statement::current(&self.query, self.cursor).to_owned();
                if statement.is_empty() {
                    self.query_running = false;
                    self.error = Some("no SQL statement under the cursor".into());
                } else if self.database.execute(statement).await.is_err() {
                    self.query_running = false;
                    self.error = Some("database worker is unavailable".into());
                }
                self.focus = Focus::Results;
            }
            Action::RunQuery => {
                self.error = Some(if self.query_running {
                    "a query is already running".into()
                } else {
                    "connect to PostgreSQL before running a query".into()
                });
            }
            Action::Activate if self.focus == Focus::Explorer => {
                if let Some(entry) = self.explorer_entries().get(self.explorer_selection)
                    && entry.expandable
                {
                    let id = entry.id.clone();
                    if !self.explorer_open.remove(&id) {
                        self.explorer_open.insert(id);
                    }
                    self.explorer_expanded = self.explorer_open.contains("database");
                }
            }
            Action::Activate => {}
            Action::OpenLineBelow => {
                self.focus = Focus::Editor;
                let line_end = self.query[self.cursor..]
                    .find('\n')
                    .map_or(self.query.len(), |offset| self.cursor + offset);
                self.query.insert(line_end, '\n');
                self.cursor = line_end + 1;
                self.mode = Mode::Insert;
                self.completion_index = 0;
            }
            Action::OpenLineAbove => {
                self.focus = Focus::Editor;
                let line_start = self.query[..self.cursor]
                    .rfind('\n')
                    .map_or(0, |position| position + 1);
                self.query.insert(line_start, '\n');
                self.cursor = line_start;
                self.mode = Mode::Insert;
                self.completion_index = 0;
            }
            Action::DeleteCurrentLine => self.delete_current_line(),
            Action::MoveLineStart => self.cursor = self.line_start(),
            Action::MoveFirstNonBlank => {
                let start = self.line_start();
                let end = self.line_end();
                self.cursor = self.query[start..end]
                    .char_indices()
                    .find(|(_, character)| !character.is_whitespace())
                    .map_or(end, |(offset, _)| start + offset);
            }
            Action::MoveLineEnd => self.cursor = self.line_end(),
            Action::MoveWordForward => self.move_word_forward(),
            Action::MoveWordBackward => self.move_word_backward(),
            Action::MoveWordEnd => self.move_word_end(),
            Action::GoToFirstLine => self.cursor = 0,
            Action::GoToLastLine => {
                self.cursor = self.query.rfind('\n').map_or(0, |position| position + 1);
            }
            Action::AppendAfterCursor => {
                self.move_right();
                self.mode = Mode::Insert;
                self.focus = Focus::Editor;
            }
            Action::AppendLineEnd => {
                self.cursor = self.line_end();
                self.mode = Mode::Insert;
                self.focus = Focus::Editor;
            }
            Action::InsertLineStart => {
                let start = self.line_start();
                let end = self.line_end();
                self.cursor = self.query[start..end]
                    .char_indices()
                    .find(|(_, character)| !character.is_whitespace())
                    .map_or(end, |(offset, _)| start + offset);
                self.mode = Mode::Insert;
                self.focus = Focus::Editor;
            }
            Action::DeleteCharacter => {
                if let Some(character) = self.query[self.cursor..].chars().next()
                    && character != '\n'
                {
                    self.query
                        .drain(self.cursor..self.cursor + character.len_utf8());
                }
            }
            Action::ToggleHelp => self.help_visible = !self.help_visible,
            Action::FocusNext => {
                self.focus = match self.focus {
                    Focus::Explorer => Focus::Editor,
                    Focus::Editor => Focus::Results,
                    Focus::Results => Focus::Explorer,
                }
            }
            Action::FocusPrevious => {
                self.focus = match self.focus {
                    Focus::Explorer => Focus::Results,
                    Focus::Editor => Focus::Explorer,
                    Focus::Results => Focus::Editor,
                }
            }
            Action::FocusLeft => self.focus = Focus::Explorer,
            Action::FocusRight | Action::FocusUp => self.focus = Focus::Editor,
            Action::FocusDown => self.focus = Focus::Results,
            Action::FocusExplorer => self.focus = Focus::Explorer,
            Action::FocusEditor => self.focus = Focus::Editor,
            Action::FocusResults => self.focus = Focus::Results,
            Action::ClickExplorerNode(index) => {
                self.focus = Focus::Explorer;
                self.explorer_selection =
                    index.min(self.explorer_entries().len().saturating_sub(1));
                if let Some(entry) = self.explorer_entries().get(self.explorer_selection) {
                    let id = entry.id.clone();
                    if entry.expandable && !self.explorer_open.remove(&id) {
                        self.explorer_open.insert(id);
                    }
                }
            }
            Action::AcceptCompletion => {
                let candidates = completion::candidates(
                    &self.query,
                    self.cursor,
                    &self.completion_items,
                    &self.relation_items,
                );
                let selected = self
                    .completion_index
                    .min(candidates.len().saturating_sub(1));
                if let Some(candidate) = candidates.get(selected) {
                    let prefix_len = completion::prefix(&self.query, self.cursor).len();
                    let start = self.cursor - prefix_len;
                    self.query.replace_range(start..self.cursor, candidate);
                    self.cursor = start + candidate.len();
                    self.completion_index = 0;
                } else {
                    self.query.insert(self.cursor, '\n');
                    self.cursor += 1;
                }
            }
            Action::NextCompletion => {
                let count = completion::candidates(
                    &self.query,
                    self.cursor,
                    &self.completion_items,
                    &self.relation_items,
                )
                .len();
                if count > 0 {
                    self.completion_index = (self.completion_index + 1) % count;
                }
            }
            Action::PreviousCompletion => {
                let count = completion::candidates(
                    &self.query,
                    self.cursor,
                    &self.completion_items,
                    &self.relation_items,
                )
                .len();
                if count > 0 {
                    self.completion_index =
                        self.completion_index.checked_sub(1).unwrap_or(count - 1);
                }
            }
            Action::Insert(character) => {
                self.query.insert(self.cursor, character);
                self.cursor += character.len_utf8();
                self.completion_index = 0;
            }
            Action::Paste(text) => {
                self.query.insert_str(self.cursor, &text);
                self.cursor += text.len();
                self.completion_index = 0;
            }
            Action::Backspace => self.backspace(),
            Action::MoveLeft => {
                if self.focus == Focus::Results {
                    self.result_column = self.result_column.saturating_sub(1);
                } else {
                    self.move_left();
                }
            }
            Action::MoveRight => {
                if self.focus == Focus::Results {
                    let columns = self
                        .result
                        .as_ref()
                        .map_or(0, |result| result.columns.len());
                    self.result_column = self
                        .result_column
                        .saturating_add(1)
                        .min(columns.saturating_sub(1));
                } else {
                    self.move_right();
                }
            }
            Action::MoveUp => {
                if self.focus == Focus::Explorer {
                    self.explorer_selection = self.explorer_selection.saturating_sub(1);
                } else if self.focus == Focus::Results {
                    self.result_row = self.result_row.saturating_sub(1);
                } else {
                    self.move_vertical(-1);
                }
            }
            Action::MoveDown => {
                if self.focus == Focus::Explorer {
                    let entries = self.explorer_entries().len();
                    self.explorer_selection = self
                        .explorer_selection
                        .saturating_add(1)
                        .min(entries.saturating_sub(1));
                } else if self.focus == Focus::Results {
                    let rows = self.result.as_ref().map_or(0, |result| result.rows.len());
                    self.result_row = self
                        .result_row
                        .saturating_add(1)
                        .min(rows.saturating_sub(1));
                } else {
                    self.move_vertical(1);
                }
            }
        }
    }

    pub fn handle_database_event(&mut self, event: DatabaseEvent) {
        match event {
            DatabaseEvent::Connected {
                database_name,
                completion_items,
                relation_items,
                catalog,
            } => {
                self.connection = ConnectionState::Connected;
                self.database_name = Some(database_name);
                self.completion_items = completion_items;
                self.relation_items = relation_items;
                self.catalog = catalog;
                if let Some(schema) = self.catalog.schemas.first() {
                    self.explorer_open.insert(format!("schema:{}", schema.name));
                }
                self.error = None;
            }
            DatabaseEvent::ConnectionFailed(message) => {
                self.connection = ConnectionState::Disconnected;
                self.database_name = None;
                self.error = Some(message);
            }
            DatabaseEvent::QueryFinished(result) => {
                self.query_running = false;
                self.result = Some(result);
                self.result_row = 0;
                self.result_column = 0;
                self.error = None;
            }
            DatabaseEvent::QueryFailed(message) => {
                self.query_running = false;
                self.error = Some(message);
            }
        }
    }

    pub fn elapsed(&self) -> Option<Duration> {
        self.result.as_ref().map(|result| result.elapsed)
    }

    pub fn explorer_entries(&self) -> Vec<ExplorerEntry> {
        let mut entries = Vec::new();
        let database_open = self.explorer_open.contains("database");
        entries.push(ExplorerEntry {
            id: "database".into(),
            label: self
                .database_name
                .clone()
                .unwrap_or_else(|| "database".into()),
            depth: 0,
            expandable: true,
            open: database_open,
        });
        if !database_open {
            return entries;
        }
        for schema in &self.catalog.schemas {
            let schema_id = format!("schema:{}", schema.name);
            let schema_open = self.explorer_open.contains(&schema_id);
            entries.push(ExplorerEntry {
                id: schema_id.clone(),
                label: schema.name.clone(),
                depth: 1,
                expandable: true,
                open: schema_open,
            });
            if !schema_open {
                continue;
            }
            add_category(
                &mut entries,
                &self.explorer_open,
                &schema_id,
                "Tables",
                &schema.tables,
            );
            add_category(
                &mut entries,
                &self.explorer_open,
                &schema_id,
                "Views",
                &schema.views,
            );
            add_category(
                &mut entries,
                &self.explorer_open,
                &schema_id,
                "Functions",
                &schema.functions,
            );
        }
        entries
    }

    fn backspace(&mut self) {
        if let Some((index, _)) = self.query[..self.cursor].char_indices().next_back() {
            self.query.drain(index..self.cursor);
            self.cursor = index;
            self.completion_index = 0;
        }
    }

    fn delete_current_line(&mut self) {
        let line_start = self.query[..self.cursor]
            .rfind('\n')
            .map_or(0, |position| position + 1);
        let line_end = self.query[self.cursor..]
            .find('\n')
            .map(|offset| self.cursor + offset + 1)
            .unwrap_or(self.query.len());
        if line_start == 0 && line_end == self.query.len() {
            self.query.clear();
            self.cursor = 0;
        } else if line_end == self.query.len() {
            let delete_start = line_start.saturating_sub(1);
            self.query.drain(delete_start..line_end);
            self.cursor = delete_start;
        } else {
            self.query.drain(line_start..line_end);
            self.cursor = line_start.min(self.query.len());
        }
        self.completion_index = 0;
    }

    fn line_start(&self) -> usize {
        self.query[..self.cursor]
            .rfind('\n')
            .map_or(0, |position| position + 1)
    }

    fn line_end(&self) -> usize {
        self.query[self.cursor..]
            .find('\n')
            .map_or(self.query.len(), |offset| self.cursor + offset)
    }

    fn move_word_forward(&mut self) {
        let mut seen_word = false;
        for (offset, character) in self.query[self.cursor..].char_indices() {
            if is_word(character) {
                if seen_word {
                    self.cursor += offset;
                    return;
                }
            } else if offset > 0 {
                seen_word = true;
            }
        }
        self.cursor = self.query.len();
    }

    fn move_word_backward(&mut self) {
        let chars = self.query[..self.cursor].char_indices().collect::<Vec<_>>();
        let mut found_word = false;
        for &(index, character) in chars.iter().rev() {
            if is_word(character) {
                found_word = true;
                self.cursor = index;
            } else if found_word {
                return;
            }
        }
    }

    fn move_word_end(&mut self) {
        let mut in_word = false;
        let mut last_word = self.cursor;
        for (offset, character) in self.query[self.cursor..].char_indices() {
            if is_word(character) {
                in_word = true;
                last_word = self.cursor + offset + character.len_utf8();
            } else if in_word {
                self.cursor = last_word;
                return;
            }
        }
        self.cursor = last_word;
    }

    fn move_left(&mut self) {
        if let Some((index, _)) = self.query[..self.cursor].char_indices().next_back() {
            self.cursor = index;
        }
    }

    fn move_right(&mut self) {
        if let Some(character) = self.query[self.cursor..].chars().next() {
            self.cursor += character.len_utf8();
        }
    }

    fn move_vertical(&mut self, direction: i32) {
        let before = &self.query[..self.cursor];
        let line_start = before.rfind('\n').map_or(0, |position| position + 1);
        let column = before[line_start..].chars().count();

        if direction < 0 {
            if line_start == 0 {
                return;
            }
            let previous_end = line_start - 1;
            let previous_start = self.query[..previous_end].rfind('\n').map_or(0, |p| p + 1);
            self.cursor = byte_at_column(
                &self.query[previous_start..previous_end],
                previous_start,
                column,
            );
        } else {
            let Some(next_start) = self.query[self.cursor..]
                .find('\n')
                .map(|p| self.cursor + p + 1)
            else {
                return;
            };
            let next_end = self.query[next_start..]
                .find('\n')
                .map_or(self.query.len(), |p| next_start + p);
            self.cursor = byte_at_column(&self.query[next_start..next_end], next_start, column);
        }
    }
}

fn add_category(
    entries: &mut Vec<ExplorerEntry>,
    open: &HashSet<String>,
    schema_id: &str,
    label: &str,
    objects: &[String],
) {
    let id = format!("{schema_id}:{label}");
    let is_open = open.contains(&id);
    entries.push(ExplorerEntry {
        id: id.clone(),
        label: format!("{label} ({})", objects.len()),
        depth: 2,
        expandable: true,
        open: is_open,
    });
    if is_open {
        entries.extend(objects.iter().map(|object| ExplorerEntry {
            id: format!("{id}:{object}"),
            label: object.clone(),
            depth: 3,
            expandable: false,
            open: false,
        }));
    }
}

fn byte_at_column(line: &str, offset: usize, column: usize) -> usize {
    offset
        + line
            .char_indices()
            .nth(column)
            .map_or(line.len(), |(index, _)| index)
}

fn is_word(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let (database, _events) = DatabaseService::spawn();
        App::new(database)
    }

    #[tokio::test]
    async fn editing_preserves_utf8_cursor_boundaries() {
        let mut app = app();
        app.query.clear();
        app.cursor = 0;

        app.update(Action::Insert('λ')).await;
        app.update(Action::Insert('x')).await;
        app.update(Action::MoveLeft).await;
        app.update(Action::Backspace).await;

        assert_eq!(app.query, "x");
        assert_eq!(app.cursor, 0);
    }

    #[tokio::test]
    async fn paste_is_inserted_atomically() {
        let mut app = app();
        app.query.clear();
        app.cursor = 0;

        app.update(Action::Paste("SELECT\n  1;".into())).await;

        assert_eq!(app.query, "SELECT\n  1;");
        assert_eq!(app.cursor, app.query.len());
    }

    #[tokio::test]
    async fn vim_open_and_delete_line_work() {
        let mut app = app();
        app.query = "SELECT 1;\nSELECT 2;".into();
        app.cursor = 3;

        app.update(Action::OpenLineBelow).await;
        assert_eq!(app.query, "SELECT 1;\n\nSELECT 2;");
        assert_eq!(app.mode, Mode::Insert);

        app.mode = Mode::Normal;
        app.update(Action::DeleteCurrentLine).await;
        assert_eq!(app.query, "SELECT 1;\nSELECT 2;");
    }

    #[tokio::test]
    async fn vim_line_end_and_append_enter_insert_mode() {
        let mut app = app();
        app.query = "SELECT 1;\nSELECT 2;".into();
        app.cursor = 2;

        app.update(Action::AppendLineEnd).await;

        assert_eq!(app.cursor, 9);
        assert_eq!(app.mode, Mode::Insert);
    }
}
