use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use serde::Serialize;
use tokio::sync::mpsc;
use tokio_postgres::{
    Client, Column, NoTls, Row,
    types::{FromSql, Type},
};

#[derive(Debug, Clone)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub elapsed: Duration,
    pub sources: Vec<Option<ResultColumnSource>>,
}

#[derive(Debug, Clone)]
pub struct ResultColumnSource {
    pub schema: String,
    pub table: String,
    pub column: String,
    pub primary_key: Vec<ResultKeyColumn>,
}

#[derive(Debug, Clone)]
pub struct ResultKeyColumn {
    pub name: String,
    pub result_index: usize,
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

#[derive(Debug, Clone, Default)]
pub struct TableDetails {
    pub schema: String,
    pub name: String,
    pub columns: Vec<TableColumn>,
    pub constraints: Vec<TableConstraint>,
    pub indexes: Vec<TableIndex>,
    pub estimated_rows: i64,
    pub table_size: String,
    pub indexes_size: String,
    pub total_size: String,
}

#[derive(Debug, Clone)]
pub struct TableColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub key: Option<String>,
    pub comment: Option<String>,
    pub enum_values: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct TableConstraint {
    pub name: String,
    pub kind: String,
    pub definition: String,
}

#[derive(Debug, Clone)]
pub struct TableIndex {
    pub name: String,
    pub definition: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SchemaDiagram {
    pub database: String,
    pub tables: Vec<DiagramTable>,
    pub relationships: Vec<DiagramRelationship>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagramTable {
    pub schema: String,
    pub name: String,
    pub kind: String,
    pub estimated_rows: i64,
    pub columns: Vec<DiagramColumn>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagramColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub comment: Option<String>,
    pub primary_key: bool,
    pub unique: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagramRelationship {
    pub name: String,
    pub source_schema: String,
    pub source_table: String,
    pub source_columns: Vec<String>,
    pub target_schema: String,
    pub target_table: String,
    pub target_columns: Vec<String>,
    pub source_optional: bool,
    pub source_unique: bool,
    pub on_update: String,
    pub on_delete: String,
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
    TransactionFinished {
        elapsed: Duration,
        completion_items: Vec<String>,
        relation_items: Vec<String>,
        catalog: DatabaseCatalog,
    },
    QueryFailed(String),
    TableInspected(TableDetails),
    TableInspectionFailed(String),
    SchemaDiagramLoaded(SchemaDiagram),
    SchemaDiagramFailed(String),
}

enum DatabaseCommand {
    Connect(String),
    Execute(String),
    ExecuteTransaction(String),
    InspectTable { schema: String, table: String },
    LoadSchemaDiagram,
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
                        let sources = load_result_sources(active_client, statement.columns()).await;
                        match active_client.query(&statement, &[]).await {
                            Ok(rows) => {
                                let result = QueryResult::from_rows(
                                    columns,
                                    sources,
                                    rows,
                                    started.elapsed(),
                                );
                                let _ = events_tx.send(DatabaseEvent::QueryFinished(result)).await;
                            }
                            Err(error) => {
                                let _ = events_tx
                                    .send(DatabaseEvent::QueryFailed(safe_error(&error)))
                                    .await;
                            }
                        }
                    }
                    DatabaseCommand::ExecuteTransaction(sql) => {
                        let Some(active_client) = client.as_mut() else {
                            let _ = events_tx
                                .send(DatabaseEvent::QueryFailed("not connected".into()))
                                .await;
                            continue;
                        };
                        let started = Instant::now();
                        let transaction = match active_client.transaction().await {
                            Ok(transaction) => transaction,
                            Err(error) => {
                                let _ = events_tx
                                    .send(DatabaseEvent::QueryFailed(safe_error(&error)))
                                    .await;
                                continue;
                            }
                        };
                        match transaction.batch_execute(&sql).await {
                            Ok(()) => match transaction.commit().await {
                                Ok(()) => {
                                    let elapsed = started.elapsed();
                                    let (completion_items, relation_items) =
                                        load_completion_items(active_client).await;
                                    let catalog = load_catalog(active_client).await;
                                    let _ = events_tx
                                        .send(DatabaseEvent::TransactionFinished {
                                            elapsed,
                                            completion_items,
                                            relation_items,
                                            catalog,
                                        })
                                        .await;
                                }
                                Err(error) => {
                                    let _ = events_tx
                                        .send(DatabaseEvent::QueryFailed(format!(
                                            "Transaction was not committed: {}",
                                            safe_error(&error)
                                        )))
                                        .await;
                                }
                            },
                            Err(error) => {
                                let message = safe_error(&error);
                                let rollback_error = transaction.rollback().await.err();
                                let message = rollback_error.map_or_else(
                                    || format!("Transaction rolled back: {message}"),
                                    |rollback_error| {
                                        format!(
                                            "Transaction failed: {message}; rollback also failed: {}",
                                            safe_error(&rollback_error)
                                        )
                                    },
                                );
                                let _ = events_tx.send(DatabaseEvent::QueryFailed(message)).await;
                            }
                        }
                    }
                    DatabaseCommand::InspectTable { schema, table } => {
                        let Some(active_client) = client.as_ref() else {
                            let _ = events_tx
                                .send(DatabaseEvent::TableInspectionFailed("not connected".into()))
                                .await;
                            continue;
                        };
                        match inspect_table(active_client, &schema, &table).await {
                            Ok(details) => {
                                let _ =
                                    events_tx.send(DatabaseEvent::TableInspected(details)).await;
                            }
                            Err(error) => {
                                let _ = events_tx
                                    .send(DatabaseEvent::TableInspectionFailed(safe_error(&error)))
                                    .await;
                            }
                        }
                    }
                    DatabaseCommand::LoadSchemaDiagram => {
                        let Some(active_client) = client.as_ref() else {
                            let _ = events_tx
                                .send(DatabaseEvent::SchemaDiagramFailed("not connected".into()))
                                .await;
                            continue;
                        };
                        match load_schema_diagram(active_client).await {
                            Ok(diagram) => {
                                let _ = events_tx
                                    .send(DatabaseEvent::SchemaDiagramLoaded(diagram))
                                    .await;
                            }
                            Err(error) => {
                                let _ = events_tx
                                    .send(DatabaseEvent::SchemaDiagramFailed(safe_error(&error)))
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

    pub async fn execute_transaction(&self, sql: String) -> Result<(), mpsc::error::SendError<()>> {
        self.commands
            .send(DatabaseCommand::ExecuteTransaction(sql))
            .await
            .map_err(|_| mpsc::error::SendError(()))
    }

    pub async fn inspect_table(
        &self,
        schema: String,
        table: String,
    ) -> Result<(), mpsc::error::SendError<()>> {
        self.commands
            .send(DatabaseCommand::InspectTable { schema, table })
            .await
            .map_err(|_| mpsc::error::SendError(()))
    }

    pub async fn load_schema_diagram(&self) -> Result<(), mpsc::error::SendError<()>> {
        self.commands
            .send(DatabaseCommand::LoadSchemaDiagram)
            .await
            .map_err(|_| mpsc::error::SendError(()))
    }
}

impl QueryResult {
    fn from_rows(
        columns: Vec<String>,
        sources: Vec<Option<ResultColumnSource>>,
        rows: Vec<Row>,
        elapsed: Duration,
    ) -> Self {
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
            sources,
        }
    }
}

async fn load_result_sources(
    client: &Client,
    columns: &[Column],
) -> Vec<Option<ResultColumnSource>> {
    let mut tables = BTreeMap::new();
    for oid in columns.iter().filter_map(Column::table_oid) {
        if tables.contains_key(&oid) {
            continue;
        }
        if let Some(table) = load_result_table(client, oid).await {
            tables.insert(oid, table);
        }
    }

    columns
        .iter()
        .map(|column| {
            let oid = column.table_oid()?;
            let column_id = column.column_id()?;
            let table = tables.get(&oid)?;
            let source_column = table
                .columns
                .iter()
                .find(|(attribute, _)| *attribute == column_id)?
                .1
                .clone();
            let primary_key = table
                .primary_key
                .iter()
                .map(|(attribute, name)| {
                    columns
                        .iter()
                        .position(|candidate| {
                            candidate.table_oid() == Some(oid)
                                && candidate.column_id() == Some(*attribute)
                        })
                        .map(|result_index| ResultKeyColumn {
                            name: name.clone(),
                            result_index,
                        })
                })
                .collect::<Option<Vec<_>>>()?;
            (!primary_key.is_empty()).then(|| ResultColumnSource {
                schema: table.schema.clone(),
                table: table.table.clone(),
                column: source_column,
                primary_key,
            })
        })
        .collect()
}

struct ResultTable {
    schema: String,
    table: String,
    columns: Vec<(i16, String)>,
    primary_key: Vec<(i16, String)>,
}

async fn load_result_table(client: &Client, oid: u32) -> Option<ResultTable> {
    let rows = client
        .query(
            "SELECT n.nspname, c.relname, a.attnum, a.attname,
                    EXISTS (
                        SELECT 1 FROM pg_index i
                        WHERE i.indrelid = c.oid AND i.indisprimary
                          AND a.attnum = ANY(i.indkey)
                    ) AS is_primary
             FROM pg_class c
             JOIN pg_namespace n ON n.oid = c.relnamespace
             JOIN pg_attribute a ON a.attrelid = c.oid
             WHERE c.oid = $1::oid AND a.attnum > 0 AND NOT a.attisdropped
             ORDER BY a.attnum",
            &[&oid],
        )
        .await
        .ok()?;
    let first = rows.first()?;
    let mut table = ResultTable {
        schema: first.get(0),
        table: first.get(1),
        columns: Vec::new(),
        primary_key: Vec::new(),
    };
    for row in rows {
        let attribute: i16 = row.get(2);
        let name: String = row.get(3);
        table.columns.push((attribute, name.clone()));
        if row.get(4) {
            table.primary_key.push((attribute, name));
        }
    }
    Some(table)
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
        Type::NUMERIC => format_optional(row.try_get::<_, Option<PgNumeric>>(index)),
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
struct PgNumeric(String);

impl std::fmt::Display for PgNumeric {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<'a> FromSql<'a> for PgNumeric {
    fn from_sql(
        _ty: &Type,
        raw: &'a [u8],
    ) -> Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        decode_numeric(raw)
            .map(Self)
            .ok_or_else(|| "invalid PostgreSQL numeric value".into())
    }

    fn accepts(ty: &Type) -> bool {
        *ty == Type::NUMERIC
    }
}

fn decode_numeric(raw: &[u8]) -> Option<String> {
    const POSITIVE: u16 = 0x0000;
    const NEGATIVE: u16 = 0x4000;
    const NAN: u16 = 0xC000;
    const POSITIVE_INFINITY: u16 = 0xD000;
    const NEGATIVE_INFINITY: u16 = 0xF000;

    if raw.len() < 8 {
        return None;
    }
    let digits_count = u16::from_be_bytes([raw[0], raw[1]]) as usize;
    let weight = i16::from_be_bytes([raw[2], raw[3]]) as i32;
    let sign = u16::from_be_bytes([raw[4], raw[5]]);
    let scale = u16::from_be_bytes([raw[6], raw[7]]) as usize;
    if raw.len() != 8 + digits_count * 2 {
        return None;
    }
    match sign {
        NAN => return Some("NaN".into()),
        POSITIVE_INFINITY => return Some("Infinity".into()),
        NEGATIVE_INFINITY => return Some("-Infinity".into()),
        POSITIVE | NEGATIVE => {}
        _ => return None,
    }

    let digits = raw[8..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|bytes| u16::from_be_bytes(*bytes))
        .collect::<Vec<_>>();
    if digits.iter().any(|digit| *digit > 9_999) {
        return None;
    }
    let digit_at = |position: i32| {
        let index = weight - position;
        usize::try_from(index)
            .ok()
            .and_then(|index| digits.get(index))
            .copied()
            .unwrap_or(0)
    };

    let mut value = String::new();
    if sign == NEGATIVE && digits.iter().any(|digit| *digit != 0) {
        value.push('-');
    }
    if weight < 0 {
        value.push('0');
    } else {
        for position in (0..=weight).rev() {
            let digit = digit_at(position);
            if position == weight {
                value.push_str(&digit.to_string());
            } else {
                value.push_str(&format!("{digit:04}"));
            }
        }
    }
    if scale > 0 {
        value.push('.');
        let groups = scale.div_ceil(4);
        for group in 1..=groups {
            value.push_str(&format!("{:04}", digit_at(-(group as i32))));
        }
        value.truncate(value.len() - (groups * 4 - scale));
    }
    Some(value)
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

async fn load_schema_diagram(client: &Client) -> Result<SchemaDiagram, tokio_postgres::Error> {
    let database = client
        .query_one("SELECT current_database()", &[])
        .await?
        .get::<_, String>(0);
    let table_rows = client
        .query(
            "
            SELECT
                n.nspname,
                c.relname,
                CASE c.relkind
                    WHEN 'r' THEN 'table'
                    WHEN 'p' THEN 'partitioned table'
                    WHEN 'v' THEN 'view'
                    WHEN 'm' THEN 'materialized view'
                    WHEN 'f' THEN 'foreign table'
                    ELSE 'relation'
                END,
                GREATEST(c.reltuples, 0)::bigint
            FROM pg_class c
            JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f')
              AND n.nspname NOT IN ('pg_catalog', 'information_schema')
              AND n.nspname NOT LIKE 'pg_toast%'
            ORDER BY n.nspname, c.relname
            ",
            &[],
        )
        .await?;
    let mut tables = BTreeMap::<String, DiagramTable>::new();
    for row in table_rows {
        let schema = row.get::<_, String>(0);
        let name = row.get::<_, String>(1);
        tables.insert(
            format!("{schema}.{name}"),
            DiagramTable {
                schema,
                name,
                kind: row.get(2),
                estimated_rows: row.get(3),
                columns: Vec::new(),
            },
        );
    }

    let column_rows = client
        .query(
            "
            SELECT
                n.nspname,
                c.relname,
                a.attname,
                format_type(a.atttypid, a.atttypmod),
                NOT a.attnotnull,
                pg_get_expr(ad.adbin, ad.adrelid),
                col_description(c.oid, a.attnum),
                EXISTS (
                    SELECT 1
                    FROM pg_constraint con
                    WHERE con.conrelid = c.oid
                      AND con.contype = 'p'
                      AND a.attnum = ANY(con.conkey)
                ),
                EXISTS (
                    SELECT 1
                    FROM pg_constraint con
                    WHERE con.conrelid = c.oid
                      AND con.contype = 'u'
                      AND cardinality(con.conkey) = 1
                      AND a.attnum = ANY(con.conkey)
                )
            FROM pg_class c
            JOIN pg_namespace n ON n.oid = c.relnamespace
            JOIN pg_attribute a ON a.attrelid = c.oid
            LEFT JOIN pg_attrdef ad ON ad.adrelid = c.oid AND ad.adnum = a.attnum
            WHERE c.relkind IN ('r', 'p', 'v', 'm', 'f')
              AND n.nspname NOT IN ('pg_catalog', 'information_schema')
              AND n.nspname NOT LIKE 'pg_toast%'
              AND a.attnum > 0
              AND NOT a.attisdropped
            ORDER BY n.nspname, c.relname, a.attnum
            ",
            &[],
        )
        .await?;
    for row in column_rows {
        let schema = row.get::<_, String>(0);
        let table = row.get::<_, String>(1);
        if let Some(entry) = tables.get_mut(&format!("{schema}.{table}")) {
            entry.columns.push(DiagramColumn {
                name: row.get(2),
                data_type: row.get(3),
                nullable: row.get(4),
                default: row.get(5),
                comment: row.get(6),
                primary_key: row.get(7),
                unique: row.get(8),
            });
        }
    }

    let relationship_rows = client
        .query(
            "
            SELECT
                con.conname,
                source_ns.nspname,
                source_table.relname,
                array_agg(source_column.attname::text ORDER BY source_key.ordinality)::text[],
                target_ns.nspname,
                target_table.relname,
                array_agg(target_column.attname::text ORDER BY source_key.ordinality)::text[],
                bool_or(NOT source_column.attnotnull),
                EXISTS (
                    SELECT 1
                    FROM pg_constraint uniqueness
                    WHERE uniqueness.conrelid = con.conrelid
                      AND uniqueness.contype IN ('p', 'u')
                      AND uniqueness.conkey = con.conkey
                ),
                CASE con.confupdtype
                    WHEN 'a' THEN 'NO ACTION'
                    WHEN 'r' THEN 'RESTRICT'
                    WHEN 'c' THEN 'CASCADE'
                    WHEN 'n' THEN 'SET NULL'
                    WHEN 'd' THEN 'SET DEFAULT'
                END,
                CASE con.confdeltype
                    WHEN 'a' THEN 'NO ACTION'
                    WHEN 'r' THEN 'RESTRICT'
                    WHEN 'c' THEN 'CASCADE'
                    WHEN 'n' THEN 'SET NULL'
                    WHEN 'd' THEN 'SET DEFAULT'
                END
            FROM pg_constraint con
            JOIN pg_class source_table ON source_table.oid = con.conrelid
            JOIN pg_namespace source_ns ON source_ns.oid = source_table.relnamespace
            JOIN pg_class target_table ON target_table.oid = con.confrelid
            JOIN pg_namespace target_ns ON target_ns.oid = target_table.relnamespace
            JOIN LATERAL unnest(con.conkey) WITH ORDINALITY
                AS source_key(attnum, ordinality) ON TRUE
            JOIN LATERAL unnest(con.confkey) WITH ORDINALITY
                AS target_key(attnum, ordinality)
                ON target_key.ordinality = source_key.ordinality
            JOIN pg_attribute source_column
                ON source_column.attrelid = con.conrelid
               AND source_column.attnum = source_key.attnum
            JOIN pg_attribute target_column
                ON target_column.attrelid = con.confrelid
               AND target_column.attnum = target_key.attnum
            WHERE con.contype = 'f'
              AND source_ns.nspname NOT IN ('pg_catalog', 'information_schema')
              AND source_ns.nspname NOT LIKE 'pg_toast%'
            GROUP BY
                con.oid,
                con.conname,
                source_ns.nspname,
                source_table.relname,
                target_ns.nspname,
                target_table.relname,
                con.conrelid,
                con.conkey,
                con.confupdtype,
                con.confdeltype
            ORDER BY source_ns.nspname, source_table.relname, con.conname
            ",
            &[],
        )
        .await?;
    let relationships = relationship_rows
        .into_iter()
        .map(|row| DiagramRelationship {
            name: row.get(0),
            source_schema: row.get(1),
            source_table: row.get(2),
            source_columns: row.get(3),
            target_schema: row.get(4),
            target_table: row.get(5),
            target_columns: row.get(6),
            source_optional: row.get(7),
            source_unique: row.get(8),
            on_update: row.get(9),
            on_delete: row.get(10),
        })
        .collect();

    Ok(SchemaDiagram {
        database,
        tables: tables.into_values().collect(),
        relationships,
    })
}

async fn inspect_table(
    client: &Client,
    schema: &str,
    table: &str,
) -> Result<TableDetails, tokio_postgres::Error> {
    let columns_sql = "
        SELECT
            a.attname,
            format_type(a.atttypid, a.atttypmod),
            NOT a.attnotnull,
            pg_get_expr(ad.adbin, ad.adrelid),
            (
                SELECT string_agg(
                    CASE c.contype WHEN 'p' THEN 'PRIMARY' WHEN 'u' THEN 'UNIQUE'
                                    WHEN 'f' THEN 'FOREIGN' ELSE NULL END,
                    ', '
                )
                FROM pg_constraint c
                WHERE c.conrelid = cls.oid AND a.attnum = ANY(c.conkey)
                  AND c.contype IN ('p', 'u', 'f')
            ),
            col_description(cls.oid, a.attnum),
            ARRAY(
                SELECT enum.enumlabel
                FROM pg_enum enum
                WHERE enum.enumtypid = a.atttypid
                ORDER BY enum.enumsortorder
            )
        FROM pg_attribute a
        JOIN pg_class cls ON cls.oid = a.attrelid
        JOIN pg_namespace n ON n.oid = cls.relnamespace
        LEFT JOIN pg_attrdef ad ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum
        WHERE n.nspname = $1 AND cls.relname = $2
          AND a.attnum > 0 AND NOT a.attisdropped
        ORDER BY a.attnum
    ";
    let columns = client
        .query(columns_sql, &[&schema, &table])
        .await?
        .into_iter()
        .map(|row| TableColumn {
            name: row.get(0),
            data_type: row.get(1),
            nullable: row.get(2),
            default: row.get(3),
            key: row.get(4),
            comment: row.get(5),
            enum_values: row.get(6),
        })
        .collect();

    let constraints_sql = "
        SELECT con.conname,
               CASE con.contype WHEN 'p' THEN 'PRIMARY KEY' WHEN 'f' THEN 'FOREIGN KEY'
                    WHEN 'u' THEN 'UNIQUE' WHEN 'c' THEN 'CHECK' WHEN 'x' THEN 'EXCLUSION'
                    ELSE con.contype::text END,
               pg_get_constraintdef(con.oid, true)
        FROM pg_constraint con
        JOIN pg_class cls ON cls.oid = con.conrelid
        JOIN pg_namespace n ON n.oid = cls.relnamespace
        WHERE n.nspname = $1 AND cls.relname = $2
        ORDER BY con.contype, con.conname
    ";
    let constraints = client
        .query(constraints_sql, &[&schema, &table])
        .await?
        .into_iter()
        .map(|row| TableConstraint {
            name: row.get(0),
            kind: row.get(1),
            definition: row.get(2),
        })
        .collect();

    let indexes = client
        .query(
            "SELECT indexname, indexdef FROM pg_indexes
             WHERE schemaname = $1 AND tablename = $2 ORDER BY indexname",
            &[&schema, &table],
        )
        .await?
        .into_iter()
        .map(|row| TableIndex {
            name: row.get(0),
            definition: row.get(1),
        })
        .collect();

    let stats = client
        .query_one(
            "SELECT cls.reltuples::bigint,
                    pg_size_pretty(pg_relation_size(cls.oid)),
                    pg_size_pretty(pg_indexes_size(cls.oid)),
                    pg_size_pretty(pg_total_relation_size(cls.oid))
             FROM pg_class cls JOIN pg_namespace n ON n.oid = cls.relnamespace
             WHERE n.nspname = $1 AND cls.relname = $2",
            &[&schema, &table],
        )
        .await?;

    Ok(TableDetails {
        schema: schema.into(),
        name: table.into(),
        columns,
        constraints,
        indexes,
        estimated_rows: stats.get(0),
        table_size: stats.get(1),
        indexes_size: stats.get(2),
        total_size: stats.get(3),
    })
}

#[cfg(test)]
mod tests {
    use super::decode_numeric;

    fn numeric(weight: i16, sign: u16, scale: u16, digits: &[u16]) -> Vec<u8> {
        let mut raw = Vec::with_capacity(8 + digits.len() * 2);
        raw.extend_from_slice(&(digits.len() as u16).to_be_bytes());
        raw.extend_from_slice(&weight.to_be_bytes());
        raw.extend_from_slice(&sign.to_be_bytes());
        raw.extend_from_slice(&scale.to_be_bytes());
        for digit in digits {
            raw.extend_from_slice(&digit.to_be_bytes());
        }
        raw
    }

    #[test]
    fn decodes_postgres_numeric_wire_values() {
        assert_eq!(
            decode_numeric(&numeric(-1, 0, 4, &[842])),
            Some("0.0842".into())
        );
        assert_eq!(
            decode_numeric(&numeric(0, 0, 2, &[184, 2_000])),
            Some("184.20".into())
        );
        assert_eq!(
            decode_numeric(&numeric(1, 0, 3, &[12, 3456, 7_000])),
            Some("123456.700".into())
        );
        assert_eq!(
            decode_numeric(&numeric(0, 0x4000, 2, &[42, 5_000])),
            Some("-42.50".into())
        );
        assert_eq!(decode_numeric(&numeric(0, 0, 2, &[])), Some("0.00".into()));
        assert_eq!(
            decode_numeric(&numeric(0, 0xC000, 0, &[])),
            Some("NaN".into())
        );
    }

    #[test]
    fn rejects_malformed_postgres_numeric_values() {
        assert_eq!(decode_numeric(&[]), None);
        assert_eq!(decode_numeric(&numeric(0, 0, 0, &[10_000])), None);
    }
}
