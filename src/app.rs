use std::{collections::HashSet, time::Duration};

use crate::{
    action::Action,
    database::{DatabaseCatalog, DatabaseEvent, DatabaseService, QueryResult, TableDetails},
    sql::completion,
    storage::{HistoryEntry, SavedQuery, Storage},
};
use nucleo_matcher::{
    Config, Matcher,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectorSection {
    Overview,
    Columns,
    Constraints,
    Indexes,
}

impl InspectorSection {
    pub const fn next(self) -> Self {
        match self {
            Self::Overview => Self::Columns,
            Self::Columns => Self::Constraints,
            Self::Constraints => Self::Indexes,
            Self::Indexes => Self::Overview,
        }
    }

    pub const fn previous(self) -> Self {
        match self {
            Self::Overview => Self::Indexes,
            Self::Columns => Self::Overview,
            Self::Constraints => Self::Columns,
            Self::Indexes => Self::Constraints,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExplorerEntry {
    pub id: String,
    pub label: String,
    pub depth: usize,
    pub expandable: bool,
    pub open: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinderKind {
    SavedQueries,
    History,
}

#[derive(Debug, Clone)]
pub enum FinderItem {
    Saved(SavedQuery),
    History(HistoryEntry),
}

impl FinderItem {
    pub fn sql(&self) -> &str {
        match self {
            Self::Saved(query) => &query.sql,
            Self::History(entry) => &entry.sql,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Saved(query) => query.name.clone(),
            Self::History(entry) => {
                let status = if entry.success { "✓" } else { "✗" };
                let summary = entry.sql.lines().next().unwrap_or_default();
                let duration = entry
                    .duration_ms
                    .map_or_else(|| "—".into(), |milliseconds| format!("{milliseconds}ms"));
                format!(
                    "{status} {}  {duration:>7}  #{:<4} {summary}",
                    entry.executed_at, entry.id
                )
            }
        }
    }
}

pub struct FinderState {
    pub kind: FinderKind,
    pub input: String,
    pub selected: usize,
    pub items: Vec<FinderItem>,
}

pub struct SaveDialogState {
    pub input: String,
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
    pub inspector: Option<TableDetails>,
    pub inspector_loading: bool,
    pub inspector_section: InspectorSection,
    pub finder: Option<FinderState>,
    pub save_dialog: Option<SaveDialogState>,
    pub saved_query_id: Option<i64>,
    pub saved_query_name: Option<String>,
    pub status_message: Option<String>,
    running_statement: Option<String>,
    database: DatabaseService,
    storage: Storage,
}

impl App {
    #[cfg(test)]
    pub fn new(database: DatabaseService) -> Self {
        Self::with_storage(
            database,
            Storage::memory().expect("create in-memory storage"),
        )
    }

    pub fn with_storage(database: DatabaseService, storage: Storage) -> Self {
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
            inspector: None,
            inspector_loading: false,
            inspector_section: InspectorSection::Columns,
            finder: None,
            save_dialog: None,
            saved_query_id: None,
            saved_query_name: None,
            status_message: None,
            running_statement: None,
            database,
            storage,
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
                } else {
                    self.running_statement =
                        Some(crate::sql::statement::current(&self.query, self.cursor).to_owned());
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
                if let Some(entry) = self
                    .explorer_entries()
                    .get(self.explorer_selection)
                    .cloned()
                {
                    if entry.expandable {
                        if !self.explorer_open.remove(&entry.id) {
                            self.explorer_open.insert(entry.id);
                        }
                        self.explorer_expanded = self.explorer_open.contains("database");
                    } else if let Some((schema, table)) = table_from_entry_id(&entry.id) {
                        self.inspector_loading = true;
                        self.inspector = None;
                        self.inspector_section = InspectorSection::Columns;
                        self.focus = Focus::Results;
                        if self.database.inspect_table(schema, table).await.is_err() {
                            self.inspector_loading = false;
                            self.error = Some("database worker is unavailable".into());
                        }
                    }
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
            Action::PreviousInspectorSection => {
                self.inspector_section = self.inspector_section.previous();
            }
            Action::NextInspectorSection => {
                self.inspector_section = self.inspector_section.next();
            }
            Action::CloseInspector => {
                if self.inspector.is_some() || self.inspector_loading {
                    self.inspector = None;
                    self.inspector_loading = false;
                    self.focus = Focus::Explorer;
                }
            }
            Action::PreviewInspectedTable => {
                if let Some(details) = &self.inspector {
                    let schema = details.schema.replace('"', "\"\"");
                    let table = details.name.replace('"', "\"\"");
                    self.query = format!("SELECT *\nFROM \"{schema}\".\"{table}\"\nLIMIT 100;");
                    self.cursor = self.query.len();
                    self.inspector = None;
                    self.result = None;
                    self.error = None;
                    self.saved_query_id = None;
                    self.saved_query_name = None;
                    self.query_running = true;
                    self.focus = Focus::Results;
                    if self.database.execute(self.query.clone()).await.is_err() {
                        self.query_running = false;
                        self.error = Some("database worker is unavailable".into());
                    } else {
                        self.running_statement = Some(self.query.clone());
                    }
                }
            }
            Action::SaveQuery => self.save_query(),
            Action::OpenSavedQueryFinder => self.open_finder(FinderKind::SavedQueries),
            Action::OpenHistoryFinder => self.open_finder(FinderKind::History),
            Action::OverlayInsert(character) => self.overlay_insert(character),
            Action::OverlayBackspace => self.overlay_backspace(),
            Action::OverlayNext => self.overlay_move(1),
            Action::OverlayPrevious => self.overlay_move(-1),
            Action::OverlayAccept => self.overlay_accept(),
            Action::OverlayCancel => {
                self.finder = None;
                self.save_dialog = None;
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
                if let Some(entry) = self
                    .explorer_entries()
                    .get(self.explorer_selection)
                    .cloned()
                {
                    if entry.expandable {
                        if !self.explorer_open.remove(&entry.id) {
                            self.explorer_open.insert(entry.id);
                        }
                    } else if let Some((schema, table)) = table_from_entry_id(&entry.id) {
                        self.inspector_loading = true;
                        self.inspector = None;
                        self.inspector_section = InspectorSection::Columns;
                        self.focus = Focus::Results;
                        if self.database.inspect_table(schema, table).await.is_err() {
                            self.inspector_loading = false;
                            self.error = Some("database worker is unavailable".into());
                        }
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
                if self.focus == Focus::Results && self.inspector.is_some() {
                    self.inspector_section = self.inspector_section.previous();
                } else if self.focus == Focus::Results {
                    self.result_column = self.result_column.saturating_sub(1);
                } else {
                    self.move_left();
                }
            }
            Action::MoveRight => {
                if self.focus == Focus::Results && self.inspector.is_some() {
                    self.inspector_section = self.inspector_section.next();
                } else if self.focus == Focus::Results {
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
                self.saved_query_id = None;
                self.saved_query_name = None;
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
                if let Some(statement) = self.running_statement.take()
                    && let Some(database_name) = self.database_name.as_deref()
                {
                    let _ = self.storage.record_history(
                        database_name,
                        &statement,
                        true,
                        Some(result.elapsed.as_millis() as i64),
                        None,
                    );
                }
                self.result = Some(result);
                self.result_row = 0;
                self.result_column = 0;
                self.error = None;
            }
            DatabaseEvent::QueryFailed(message) => {
                self.query_running = false;
                if let Some(statement) = self.running_statement.take()
                    && let Some(database_name) = self.database_name.as_deref()
                {
                    let _ = self.storage.record_history(
                        database_name,
                        &statement,
                        false,
                        None,
                        Some(&message),
                    );
                }
                self.error = Some(message);
            }
            DatabaseEvent::TableInspected(details) => {
                self.inspector_loading = false;
                self.inspector = Some(details);
                self.error = None;
            }
            DatabaseEvent::TableInspectionFailed(message) => {
                self.inspector_loading = false;
                self.error = Some(message);
            }
        }
    }

    pub fn elapsed(&self) -> Option<Duration> {
        self.result.as_ref().map(|result| result.elapsed)
    }

    pub fn overlay_active(&self) -> bool {
        self.finder.is_some() || self.save_dialog.is_some()
    }

    pub fn finder_matches(&self) -> Vec<usize> {
        struct Candidate {
            index: usize,
            label: String,
        }

        impl AsRef<str> for Candidate {
            fn as_ref(&self) -> &str {
                &self.label
            }
        }

        let Some(finder) = &self.finder else {
            return Vec::new();
        };
        if finder.input.trim().is_empty() {
            return (0..finder.items.len()).collect();
        }
        let candidates = finder
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| Candidate {
                index,
                label: item.label(),
            })
            .collect::<Vec<_>>();
        let mut matcher = Matcher::new(Config::DEFAULT);
        Pattern::new(
            &finder.input,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        )
        .match_list(candidates, &mut matcher)
        .into_iter()
        .map(|(candidate, _)| candidate.index)
        .collect()
    }

    fn save_query(&mut self) {
        if self.database_name.is_none() {
            self.status_message = Some("Connect to a database before saving a query".into());
            return;
        }
        if self.saved_query_id.is_none() {
            self.save_dialog = Some(SaveDialogState {
                input: self.saved_query_name.clone().unwrap_or_default(),
            });
            return;
        }
        self.persist_saved_query();
    }

    fn persist_saved_query(&mut self) {
        let Some(database_name) = self.database_name.as_deref() else {
            self.status_message = Some("Connect to a database before saving a query".into());
            return;
        };
        let Some(name) = self.saved_query_name.as_deref() else {
            return;
        };
        match self
            .storage
            .save_query(self.saved_query_id, database_name, name, &self.query)
        {
            Ok(saved) => {
                self.saved_query_id = Some(saved.id);
                self.saved_query_name = Some(saved.name.clone());
                self.status_message = Some(format!("Saved '{}'", saved.name));
            }
            Err(error) => self.status_message = Some(format!("Save failed: {error}")),
        }
    }

    fn open_finder(&mut self, kind: FinderKind) {
        let Some(database_name) = self.database_name.as_deref() else {
            self.status_message = Some("Connect to a database first".into());
            return;
        };
        let loaded = match kind {
            FinderKind::SavedQueries => self
                .storage
                .saved_queries(database_name)
                .map(|items| items.into_iter().map(FinderItem::Saved).collect()),
            FinderKind::History => self
                .storage
                .history(database_name, 1_000)
                .map(|items| items.into_iter().map(FinderItem::History).collect()),
        };
        match loaded {
            Ok(items) => {
                self.finder = Some(FinderState {
                    kind,
                    input: String::new(),
                    selected: 0,
                    items,
                });
            }
            Err(error) => self.status_message = Some(format!("Could not open finder: {error}")),
        }
    }

    fn overlay_insert(&mut self, character: char) {
        if let Some(dialog) = &mut self.save_dialog {
            dialog.input.push(character);
        } else if let Some(finder) = &mut self.finder {
            finder.input.push(character);
            finder.selected = 0;
        }
    }

    fn overlay_backspace(&mut self) {
        if let Some(dialog) = &mut self.save_dialog {
            dialog.input.pop();
        } else if let Some(finder) = &mut self.finder {
            finder.input.pop();
            finder.selected = 0;
        }
    }

    fn overlay_move(&mut self, direction: i32) {
        let count = self.finder_matches().len();
        let Some(finder) = &mut self.finder else {
            return;
        };
        if count == 0 {
            finder.selected = 0;
        } else if direction > 0 {
            finder.selected = (finder.selected + 1) % count;
        } else {
            finder.selected = finder.selected.checked_sub(1).unwrap_or(count - 1);
        }
    }

    fn overlay_accept(&mut self) {
        if let Some(dialog) = self.save_dialog.take() {
            let name = dialog.input.trim().to_owned();
            if name.is_empty() {
                self.status_message = Some("Query name cannot be empty".into());
                self.save_dialog = Some(dialog);
                return;
            }
            self.saved_query_name = Some(name);
            self.persist_saved_query();
            return;
        }
        let matches = self.finder_matches();
        let Some(finder) = self.finder.take() else {
            return;
        };
        let Some(index) = matches.get(finder.selected).copied() else {
            return;
        };
        let item = &finder.items[index];
        self.query = item.sql().into();
        self.cursor = self.query.len();
        self.focus = Focus::Editor;
        self.mode = Mode::Normal;
        self.inspector = None;
        self.inspector_loading = false;
        self.error = None;
        match item {
            FinderItem::Saved(saved) => {
                self.saved_query_id = Some(saved.id);
                self.saved_query_name = Some(saved.name.clone());
                self.status_message = Some(format!("Opened '{}'", saved.name));
            }
            FinderItem::History(_) => {
                self.saved_query_id = None;
                self.saved_query_name = None;
                self.status_message = Some("Opened query from history".into());
            }
        }
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

fn table_from_entry_id(id: &str) -> Option<(String, String)> {
    let rest = id.strip_prefix("schema:")?;
    let mut parts = rest.splitn(3, ':');
    let schema = parts.next()?;
    let category = parts.next()?;
    let table = parts.next()?;
    (category == "Tables").then(|| (schema.into(), table.into()))
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

    #[tokio::test]
    async fn first_save_names_query_and_later_saves_update_it() {
        let mut app = app();
        app.connection = ConnectionState::Connected;
        app.database_name = Some("project_db".into());
        app.query = "SELECT 1;".into();
        app.cursor = app.query.len();

        app.update(Action::SaveQuery).await;
        assert!(app.save_dialog.is_some());
        for character in "Health check".chars() {
            app.update(Action::OverlayInsert(character)).await;
        }
        app.update(Action::OverlayAccept).await;

        let saved_id = app.saved_query_id.expect("saved query id");
        assert_eq!(app.saved_query_name.as_deref(), Some("Health check"));
        app.query = "SELECT 2;".into();
        app.update(Action::SaveQuery).await;

        let saved = app.storage.saved_queries("project_db").unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].id, saved_id);
        assert_eq!(saved[0].sql, "SELECT 2;");
    }

    #[tokio::test]
    async fn saved_query_finder_filters_and_opens_selected_sql() {
        let mut app = app();
        app.connection = ConnectionState::Connected;
        app.database_name = Some("project_db".into());
        app.storage
            .save_query(None, "project_db", "Daily revenue", "SELECT 42;")
            .unwrap();
        app.storage
            .save_query(None, "project_db", "Failed jobs", "SELECT 0;")
            .unwrap();

        app.update(Action::OpenSavedQueryFinder).await;
        for character in "revenue".chars() {
            app.update(Action::OverlayInsert(character)).await;
        }
        assert_eq!(app.finder_matches().len(), 1);
        app.update(Action::OverlayAccept).await;

        assert_eq!(app.query, "SELECT 42;");
        assert_eq!(app.saved_query_name.as_deref(), Some("Daily revenue"));
        assert!(app.finder.is_none());
    }

    #[tokio::test]
    async fn completed_queries_are_added_to_database_history() {
        let mut app = app();
        app.database_name = Some("project_db".into());
        app.query_running = true;
        app.running_statement = Some("SELECT now();".into());

        app.handle_database_event(DatabaseEvent::QueryFinished(QueryResult {
            columns: vec!["now".into()],
            rows: vec![vec!["2026-08-16".into()]],
            elapsed: Duration::from_millis(12),
        }));

        let history = app.storage.history("project_db", 10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].sql, "SELECT now();");
        assert_eq!(history[0].duration_ms, Some(12));
        assert!(history[0].success);
    }

    #[test]
    fn table_entries_resolve_to_qualified_names() {
        assert_eq!(
            table_from_entry_id("schema:public:Tables:collections"),
            Some(("public".into(), "collections".into()))
        );
        assert_eq!(
            table_from_entry_id("schema:public:Views:active_users"),
            None
        );
    }

    #[test]
    fn inspector_sections_cycle_in_both_directions() {
        assert_eq!(InspectorSection::Indexes.next(), InspectorSection::Overview);
        assert_eq!(
            InspectorSection::Overview.previous(),
            InspectorSection::Indexes
        );
    }
}
