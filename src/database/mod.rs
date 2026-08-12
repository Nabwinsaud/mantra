use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use tokio::sync::mpsc;
use tokio_postgres::{
    Client, NoTls, Row,
    types::{FromSql, Type},
};

#[derive(Debug, Clone)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub elapsed: Duration,
}

#[derive(Debug, Clone, Default)]
pub struct DatabaseCatalog {
    pub schemas: Vec<SchemaCatalog>,
}

#[derive(Debug, Clone, Default)]
pub struct SchemaCatalog {
    pub name: String,
    pub tables: Vec<String>,
    pub views: Vec<String>,
    pub functions: Vec<String>,
}

#[derive(Debug)]
pub enum DatabaseEvent {
    Connected {
        database_name: String,
        completion_items: Vec<String>,
        relation_items: Vec<String>,
        catalog: DatabaseCatalog,
    },
    ConnectionFailed(String),
    QueryFinished(QueryResult),
    QueryFailed(String),
}

enum DatabaseCommand {
    Connect(String),
    Execute(String),
}

#[derive(Clone)]
pub struct DatabaseService {
    commands: mpsc::Sender<DatabaseCommand>,
}

impl DatabaseService {
    pub fn spawn() -> (Self, mpsc::Receiver<DatabaseEvent>) {
        let (commands_tx, mut commands_rx) = mpsc::channel(16);
        let (events_tx, events_rx) = mpsc::channel(16);

        tokio::spawn(async move {
            let mut client: Option<Client> = None;
            while let Some(command) = commands_rx.recv().await {
                match command {
                    DatabaseCommand::Connect(url) => {
                        match tokio_postgres::connect(&url, NoTls).await {
                            Ok((new_client, connection)) => {
                                tokio::spawn(async move {
                                    if let Err(error) = connection.await {
                                        tracing::warn!(%error, "PostgreSQL connection ended");
                                    }
                                });
                                let database_name = new_client
                                    .query_one("SELECT current_database()", &[])
                                    .await
                                    .ok()
                                    .and_then(|row| row.try_get::<_, String>(0).ok())
                                    .unwrap_or_else(|| "postgres".into());
                                let (completion_items, relation_items) =
                                    load_completion_items(&new_client).await;
                                let catalog = load_catalog(&new_client).await;
                                client = Some(new_client);
                                let _ = events_tx
                                    .send(DatabaseEvent::Connected {
                                        database_name,
                                        completion_items,
                                        relation_items,
                                        catalog,
                                    })
                                    .await;
                            }
                            Err(error) => {
                                client = None;
                                let _ = events_tx
                                    .send(DatabaseEvent::ConnectionFailed(safe_error(&error)))
                                    .await;
                            }
                        }
                    }
                    DatabaseCommand::Execute(sql) => {
                        let Some(active_client) = client.as_ref() else {
                            let _ = events_tx
                                .send(DatabaseEvent::QueryFailed("not connected".into()))
                                .await;
                            continue;
                        };
                        let started = Instant::now();
                        let statement = match active_client.prepare(&sql).await {
                            Ok(statement) => statement,
                            Err(error) => {
                                let _ = events_tx
                                    .send(DatabaseEvent::QueryFailed(safe_error(&error)))
                                    .await;
                                continue;
                            }
                        };
                        let columns = statement
                            .columns()
                            .iter()
                            .map(|column| column.name().to_owned())
                            .collect();
                        match active_client.query(&statement, &[]).await {
                            Ok(rows) => {
                                let result =
                                    QueryResult::from_rows(columns, rows, started.elapsed());
                                let _ = events_tx.send(DatabaseEvent::QueryFinished(result)).await;
                            }
                            Err(error) => {
                                let _ = events_tx
                                    .send(DatabaseEvent::QueryFailed(safe_error(&error)))
                                    .await;
                            }
                        }
                    }
                }
            }
        });

        (
            Self {
                commands: commands_tx,
            },
            events_rx,
        )
    }

    pub async fn connect(&self, url: String) -> Result<(), mpsc::error::SendError<()>> {
        self.commands
            .send(DatabaseCommand::Connect(url))
            .await
            .map_err(|_| mpsc::error::SendError(()))
    }

    pub async fn execute(&self, sql: String) -> Result<(), mpsc::error::SendError<()>> {
        self.commands
            .send(DatabaseCommand::Execute(sql))
            .await
            .map_err(|_| mpsc::error::SendError(()))
    }
}

impl QueryResult {
    fn from_rows(columns: Vec<String>, rows: Vec<Row>, elapsed: Duration) -> Self {
        let rows = rows
            .iter()
            .map(|row| {
                (0..row.len())
                    .map(|index| display_cell(row, index))
                    .collect()
            })
            .collect();
        Self {
            columns,
            rows,
            elapsed,
        }
    }
}

