use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug, Clone)]
pub struct SavedQuery {
    pub id: i64,
    pub name: String,
    pub sql: String,
    pub database_name: String,
    pub file_path: String,
}

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub id: i64,
    pub sql: String,
    pub success: bool,
    pub duration_ms: Option<i64>,
    pub error: Option<String>,
    pub executed_at: String,
}

pub struct Storage {
    connection: Connection,
    queries_dir: PathBuf,
    workspace: String,
}

impl Storage {
    pub fn open(project_root: &Path) -> Result<Self> {
        let data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| project_root.join(".pgide"))
            .join("pgide");
        fs::create_dir_all(&data_dir).context("create PGIDE data directory")?;
        let queries_dir = project_root.join(".pgide").join("queries");
        fs::create_dir_all(&queries_dir).context("create project query directory")?;
        Self::from_connection(
            Connection::open(data_dir.join("pgide.sqlite")).context("open PGIDE storage")?,
            queries_dir,
            project_root.to_string_lossy().into_owned(),
        )
    }

    #[cfg(test)]
    pub fn memory() -> Result<Self> {
        let queries_dir = std::env::temp_dir().join(format!("pgide-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&queries_dir)?;
        let workspace = queries_dir.to_string_lossy().into_owned();
        Self::from_connection(Connection::open_in_memory()?, queries_dir, workspace)
    }

    fn from_connection(
        connection: Connection,
        queries_dir: PathBuf,
        workspace: String,
    ) -> Result<Self> {
        connection.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS saved_queries (
                 id INTEGER PRIMARY KEY,
                 workspace TEXT NOT NULL,
                 database_name TEXT NOT NULL,
                 name TEXT NOT NULL,
                 sql TEXT NOT NULL,
                 file_path TEXT NOT NULL DEFAULT '',
                 created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                 updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
             );
             CREATE TABLE IF NOT EXISTS query_history (
                 id INTEGER PRIMARY KEY,
                 workspace TEXT NOT NULL,
                 database_name TEXT NOT NULL,
                 sql TEXT NOT NULL,
                 success INTEGER NOT NULL,
                 duration_ms INTEGER,
                 error TEXT,
                 executed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
             );",
        )?;
        ensure_column(
            &connection,
            "saved_queries",
            "workspace",
            "TEXT NOT NULL DEFAULT ''",
        )?;
        ensure_column(
            &connection,
            "query_history",
            "workspace",
            "TEXT NOT NULL DEFAULT ''",
        )?;
        connection.execute_batch(
            "CREATE INDEX IF NOT EXISTS saved_queries_workspace_database
                 ON saved_queries(workspace, database_name, updated_at DESC);
             CREATE INDEX IF NOT EXISTS query_history_workspace_database
                 ON query_history(workspace, database_name, executed_at DESC);",
        )?;
        Ok(Self {
            connection,
            queries_dir,
            workspace,
        })
    }

    pub fn save_query(
        &mut self,
        existing_id: Option<i64>,
        database_name: &str,
        name: &str,
        sql: &str,
    ) -> Result<SavedQuery> {
        let transaction = self.connection.transaction()?;
        let (id, file_path) = if let Some(id) = existing_id {
            let file_path = transaction
                .query_row(
                    "SELECT file_path FROM saved_queries WHERE id = ?1",
                    [id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .filter(|path| !path.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| self.queries_dir.join(format!("{}-{id}.sql", slug(name))));
            transaction.execute(
                "UPDATE saved_queries SET workspace=?1, database_name=?2, name=?3, sql=?4,
                 file_path=?5, updated_at=CURRENT_TIMESTAMP WHERE id=?6",
                params![
                    self.workspace,
                    database_name,
                    name,
                    sql,
                    file_path.to_string_lossy(),
                    id
                ],
            )?;
            (id, file_path)
        } else {
            transaction.execute(
                "INSERT INTO saved_queries(workspace, database_name, name, sql)
                 VALUES (?1, ?2, ?3, ?4)",
                params![self.workspace, database_name, name, sql],
            )?;
            let id = transaction.last_insert_rowid();
            let file_path = self.queries_dir.join(format!("{}-{id}.sql", slug(name)));
            transaction.execute(
                "UPDATE saved_queries SET file_path=?1 WHERE id=?2",
                params![file_path.to_string_lossy(), id],
            )?;
            (id, file_path)
        };
        atomic_write(&file_path, sql)?;
        transaction.commit()?;
        Ok(SavedQuery {
            id,
            name: name.into(),
            sql: sql.into(),
            database_name: database_name.into(),
            file_path: file_path.to_string_lossy().into_owned(),
        })
    }

    pub fn saved_queries(&self, database_name: &str) -> Result<Vec<SavedQuery>> {
        let mut statement = self.connection.prepare(
            "SELECT id, name, sql, database_name, file_path FROM saved_queries
             WHERE workspace=?1 AND database_name=?2 ORDER BY updated_at DESC",
        )?;
        let rows = statement.query_map(params![self.workspace, database_name], |row| {
            Ok(SavedQuery {
                id: row.get(0)?,
                name: row.get(1)?,
                sql: row.get(2)?,
                database_name: row.get(3)?,
                file_path: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn record_history(
        &self,
        database_name: &str,
        sql: &str,
        success: bool,
        duration_ms: Option<i64>,
        error: Option<&str>,
    ) -> Result<()> {
        self.connection.execute(
            "INSERT INTO query_history(
                 workspace, database_name, sql, success, duration_ms, error
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                self.workspace,
                database_name,
                sql,
                success,
                duration_ms,
                error
            ],
        )?;
        Ok(())
    }

    pub fn history(&self, database_name: &str, limit: usize) -> Result<Vec<HistoryEntry>> {
        let mut statement = self.connection.prepare(
            "SELECT id, sql, success, duration_ms, error, executed_at FROM query_history
             WHERE workspace=?1 AND database_name=?2 ORDER BY id DESC LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![self.workspace, database_name, limit as i64],
            |row| {
                Ok(HistoryEntry {
                    id: row.get(0)?,
                    sql: row.get(1)?,
                    success: row.get(2)?,
                    duration_ms: row.get(3)?,
                    error: row.get(4)?,
                    executed_at: row.get(5)?,
                })
            },
        )?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

fn ensure_column(
    connection: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<()> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?;
    if !columns.iter().any(|existing| existing == column) {
        connection.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {definition}"
        ))?;
    }
    Ok(())
}

fn slug(name: &str) -> String {
    let value = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let value = value
        .trim_matches('-')
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if value.is_empty() {
        "query".into()
    } else {
        value
    }
}

fn atomic_write(path: &Path, sql: &str) -> Result<()> {
    let temporary = path.with_extension("sql.tmp");
    fs::write(&temporary, format!("{}\n", sql.trim_end())).context("write saved query")?;
    fs::rename(&temporary, path).context("publish saved query")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_updates_and_lists_queries() {
        let mut storage = Storage::memory().unwrap();
        let saved = storage
            .save_query(None, "app", "Active Users", "SELECT 1;")
            .unwrap();
        storage
            .save_query(Some(saved.id), "app", "Active Users", "SELECT 2;")
            .unwrap();
        let queries = storage.saved_queries("app").unwrap();
        assert_eq!(queries.len(), 1);
        assert_eq!(queries[0].sql, "SELECT 2;");
        assert!(Path::new(&queries[0].file_path).exists());
    }

    #[test]
    fn history_is_scoped_to_database() {
        let storage = Storage::memory().unwrap();
        storage
            .record_history("one", "SELECT 1", true, Some(1), None)
            .unwrap();
        storage
            .record_history("two", "SELECT 2", false, None, Some("failed"))
            .unwrap();
        assert_eq!(storage.history("one", 100).unwrap().len(), 1);
    }

    #[test]
    fn saved_queries_are_scoped_to_workspace() {
        let root = std::env::temp_dir().join(format!("pgide-scope-{}", uuid::Uuid::new_v4()));
        let database = root.join("storage.sqlite");
        let first_queries = root.join("first");
        let second_queries = root.join("second");
        fs::create_dir_all(&first_queries).unwrap();
        fs::create_dir_all(&second_queries).unwrap();
        let mut first = Storage::from_connection(
            Connection::open(&database).unwrap(),
            first_queries,
            "workspace-one".into(),
        )
        .unwrap();
        let second = Storage::from_connection(
            Connection::open(&database).unwrap(),
            second_queries,
            "workspace-two".into(),
        )
        .unwrap();

        first
            .save_query(None, "app", "Only here", "SELECT 1;")
            .unwrap();

        assert_eq!(first.saved_queries("app").unwrap().len(), 1);
        assert!(second.saved_queries("app").unwrap().is_empty());
    }
}
