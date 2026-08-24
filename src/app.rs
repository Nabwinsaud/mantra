use std::{
    collections::HashSet,
    io::Write,
    process::{Command, Stdio},
    time::Duration,
};

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
    Tables,
}

#[derive(Debug, Clone)]
pub enum FinderItem {
    Saved(SavedQuery),
    History(HistoryEntry),
    Table { schema: String, table: String },
}

impl FinderItem {
    pub fn sql(&self) -> &str {
        match self {
            Self::Saved(query) => &query.sql,
            Self::History(entry) => &entry.sql,
            Self::Table { .. } => "",
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
            Self::Table { schema, table } => format!("{schema}.{table}"),
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
    pub save_as: bool,
}

pub struct CloseTabDialogState {
    pub tab_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationKind {
    OpenDeleteSql,
    ExecuteDestructiveSql,
}

pub struct ConfirmationDialogState {
    pub title: String,
    pub message: String,
    pub sql: String,
    pub kind: ConfirmationKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditSnapshot {
    query: String,
    cursor: usize,
}

#[derive(Debug, Clone)]
pub struct QueryTab {
    pub query: String,
    pub cursor: usize,
    pub saved_query_id: Option<i64>,
    pub name: Option<String>,
    pub saved_snapshot: Option<String>,
    undo_stack: Vec<EditSnapshot>,
    redo_stack: Vec<EditSnapshot>,
}

impl QueryTab {
    pub fn is_modified(&self) -> bool {
        self.saved_snapshot
            .as_deref()
            .map_or_else(|| !self.query.is_empty(), |saved| saved != self.query)
    }
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
    pub explorer_visible: bool,
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
    pub close_tab_dialog: Option<CloseTabDialogState>,
    pub confirmation_dialog: Option<ConfirmationDialogState>,
    pub saved_query_id: Option<i64>,
    pub saved_query_name: Option<String>,
    pub saved_query_snapshot: Option<String>,
    pub query_tabs: Vec<QueryTab>,
    pub active_query_tab: usize,
    pub status_message: Option<String>,
    undo_stack: Vec<EditSnapshot>,
    redo_stack: Vec<EditSnapshot>,
    running_statement: Option<String>,
    database: DatabaseService,
    storage: Storage,
    restored_database: Option<String>,
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
        let query = "SELECT 1;".to_owned();
        Self {
            mode: Mode::Normal,
            connection: ConnectionState::Disconnected,
            database_name: None,
            query: query.clone(),
            cursor: 9,
            result: None,
            error: None,
            query_running: false,
            should_quit: false,
            key_sequence: None,
            help_visible: false,
            focus: Focus::Editor,
            explorer_expanded: true,
            explorer_visible: true,
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
            close_tab_dialog: None,
            confirmation_dialog: None,
            saved_query_id: None,
            saved_query_name: None,
            saved_query_snapshot: None,
            query_tabs: vec![QueryTab {
                query,
                cursor: 9,
                saved_query_id: None,
                name: None,
                saved_snapshot: None,
                undo_stack: Vec::new(),
                redo_stack: Vec::new(),
            }],
            active_query_tab: 0,
            status_message: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            running_statement: None,
            database,
            storage,
            restored_database: None,
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
            Action::Noop => {}
            Action::Quit => self.should_quit = true,
            Action::EnterInsertMode => {
                self.mode = Mode::Insert;
                self.focus = Focus::Editor;
            }
            Action::EnterNormalMode => self.mode = Mode::Normal,
            Action::RunQuery => {
                let statement = crate::sql::statement::current(&self.query, self.cursor).to_owned();
                if self.connection != ConnectionState::Connected {
                    self.error = Some("connect to PostgreSQL before running a query".into());
                } else if self.query_running {
                    self.error = Some("a query is already running".into());
                } else if statement.is_empty() {
                    self.error = Some("no SQL statement under the cursor".into());
                } else if is_destructive_sql(&statement) {
                    self.confirmation_dialog = Some(ConfirmationDialogState {
                        title: "Confirm destructive SQL".into(),
                        message:
                            "This statement can permanently delete data. Review it before running."
                                .into(),
                        sql: statement,
                        kind: ConfirmationKind::ExecuteDestructiveSql,
                    });
                } else {
                    self.execute_statement(statement).await;
                }
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
                self.record_edit();
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
                self.record_edit();
                self.focus = Focus::Editor;
                let line_start = self.query[..self.cursor]
                    .rfind('\n')
                    .map_or(0, |position| position + 1);
                self.query.insert(line_start, '\n');
                self.cursor = line_start;
                self.mode = Mode::Insert;
                self.completion_index = 0;
            }
            Action::DeleteCurrentLine => {
                self.record_edit();
                self.delete_current_line();
            }
            Action::Undo => self.undo(),
            Action::Redo => self.redo(),
            Action::YankResultCell => self.yank_result_cell(),
            Action::YankResultRow => self.yank_result_row(),
            Action::YankTableAiPrompt => self.yank_table_ai_prompt(),
            Action::EditResultCell => match self.update_sql_for_selected_cell() {
                Ok(sql) => {
                    self.open_scratch_tab(sql);
                    self.status_message = Some(
                        "Opened UPDATE in a new SQL tab; edit the SET value before running".into(),
                    );
                }
                Err(message) => self.status_message = Some(message),
            },
            Action::RequestDeleteResultRow => match self.delete_sql_for_selected_row() {
                Ok(sql) => {
                    self.confirmation_dialog = Some(ConfirmationDialogState {
                        title: "Prepare DELETE statement?".into(),
                        message: "Mantra found the table primary key. Enter opens this SQL in a new tab; it does not execute it."
                            .into(),
                        sql,
                        kind: ConfirmationKind::OpenDeleteSql,
                    });
                }
                Err(message) => self.status_message = Some(message),
            },
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
                    self.record_edit();
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
                    let query = format!("SELECT *\nFROM \"{schema}\".\"{table}\"\nLIMIT 100;");
                    self.open_scratch_tab(query);
                    self.inspector = None;
                    self.result = None;
                    self.error = None;
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
            Action::SaveQueryAs => self.save_query_as(),
            Action::OpenSavedQueryFinder => self.open_finder(FinderKind::SavedQueries),
            Action::OpenHistoryFinder => self.open_finder(FinderKind::History),
            Action::OpenTableFinder => self.open_finder(FinderKind::Tables),
            Action::OverlayInsert(character) => self.overlay_insert(character),
            Action::OverlayBackspace => self.overlay_backspace(),
            Action::OverlayNext => self.overlay_move(1),
            Action::OverlayPrevious => self.overlay_move(-1),
            Action::OverlayAccept => {
                if let Some(dialog) = self.confirmation_dialog.take() {
                    match dialog.kind {
                        ConfirmationKind::OpenDeleteSql => {
                            self.open_scratch_tab(dialog.sql);
                            self.status_message = Some(
                                "Opened DELETE in a new SQL tab; review before running".into(),
                            );
                        }
                        ConfirmationKind::ExecuteDestructiveSql => {
                            self.execute_statement(dialog.sql).await;
                        }
                    }
                } else {
                    self.overlay_accept().await;
                }
            }
            Action::OverlayCancel => {
                self.finder = None;
                self.save_dialog = None;
                self.close_tab_dialog = None;
                self.confirmation_dialog = None;
            }
            Action::ToggleHelp => self.help_visible = !self.help_visible,
            Action::FocusNext => {
                self.focus = if self.explorer_visible {
                    match self.focus {
                        Focus::Explorer => Focus::Editor,
                        Focus::Editor => Focus::Results,
                        Focus::Results => Focus::Explorer,
                    }
                } else if self.focus == Focus::Editor {
                    Focus::Results
                } else {
                    Focus::Editor
                };
            }
            Action::FocusPrevious => {
                self.focus = if self.explorer_visible {
                    match self.focus {
                        Focus::Explorer => Focus::Results,
                        Focus::Editor => Focus::Explorer,
                        Focus::Results => Focus::Editor,
                    }
                } else if self.focus == Focus::Editor {
                    Focus::Results
                } else {
                    Focus::Editor
                };
            }
            Action::FocusLeft => {
                self.explorer_visible = true;
                self.focus = Focus::Explorer;
            }
            Action::FocusRight | Action::FocusUp => self.focus = Focus::Editor,
            Action::FocusDown => self.focus = Focus::Results,
            Action::FocusExplorer => {
                self.explorer_visible = true;
                self.focus = Focus::Explorer;
            }
            Action::FocusEditor => self.focus = Focus::Editor,
            Action::FocusResults => self.focus = Focus::Results,
            Action::ToggleExplorer => {
                self.explorer_visible = !self.explorer_visible;
                if !self.explorer_visible && self.focus == Focus::Explorer {
                    self.focus = Focus::Editor;
                }
                self.status_message = Some(
                    if self.explorer_visible {
                        "Explorer shown"
                    } else {
                        "Explorer hidden"
                    }
                    .into(),
                );
            }
            Action::FocusQueryTab(index) => self.activate_query_tab(index),
            Action::NewQueryTab => {
                self.open_scratch_tab(String::new());
                self.status_message = Some("Opened a new SQL buffer".into());
            }
            Action::RequestCloseQueryTab => {
                self.close_tab_dialog = Some(CloseTabDialogState {
                    tab_index: self.active_query_tab,
                });
            }
            Action::ConfirmCloseQueryTab => self.confirm_close_query_tab(),
            Action::NextQueryTab => {
                let next = (self.active_query_tab + 1) % self.query_tabs.len();
                self.activate_query_tab(next);
            }
            Action::PreviousQueryTab => {
                let previous = self
                    .active_query_tab
                    .checked_sub(1)
                    .unwrap_or(self.query_tabs.len() - 1);
                self.activate_query_tab(previous);
            }
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
                    self.record_edit();
                    let prefix_len = completion::prefix(&self.query, self.cursor).len();
                    let start = self.cursor - prefix_len;
                    self.query.replace_range(start..self.cursor, candidate);
                    self.cursor = start + candidate.len();
                    self.completion_index = 0;
                } else {
                    self.record_edit();
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
                self.record_edit();
                self.query.insert(self.cursor, character);
                self.cursor += character.len_utf8();
                self.completion_index = 0;
            }
            Action::Paste(text) => {
                if !text.is_empty() {
                    self.record_edit();
                }
                self.query.insert_str(self.cursor, &text);
                self.cursor += text.len();
                self.completion_index = 0;
            }
            Action::Backspace => {
                if self.cursor > 0 {
                    self.record_edit();
                }
                self.backspace();
            }
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
                self.database_name = Some(database_name.clone());
                self.completion_items = completion_items;
                self.relation_items = relation_items;
                self.catalog = catalog;
                if let Some(schema) = self.catalog.schemas.first() {
                    self.explorer_open.insert(format!("schema:{}", schema.name));
                }
                self.error = None;
                self.restore_query_tab_session(&database_name);
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

    pub fn active_query_is_modified(&self) -> bool {
        self.saved_query_snapshot
            .as_deref()
            .map_or_else(|| !self.query.is_empty(), |saved| saved != self.query)
    }

    fn sync_active_query_tab(&mut self) {
        if let Some(tab) = self.query_tabs.get_mut(self.active_query_tab) {
            tab.query.clone_from(&self.query);
            tab.cursor = self.cursor;
            tab.saved_query_id = self.saved_query_id;
            tab.name.clone_from(&self.saved_query_name);
            tab.saved_snapshot.clone_from(&self.saved_query_snapshot);
            tab.undo_stack.clone_from(&self.undo_stack);
            tab.redo_stack.clone_from(&self.redo_stack);
        }
    }

    fn activate_query_tab(&mut self, index: usize) {
        if index >= self.query_tabs.len() {
            return;
        }
        self.sync_active_query_tab();
        self.load_query_tab(index);
        self.persist_query_tab_session();
    }

    fn load_query_tab(&mut self, index: usize) {
        self.active_query_tab = index;
        let tab = self.query_tabs[index].clone();
        self.query = tab.query;
        self.cursor = tab.cursor.min(self.query.len());
        self.saved_query_id = tab.saved_query_id;
        self.saved_query_name = tab.name;
        self.saved_query_snapshot = tab.saved_snapshot;
        self.undo_stack = tab.undo_stack;
        self.redo_stack = tab.redo_stack;
        self.mode = Mode::Normal;
        self.focus = Focus::Editor;
        self.completion_index = 0;
    }

    fn confirm_close_query_tab(&mut self) {
        let Some(dialog) = self.close_tab_dialog.take() else {
            return;
        };
        if dialog.tab_index >= self.query_tabs.len() {
            return;
        }
        self.sync_active_query_tab();
        let closed = self.query_tabs.remove(dialog.tab_index);
        if self.query_tabs.is_empty() {
            self.query_tabs.push(QueryTab {
                query: String::new(),
                cursor: 0,
                saved_query_id: None,
                name: None,
                saved_snapshot: None,
                undo_stack: Vec::new(),
                redo_stack: Vec::new(),
            });
        }
        let next = dialog.tab_index.min(self.query_tabs.len() - 1);
        self.load_query_tab(next);
        self.persist_query_tab_session();
        self.status_message = Some(match closed.name {
            Some(name) => format!("Closed '{name}'; the saved query remains on disk"),
            None => "Closed scratch tab; its buffer was discarded".into(),
        });
    }

    fn open_saved_query_tab(&mut self, saved: SavedQuery) {
        if let Some(index) = self
            .query_tabs
            .iter()
            .position(|tab| tab.saved_query_id == Some(saved.id))
        {
            self.activate_query_tab(index);
            return;
        }
        self.sync_active_query_tab();
        let cursor = saved.sql.len();
        self.query_tabs.push(QueryTab {
            query: saved.sql.clone(),
            cursor,
            saved_query_id: Some(saved.id),
            name: Some(saved.name.clone()),
            saved_snapshot: Some(saved.sql.clone()),
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        });
        self.active_query_tab = self.query_tabs.len() - 1;
        self.query = saved.sql.clone();
        self.cursor = cursor;
        self.saved_query_id = Some(saved.id);
        self.saved_query_name = Some(saved.name.clone());
        self.saved_query_snapshot = Some(saved.sql);
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.mode = Mode::Normal;
        self.focus = Focus::Editor;
        self.completion_index = 0;
        self.status_message = Some(format!("Opened '{}'", saved.name));
        self.persist_query_tab_session();
    }

    fn open_scratch_tab(&mut self, query: String) {
        self.sync_active_query_tab();
        let cursor = query.len();
        self.query_tabs.push(QueryTab {
            query: query.clone(),
            cursor,
            saved_query_id: None,
            name: None,
            saved_snapshot: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        });
        self.active_query_tab = self.query_tabs.len() - 1;
        self.query = query;
        self.cursor = cursor;
        self.saved_query_id = None;
        self.saved_query_name = None;
        self.saved_query_snapshot = None;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.mode = Mode::Normal;
        self.focus = Focus::Editor;
        self.completion_index = 0;
        self.persist_query_tab_session();
    }

    fn persist_query_tab_session(&mut self) {
        let Some(database_name) = self.database_name.as_deref() else {
            return;
        };
        let tabs = self
            .query_tabs
            .iter()
            .enumerate()
            .filter_map(|(index, tab)| {
                tab.saved_query_id
                    .map(|id| (id, index == self.active_query_tab))
            })
            .collect::<Vec<_>>();
        if let Err(error) = self.storage.save_tab_session(database_name, &tabs) {
            self.status_message = Some(format!("Could not save tab session: {error}"));
        }
    }

    fn restore_query_tab_session(&mut self, database_name: &str) {
        if self.restored_database.as_deref() == Some(database_name) {
            return;
        }
        self.restored_database = Some(database_name.to_owned());
        let Ok(mut restored) = self.storage.restore_tab_session(database_name) else {
            self.status_message = Some("Could not restore saved query tabs".into());
            return;
        };
        if restored.is_empty()
            && let Ok(saved) = self.storage.saved_queries(database_name)
            && let Some(most_recent) = saved.into_iter().next()
        {
            restored.push((most_recent, true));
        }
        self.sync_active_query_tab();
        let mut active = None;
        for (saved, is_active) in restored {
            let index = if let Some(index) = self
                .query_tabs
                .iter()
                .position(|tab| tab.saved_query_id == Some(saved.id))
            {
                index
            } else {
                let cursor = saved.sql.len();
                self.query_tabs.push(QueryTab {
                    query: saved.sql.clone(),
                    cursor,
                    saved_query_id: Some(saved.id),
                    name: Some(saved.name),
                    saved_snapshot: Some(saved.sql),
                    undo_stack: Vec::new(),
                    redo_stack: Vec::new(),
                });
                self.query_tabs.len() - 1
            };
            if is_active {
                active = Some(index);
            }
        }
        if let Some(index) = active {
            self.activate_query_tab(index);
        }
    }

    pub fn overlay_active(&self) -> bool {
        self.finder.is_some()
            || self.save_dialog.is_some()
            || self.close_tab_dialog.is_some()
            || self.confirmation_dialog.is_some()
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
                save_as: false,
            });
            return;
        }
        self.persist_saved_query();
    }

    fn save_query_as(&mut self) {
        if self.database_name.is_none() {
            self.status_message = Some("Connect to a database before saving a query".into());
            return;
        }
        self.save_dialog = Some(SaveDialogState {
            input: String::new(),
            save_as: true,
        });
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
                self.saved_query_snapshot = Some(saved.sql.clone());
                self.sync_active_query_tab();
                self.persist_query_tab_session();
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
            FinderKind::Tables => Ok::<Vec<_>, anyhow::Error>(
                self.catalog
                    .schemas
                    .iter()
                    .flat_map(|schema| {
                        schema.tables.iter().map(|table| FinderItem::Table {
                            schema: schema.name.clone(),
                            table: table.clone(),
                        })
                    })
                    .collect(),
            ),
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
        if self.close_tab_dialog.is_some() {
            match character.to_ascii_lowercase() {
                'y' => self.confirm_close_query_tab(),
                'n' => self.close_tab_dialog = None,
                _ => {}
            }
        } else if let Some(dialog) = &mut self.save_dialog {
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

    async fn overlay_accept(&mut self) {
        if self.close_tab_dialog.is_some() {
            self.close_tab_dialog = None;
            return;
        }
        if let Some(dialog) = self.save_dialog.take() {
            let name = dialog.input.trim().to_owned();
            if name.is_empty() {
                self.status_message = Some("Query name cannot be empty".into());
                self.save_dialog = Some(dialog);
                return;
            }
            if dialog.save_as {
                self.saved_query_id = None;
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
        let item = finder.items[index].clone();
        self.inspector = None;
        self.inspector_loading = false;
        self.error = None;
        match item {
            FinderItem::Saved(saved) => {
                self.open_saved_query_tab(saved);
            }
            FinderItem::History(entry) => {
                let matching_saved = self.database_name.as_deref().and_then(|database_name| {
                    self.storage
                        .saved_queries(database_name)
                        .ok()?
                        .into_iter()
                        .find(|saved| saved.sql.trim_end() == entry.sql.trim_end())
                });
                if let Some(saved) = matching_saved {
                    self.open_saved_query_tab(saved);
                } else {
                    self.open_scratch_tab(entry.sql);
                    self.status_message = Some("Opened history in a new scratch tab".into());
                }
            }
            FinderItem::Table { schema, table } => {
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

    fn record_edit(&mut self) {
        const MAX_UNDO_STEPS: usize = 1_000;
        if self.undo_stack.len() == MAX_UNDO_STEPS {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(EditSnapshot {
            query: self.query.clone(),
            cursor: self.cursor,
        });
        self.redo_stack.clear();
    }

    fn undo(&mut self) {
        let Some(snapshot) = self.undo_stack.pop() else {
            self.status_message = Some("Already at oldest change".into());
            return;
        };
        self.redo_stack.push(EditSnapshot {
            query: self.query.clone(),
            cursor: self.cursor,
        });
        self.query = snapshot.query;
        self.cursor = snapshot.cursor.min(self.query.len());
        self.completion_index = 0;
        self.status_message = Some("Undid change".into());
    }

    fn redo(&mut self) {
        let Some(snapshot) = self.redo_stack.pop() else {
            self.status_message = Some("Already at newest change".into());
            return;
        };
        self.undo_stack.push(EditSnapshot {
            query: self.query.clone(),
            cursor: self.cursor,
        });
        self.query = snapshot.query;
        self.cursor = snapshot.cursor.min(self.query.len());
        self.completion_index = 0;
        self.status_message = Some("Redid change".into());
    }

    async fn execute_statement(&mut self, statement: String) {
        self.error = None;
        self.result = None;
        self.result_row = 0;
        self.result_column = 0;
        self.query_running = true;
        self.focus = Focus::Results;
        self.running_statement = Some(statement.clone());
        if self.database.execute(statement).await.is_err() {
            self.query_running = false;
            self.running_statement = None;
            self.error = Some("database worker is unavailable".into());
        }
    }

    fn update_sql_for_selected_cell(&self) -> Result<String, String> {
        let (source, row) = self.selected_result_target()?;
        let value = row
            .get(self.result_column)
            .ok_or_else(|| "Selected result cell is unavailable".to_owned())?;
        Ok(format!(
            "UPDATE {}.{}\nSET {} = {}\nWHERE {};",
            quote_identifier(&source.schema),
            quote_identifier(&source.table),
            quote_identifier(&source.column),
            sql_value(value),
            primary_key_predicate(source, row)?,
        ))
    }

    fn delete_sql_for_selected_row(&self) -> Result<String, String> {
        let (source, row) = self.selected_result_target()?;
        Ok(format!(
            "DELETE FROM {}.{}\nWHERE {};",
            quote_identifier(&source.schema),
            quote_identifier(&source.table),
            primary_key_predicate(source, row)?,
        ))
    }

    fn selected_result_target(
        &self,
    ) -> Result<(&crate::database::ResultColumnSource, &[String]), String> {
        let result = self
            .result
            .as_ref()
            .ok_or_else(|| "No query result is available".to_owned())?;
        let row = result
            .rows
            .get(self.result_row)
            .ok_or_else(|| "Selected result row is unavailable".to_owned())?;
        let source = result
            .sources
            .get(self.result_column)
            .and_then(Option::as_ref)
            .ok_or_else(|| {
                "Cannot identify this cell's source table and complete primary key".to_owned()
            })?;
        Ok((source, row))
    }

    fn yank_result_cell(&mut self) {
        if self.inspector.is_some() {
            self.status_message =
                Some("Table Inspector: press yy for schema or ya for AI prompt".into());
            return;
        }
        let Some(value) = self
            .result
            .as_ref()
            .and_then(|result| result.rows.get(self.result_row))
            .and_then(|row| row.get(self.result_column))
            .cloned()
        else {
            self.status_message = Some("No result cell to copy".into());
            return;
        };
        self.finish_yank(&value, "cell");
    }

    fn yank_result_row(&mut self) {
        if let Some(details) = &self.inspector {
            let value = table_markdown(details);
            self.finish_yank(
                &value,
                &format!("{}.{} schema", details.schema, details.name),
            );
            return;
        }
        let Some(row) = self
            .result
            .as_ref()
            .and_then(|result| result.rows.get(self.result_row))
        else {
            self.status_message = Some("No result row to copy".into());
            return;
        };
        let value = row.join("\t");
        self.finish_yank(&value, "row as TSV");
    }

    fn yank_table_ai_prompt(&mut self) {
        let Some(details) = &self.inspector else {
            self.status_message = Some("Open a table in Table Inspector before using ya".into());
            return;
        };
        let value = table_ai_prompt(details);
        self.finish_yank(
            &value,
            &format!("{}.{} AI schema prompt", details.schema, details.name),
        );
    }

    fn finish_yank(&mut self, value: &str, description: &str) {
        match copy_to_clipboard(value) {
            Ok(()) => self.status_message = Some(format!("Copied {description}")),
            Err(error) => {
                self.status_message = Some(format!("Could not copy {description}: {error}"))
            }
        }
    }
}

fn table_markdown(details: &TableDetails) -> String {
    let mut output = format!(
        "# PostgreSQL table: {}.{}\n\n## Columns\n\n| Column | Type | Nullable | Default | Key | Comment |\n|---|---|---|---|---|---|\n",
        details.schema, details.name
    );
    for column in &details.columns {
        let data_type = if column.enum_values.is_empty() {
            column.data_type.clone()
        } else {
            format!(
                "{} (enum: {})",
                column.data_type,
                column.enum_values.join(", ")
            )
        };
        output.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} |\n",
            markdown_cell(&column.name),
            markdown_cell(&data_type),
            if column.nullable { "yes" } else { "no" },
            markdown_cell(column.default.as_deref().unwrap_or("—")),
            markdown_cell(column.key.as_deref().unwrap_or("—")),
            markdown_cell(column.comment.as_deref().unwrap_or("—")),
        ));
    }

    output.push_str("\n## Constraints\n\n");
    if details.constraints.is_empty() {
        output.push_str("- None\n");
    } else {
        for constraint in &details.constraints {
            output.push_str(&format!(
                "- **{}** `{}`: `{}`\n",
                constraint.kind,
                constraint.name.replace('`', "\\`"),
                constraint.definition.replace('`', "\\`")
            ));
        }
    }

    output.push_str("\n## Indexes\n\n");
    if details.indexes.is_empty() {
        output.push_str("- None\n");
    } else {
        for index in &details.indexes {
            output.push_str(&format!(
                "- `{}`: `{}`\n",
                index.name.replace('`', "\\`"),
                index.definition.replace('`', "\\`")
            ));
        }
    }
    output
}

fn table_ai_prompt(details: &TableDetails) -> String {
    let mut value = table_markdown(details);
    value.push_str(
        "\n## Request\n\nGenerate 20 realistic sample rows for this PostgreSQL table.\n\n\
         Requirements:\n\
         - Respect primary keys, foreign keys, unique constraints, checks, nullability, and enum values.\n\
         - Omit identity and generated columns when PostgreSQL supplies their values.\n\
         - Use PostgreSQL-compatible literals.\n\
         - Return executable INSERT statements only.\n",
    );
    value
}

fn markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace(['\r', '\n'], "<br>")
}

fn primary_key_predicate(
    source: &crate::database::ResultColumnSource,
    row: &[String],
) -> Result<String, String> {
    source
        .primary_key
        .iter()
        .map(|key| {
            let value = row
                .get(key.result_index)
                .ok_or_else(|| format!("Primary-key column '{}' is unavailable", key.name))?;
            if value == "NULL" {
                return Err(format!("Primary-key column '{}' is NULL", key.name));
            }
            Ok(format!(
                "{} = {}",
                quote_identifier(&key.name),
                sql_value(value)
            ))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|predicates| predicates.join(" AND "))
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn sql_value(value: &str) -> String {
    if value == "NULL" {
        "NULL".into()
    } else {
        format!("'{}'", value.replace('\'', "''"))
    }
}

fn is_destructive_sql(sql: &str) -> bool {
    sql.split_whitespace()
        .next()
        .is_some_and(|keyword| keyword.eq_ignore_ascii_case("DELETE"))
}

fn copy_to_clipboard(value: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if pipe_to_command("pbcopy", &[], value).is_ok() {
        return Ok(());
    }

    #[cfg(not(target_os = "macos"))]
    for (program, arguments) in [
        ("wl-copy", &[][..]),
        ("xclip", &["-selection", "clipboard"][..]),
        ("xsel", &["--clipboard", "--input"][..]),
    ] {
        if pipe_to_command(program, arguments, value).is_ok() {
            return Ok(());
        }
    }

    Err("install pbcopy, wl-copy, xclip, or xsel".into())
}

fn pipe_to_command(program: &str, arguments: &[&str], value: &str) -> Result<(), String> {
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| error.to_string())?;
    child
        .stdin
        .take()
        .ok_or_else(|| "clipboard input unavailable".to_owned())?
        .write_all(value.as_bytes())
        .map_err(|error| error.to_string())?;
    child
        .wait()
        .map_err(|error| error.to_string())
        .and_then(|status| {
            status
                .success()
                .then_some(())
                .ok_or_else(|| format!("{program} failed"))
        })
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
    async fn undo_and_redo_restore_query_and_cursor() {
        let mut app = app();
        app.query.clear();
        app.cursor = 0;
        app.undo_stack.clear();
        app.redo_stack.clear();

        app.update(Action::Insert('λ')).await;
        app.update(Action::Insert('x')).await;
        app.update(Action::Undo).await;
        assert_eq!((app.query.as_str(), app.cursor), ("λ", 2));

        app.update(Action::Redo).await;
        assert_eq!((app.query.as_str(), app.cursor), ("λx", 3));
    }

    #[tokio::test]
    async fn undo_history_is_scoped_to_each_query_tab() {
        let mut app = app();
        app.query.clear();
        app.cursor = 0;
        app.undo_stack.clear();

        app.update(Action::Insert('a')).await;
        app.update(Action::NewQueryTab).await;
        app.update(Action::Insert('b')).await;
        app.update(Action::Undo).await;
        assert_eq!(app.query, "");

        app.update(Action::PreviousQueryTab).await;
        app.update(Action::Undo).await;
        assert_eq!(app.query, "");
    }

    fn editable_result() -> QueryResult {
        let source = crate::database::ResultColumnSource {
            schema: "public".into(),
            table: "messages".into(),
            column: "content".into(),
            primary_key: vec![crate::database::ResultKeyColumn {
                name: "id".into(),
                result_index: 0,
            }],
        };
        QueryResult {
            columns: vec!["id".into(), "content".into()],
            rows: vec![vec!["4".into(), "O'Reilly".into()]],
            elapsed: Duration::from_millis(1),
            sources: vec![None, Some(source)],
        }
    }

    #[test]
    fn table_schema_markdown_includes_types_enums_constraints_and_indexes() {
        let details = TableDetails {
            schema: "public".into(),
            name: "messages".into(),
            columns: vec![crate::database::TableColumn {
                name: "role".into(),
                data_type: "message_role".into(),
                nullable: false,
                default: Some("'user'::message_role".into()),
                key: None,
                comment: Some("Who | authored the message".into()),
                enum_values: vec!["user".into(), "assistant".into()],
            }],
            constraints: vec![crate::database::TableConstraint {
                name: "messages_role_check".into(),
                kind: "CHECK".into(),
                definition: "CHECK (role IS NOT NULL)".into(),
            }],
            indexes: vec![crate::database::TableIndex {
                name: "messages_pkey".into(),
                definition: "CREATE UNIQUE INDEX messages_pkey ON public.messages (id)".into(),
            }],
            estimated_rows: 4,
            table_size: "16 kB".into(),
            indexes_size: "16 kB".into(),
            total_size: "32 kB".into(),
        };

        let markdown = table_markdown(&details);

        assert!(markdown.contains("# PostgreSQL table: public.messages"));
        assert!(markdown.contains("message_role (enum: user, assistant)"));
        assert!(markdown.contains("Who \\| authored the message"));
        assert!(markdown.contains("messages_role_check"));
        assert!(markdown.contains("messages_pkey"));
        let prompt = table_ai_prompt(&details);
        assert!(prompt.contains("Generate 20 realistic sample rows"));
        assert!(prompt.contains("Return executable INSERT statements only"));
    }

    #[tokio::test]
    async fn result_edit_opens_primary_key_scoped_update_sql() {
        let mut app = app();
        app.result = Some(editable_result());
        app.result_column = 1;

        app.update(Action::EditResultCell).await;

        assert_eq!(
            app.query,
            "UPDATE \"public\".\"messages\"\nSET \"content\" = 'O''Reilly'\nWHERE \"id\" = '4';"
        );
        assert_eq!(app.query_tabs.len(), 2);
    }

    #[tokio::test]
    async fn table_finder_filters_catalog_tables() {
        let mut app = app();
        app.database_name = Some("project_db".into());
        app.catalog.schemas = vec![crate::database::SchemaCatalog {
            name: "public".into(),
            tables: vec!["agents".into(), "messages".into()],
            views: Vec::new(),
            functions: Vec::new(),
        }];

        app.update(Action::OpenTableFinder).await;
        for character in "agents".chars() {
            app.update(Action::OverlayInsert(character)).await;
        }

        let finder = app.finder.as_ref().expect("table finder");
        assert_eq!(finder.kind, FinderKind::Tables);
        assert_eq!(app.finder_matches().len(), 1);
        assert_eq!(
            finder.items[app.finder_matches()[0]].label(),
            "public.agents"
        );
    }

    #[tokio::test]
    async fn hiding_explorer_moves_focus_and_skips_hidden_panel() {
        let mut app = app();
        app.focus = Focus::Explorer;

        app.update(Action::ToggleExplorer).await;
        assert!(!app.explorer_visible);
        assert_eq!(app.focus, Focus::Editor);

        app.update(Action::FocusPrevious).await;
        assert_eq!(app.focus, Focus::Results);
        app.update(Action::FocusNext).await;
        assert_eq!(app.focus, Focus::Editor);
    }

    #[tokio::test]
    async fn result_delete_requires_confirmation_then_opens_sql() {
        let mut app = app();
        app.result = Some(editable_result());
        app.result_column = 1;

        app.update(Action::RequestDeleteResultRow).await;
        assert!(app.confirmation_dialog.is_some());
        assert_eq!(app.query_tabs.len(), 1);

        app.update(Action::OverlayAccept).await;
        assert_eq!(
            app.query,
            "DELETE FROM \"public\".\"messages\"\nWHERE \"id\" = '4';"
        );
        assert_eq!(app.query_tabs.len(), 2);
    }

    #[tokio::test]
    async fn running_generated_delete_requires_second_confirmation() {
        let mut app = app();
        app.connection = ConnectionState::Connected;
        app.query = "DELETE FROM \"public\".\"messages\" WHERE \"id\" = '4';".into();
        app.cursor = 0;

        app.update(Action::RunQuery).await;

        assert!(app.confirmation_dialog.is_some());
        assert!(!app.query_running);
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
        app.handle_database_event(DatabaseEvent::Connected {
            database_name: "project_db".into(),
            completion_items: Vec::new(),
            relation_items: Vec::new(),
            catalog: DatabaseCatalog::default(),
        });
        assert_eq!(app.saved_query_id, Some(saved_id));
        app.query = "SELECT 2;".into();
        app.update(Action::SaveQuery).await;
        assert!(app.save_dialog.is_none());

        let saved = app.storage.saved_queries("project_db").unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].id, saved_id);
        assert_eq!(saved[0].sql, "SELECT 2;");

        app.update(Action::SaveQueryAs).await;
        for character in "Second check".chars() {
            app.update(Action::OverlayInsert(character)).await;
        }
        app.update(Action::OverlayAccept).await;
        assert_eq!(app.storage.saved_queries("project_db").unwrap().len(), 2);
        assert_eq!(app.saved_query_name.as_deref(), Some("Second check"));
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
    async fn query_tabs_preserve_each_buffers_text_and_saved_identity() {
        let mut app = app();
        app.connection = ConnectionState::Connected;
        app.database_name = Some("project_db".into());
        let first = app
            .storage
            .save_query(None, "project_db", "First", "SELECT 1;")
            .unwrap();
        let second = app
            .storage
            .save_query(None, "project_db", "Second", "SELECT 2;")
            .unwrap();

        app.open_saved_query_tab(first.clone());
        app.query.push_str("\n-- edited");
        app.open_saved_query_tab(second);
        app.update(Action::PreviousQueryTab).await;

        assert_eq!(app.saved_query_id, Some(first.id));
        assert_eq!(app.saved_query_name.as_deref(), Some("First"));
        assert!(app.query.ends_with("-- edited"));
        assert!(app.active_query_is_modified());
    }

    #[tokio::test]
    async fn new_query_tab_opens_an_empty_independent_buffer() {
        let mut app = app();
        app.query = "SELECT important_data;".into();
        app.cursor = app.query.len();

        app.update(Action::NewQueryTab).await;

        assert_eq!(app.query_tabs.len(), 2);
        assert_eq!(app.active_query_tab, 1);
        assert!(app.query.is_empty());
        assert_eq!(app.saved_query_id, None);
        app.update(Action::PreviousQueryTab).await;
        assert_eq!(app.query, "SELECT important_data;");
    }

    #[tokio::test]
    async fn closing_a_saved_tab_never_deletes_its_query_or_file() {
        let mut app = app();
        app.connection = ConnectionState::Connected;
        app.database_name = Some("project_db".into());
        let saved = app
            .storage
            .save_query(None, "project_db", "Keep me", "SELECT 1;")
            .unwrap();
        let file_path = saved.file_path.clone();
        app.open_saved_query_tab(saved);

        app.update(Action::RequestCloseQueryTab).await;
        assert!(app.close_tab_dialog.is_some());
        app.update(Action::OverlayInsert('y')).await;

        assert!(app.close_tab_dialog.is_none());
        assert!(std::path::Path::new(&file_path).exists());
        assert_eq!(app.storage.saved_queries("project_db").unwrap().len(), 1);
        assert!(
            app.storage
                .restore_tab_session("project_db")
                .unwrap()
                .is_empty()
        );
        assert!(
            app.query_tabs
                .iter()
                .all(|tab| tab.name.as_deref() != Some("Keep me"))
        );
    }

    #[tokio::test]
    async fn closing_the_final_tab_leaves_a_fresh_scratch_buffer() {
        let mut app = app();
        app.update(Action::RequestCloseQueryTab).await;
        app.update(Action::ConfirmCloseQueryTab).await;

        assert_eq!(app.query_tabs.len(), 1);
        assert!(app.query.is_empty());
        assert_eq!(app.saved_query_id, None);
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
            sources: vec![None],
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