fn display_cell(row: &Row, index: usize) -> String {
    let kind = row
        .columns()
        .get(index)
        .map(|column| column.type_())
        .unwrap_or(&Type::UNKNOWN);
    match *kind {
        Type::BOOL => format_optional(row.try_get::<_, Option<bool>>(index)),
        Type::INT2 => format_optional(row.try_get::<_, Option<i16>>(index)),
        Type::INT4 => format_optional(row.try_get::<_, Option<i32>>(index)),
        Type::INT8 => format_optional(row.try_get::<_, Option<i64>>(index)),
        Type::FLOAT4 => format_optional(row.try_get::<_, Option<f32>>(index)),
        Type::FLOAT8 => format_optional(row.try_get::<_, Option<f64>>(index)),
        Type::UUID => format_optional(row.try_get::<_, Option<uuid::Uuid>>(index)),
        Type::TIMESTAMP => format_optional(row.try_get::<_, Option<chrono::NaiveDateTime>>(index)),
        Type::TIMESTAMPTZ => {
            format_optional(row.try_get::<_, Option<chrono::DateTime<chrono::Utc>>>(index))
        }
        Type::DATE => format_optional(row.try_get::<_, Option<chrono::NaiveDate>>(index)),
        Type::TIME => format_optional(row.try_get::<_, Option<chrono::NaiveTime>>(index)),
        Type::JSON | Type::JSONB => {
            format_optional(row.try_get::<_, Option<serde_json::Value>>(index))
        }
        Type::BYTEA => row
            .try_get::<_, Option<Vec<u8>>>(index)
            .map(|value| {
                value.map_or_else(|| "NULL".into(), |bytes| format!("<{} bytes>", bytes.len()))
            })
            .unwrap_or_else(|_| "<bytea>".into()),
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME => {
            format_optional(row.try_get::<_, Option<String>>(index))
        }
        _ => row
            .try_get::<_, Option<RawValue>>(index)
            .map(|value| value.map_or_else(|| "NULL".into(), |raw| raw.0))
            .unwrap_or_else(|_| format!("<{}>", type_name(row, index))),
    }
}

#[derive(Debug)]
struct RawValue(String);

impl<'a> FromSql<'a> for RawValue {
    fn from_sql(
        _ty: &Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Self(String::from_utf8_lossy(raw).into_owned()))
    }

    fn accepts(_ty: &Type) -> bool {
        true
    }
}

fn format_optional<T: ToString>(value: Result<Option<T>, tokio_postgres::Error>) -> String {
    match value {
        Ok(Some(value)) => value.to_string(),
        Ok(None) => "NULL".into(),
        Err(_) => "<unavailable>".into(),
    }
}

fn type_name(row: &Row, index: usize) -> &str {
    row.columns()
        .get(index)
        .map(|column| column.type_().name())
        .unwrap_or(Type::UNKNOWN.name())
}

fn safe_error(error: &tokio_postgres::Error) -> String {
    error.as_db_error().map_or_else(
        || "PostgreSQL connection or protocol error".into(),
        |db| {
            let mut message = db.message().to_owned();
            if let Some(detail) = db.detail() {
                message.push_str(": ");
                message.push_str(detail);
            }
            message
        },
    )
}

async fn load_completion_items(client: &Client) -> (Vec<String>, Vec<String>) {
    let sql = "
        SELECT table_schema, table_name, column_name
        FROM information_schema.columns
        WHERE table_schema NOT IN ('pg_catalog', 'information_schema')
        ORDER BY table_schema, table_name, ordinal_position
    ";
    let Ok(rows) = client.query(sql, &[]).await else {
        return (Vec::new(), Vec::new());
    };
    let mut items = Vec::new();
    let mut relations = Vec::new();
    for row in rows {
        let schema: String = row.get(0);
        let table: String = row.get(1);
        let column: String = row.get(2);
        items.extend([
            schema.clone(),
            table.clone(),
            column.clone(),
            format!("{schema}.{table}"),
            format!("{table}.{column}"),
            format!("{schema}.{table}.{column}"),
        ]);
        relations.extend([schema.clone(), table.clone(), format!("{schema}.{table}")]);
    }
    items.sort_unstable();
    items.dedup();
    relations.sort_unstable();
    relations.dedup();
    (items, relations)
}

async fn load_catalog(client: &Client) -> DatabaseCatalog {
    let mut schemas = BTreeMap::<String, SchemaCatalog>::new();
    let objects_sql = "
        SELECT table_schema, table_name, table_type
        FROM information_schema.tables
        WHERE table_schema NOT IN ('pg_catalog', 'information_schema')
        ORDER BY table_schema, table_name
    ";
    if let Ok(rows) = client.query(objects_sql, &[]).await {
        for row in rows {
            let schema_name: String = row.get(0);
            let object_name: String = row.get(1);
            let object_type: String = row.get(2);
            let schema = schemas
                .entry(schema_name.clone())
                .or_insert_with(|| SchemaCatalog {
                    name: schema_name,
                    ..SchemaCatalog::default()
                });
            if object_type == "VIEW" {
                schema.views.push(object_name);
            } else {
                schema.tables.push(object_name);
            }
        }
    }
    let functions_sql = "
        SELECT n.nspname, p.proname || '(' || pg_get_function_identity_arguments(p.oid) || ')'
        FROM pg_proc p
        JOIN pg_namespace n ON n.oid = p.pronamespace
        WHERE n.nspname NOT IN ('pg_catalog', 'information_schema')
          AND n.nspname NOT LIKE 'pg_toast%'
        ORDER BY n.nspname, p.proname
    ";
    if let Ok(rows) = client.query(functions_sql, &[]).await {
        for row in rows {
            let schema_name: String = row.get(0);
            let function_name: String = row.get(1);
            schemas
                .entry(schema_name.clone())
                .or_insert_with(|| SchemaCatalog {
                    name: schema_name,
                    ..SchemaCatalog::default()
                })
                .functions
                .push(function_name);
        }
    }
    DatabaseCatalog {
        schemas: schemas.into_values().collect(),
    }
}
