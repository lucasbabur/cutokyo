//! Tool-call analytics and the bounded read-only SQL surface for agents.
//!
//! Both read the derived projections only. The SQL surface runs one statement on a
//! fresh read-only connection behind a SQLite authorizer that admits reads of
//! projection tables and nothing else, with row, byte and time limits.

use std::time::Instant;

use cutokyo_domain::RunState;
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    types::ValueRef,
};
use serde::{Deserialize, Serialize};

use super::{
    ContractError, Duration, ErrorCode, Harness, ReadStore, Result, SessionId, Timestamp, Value,
    open_read_connection, params, sqlite_error,
};

/// Maximum distinct tool names returned by one statistics read.
const MAX_TOOL_CALL_STATS: u32 = 2_000;
/// Maximum samples in one page.
pub const MAX_TOOL_CALL_SAMPLES: u32 = 50;
/// Maximum rows returned by one SQL query.
pub const MAX_SQL_ROWS: u32 = 500;
/// Maximum characters kept from one text cell before truncation.
pub(super) const MAX_SQL_CELL_CHARS: usize = 4_000;
/// Maximum serialized result bytes before remaining rows are dropped.
const MAX_SQL_RESULT_BYTES: usize = 1024 * 1024;
/// Wall-clock budget for one SQL query.
const SQL_QUERY_TIMEOUT: Duration = Duration::from_secs(10);
/// Derived projection tables an agent may read through SQL. Raw evidence, health,
/// spool, import bookkeeping and configuration snapshots stay out of reach.
pub const SQL_READABLE_TABLES: [&str; 12] = [
    "sessions",
    "projects",
    "turns",
    "messages",
    "tool_calls",
    "agent_runs",
    "usage_values",
    "summaries",
    "summary_sessions",
    "price_snapshots",
    "quota_windows",
    "context_breakdowns",
];

/// Filters shared by tool-call statistics and samples.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolCallFilter {
    /// Inclusive start of the call's start time.
    pub from: Option<Timestamp>,
    /// Exact harness.
    pub harness: Option<Harness>,
}

/// Outcome counts for one native tool name.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCallStat {
    /// Native tool name, exactly as the harness reported it.
    pub tool_name: String,
    /// Harnesses that called it.
    pub harnesses: Vec<Harness>,
    /// All calls.
    pub total: u64,
    /// Calls that completed successfully.
    pub succeeded: u64,
    /// Calls that completed with failure.
    pub failed: u64,
    /// Running, cancelled, or unknown calls.
    pub other: u64,
    /// Start time of the newest call.
    pub last_called_at: Timestamp,
}

/// Which outcomes a sample page lists.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallOutcome {
    /// Every call.
    #[default]
    All,
    /// Successful calls only.
    Succeeded,
    /// Failed calls only.
    Failed,
}

/// One page of calls for a tool name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolCallSampleQuery {
    /// Exact native tool name.
    pub tool_name: String,
    /// Outcome filter.
    pub outcome: ToolCallOutcome,
    /// Shared filters.
    pub filter: ToolCallFilter,
    /// Calls to skip, newest first.
    pub offset: u32,
    /// Page size, at most [`MAX_TOOL_CALL_SAMPLES`].
    pub limit: u32,
}

/// One stored call with its native input and output.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCallSample {
    /// Stable tool-call identity.
    pub tool_call_id: String,
    /// Parent session.
    pub session_id: SessionId,
    /// Parent session title, when known.
    pub session_title: Option<String>,
    /// Harness of the parent session.
    pub harness: Harness,
    /// Execution state.
    pub state: RunState,
    /// Start time.
    pub started_at: Timestamp,
    /// End time, when observed.
    pub ended_at: Option<Timestamp>,
    /// Native input, when retained.
    pub input: Option<Value>,
    /// Native output, when retained.
    pub output: Option<Value>,
}

/// Bounded page of samples with the exact matching count.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCallSamplePage {
    /// Samples, newest first.
    pub samples: Vec<ToolCallSample>,
    /// Exact number of matching calls.
    pub total: u64,
    /// Whether more calls follow this page.
    pub has_more: bool,
}

/// Result of one read-only SQL query.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SqlQueryResult {
    /// Result column names.
    pub columns: Vec<String>,
    /// Rows as JSON arrays in column order.
    pub rows: Vec<Vec<Value>>,
    /// True when the row limit, byte budget or cell truncation cut the result.
    pub truncated: bool,
}

fn filter_clause(filter: &ToolCallFilter) -> (String, Vec<rusqlite::types::Value>) {
    use rusqlite::types::Value as SqlValue;
    let mut clause =
        String::from(" FROM tool_calls t JOIN sessions s ON s.session_id=t.session_id WHERE 1=1");
    let mut values = Vec::new();
    if let Some(from) = &filter.from {
        clause.push_str(" AND t.started_at_epoch >= ?");
        values.push(SqlValue::Integer(from.unix_timestamp()));
    }
    if let Some(harness) = filter.harness {
        clause.push_str(" AND s.harness = ?");
        values.push(SqlValue::Text(harness.as_str().to_owned()));
    }
    (clause, values)
}

fn decode_harness(value: &str) -> Result<Harness> {
    serde_json::from_value(Value::String(value.to_owned())).map_err(|_| {
        ContractError::new(
            ErrorCode::InvalidContract,
            "stored harness is not supported",
        )
    })
}

fn decode_json(value: Option<String>, what: &str) -> Result<Option<Value>> {
    value
        .map(|text| serde_json::from_str(&text))
        .transpose()
        .map_err(|error| {
            ContractError::new(
                ErrorCode::InvalidContract,
                format!("failed to decode {what}: {error}"),
            )
        })
}

fn count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

impl ReadStore {
    /// Counts outcomes per native tool name, most-called first.
    ///
    /// # Errors
    ///
    /// Returns a store read or decoding error.
    pub fn tool_call_stats(&self, filter: &ToolCallFilter) -> Result<Vec<ToolCallStat>> {
        let connection = open_read_connection(&self.path)?;
        let (clause, mut values) = filter_clause(filter);
        values.push(rusqlite::types::Value::Integer(i64::from(
            MAX_TOOL_CALL_STATS,
        )));
        let sql = format!(
            "SELECT t.tool_name, group_concat(DISTINCT s.harness), count(*), sum(t.state='succeeded'), sum(t.state='failed'), max(t.started_at_epoch), t.started_at{clause} GROUP BY t.tool_name ORDER BY count(*) DESC, t.tool_name LIMIT ?"
        );
        let mut statement = connection
            .prepare(&sql)
            .map_err(|error| sqlite_error("prepare tool-call statistics", &error))?;
        let rows = statement
            .query_map(rusqlite::params_from_iter(values), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(6)?,
                ))
            })
            .map_err(|error| sqlite_error("query tool-call statistics", &error))?;
        rows.map(|row| {
            let (tool_name, harnesses, total, succeeded, failed, last) =
                row.map_err(|error| sqlite_error("read tool-call statistics", &error))?;
            let mut harnesses = harnesses
                .split(',')
                .map(decode_harness)
                .collect::<Result<Vec<_>>>()?;
            harnesses.sort_by_key(|harness| harness.as_str());
            Ok(ToolCallStat {
                tool_name,
                harnesses,
                total: count(total),
                succeeded: count(succeeded),
                failed: count(failed),
                other: count(total - succeeded - failed),
                last_called_at: Timestamp::parse(last)?,
            })
        })
        .collect()
    }

    /// Lists calls of one tool, newest first, with their stored input and output.
    ///
    /// # Errors
    ///
    /// Returns invalid input for an empty name or oversized page, or a store error.
    pub fn tool_call_samples(&self, query: &ToolCallSampleQuery) -> Result<ToolCallSamplePage> {
        if query.tool_name.is_empty() || query.tool_name.len() > 512 {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "tool name must be 1 to 512 bytes",
            ));
        }
        if query.limit == 0 || query.limit > MAX_TOOL_CALL_SAMPLES {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                format!("sample limit must be 1 to {MAX_TOOL_CALL_SAMPLES}"),
            ));
        }
        let mut connection = open_read_connection(&self.path)?;
        let transaction = connection
            .transaction()
            .map_err(|error| sqlite_error("begin tool-call sample snapshot", &error))?;
        let (mut clause, mut values) = filter_clause(&query.filter);
        clause.push_str(" AND t.tool_name = ?");
        values.push(rusqlite::types::Value::Text(query.tool_name.clone()));
        match query.outcome {
            ToolCallOutcome::All => {}
            ToolCallOutcome::Succeeded => clause.push_str(" AND t.state = 'succeeded'"),
            ToolCallOutcome::Failed => clause.push_str(" AND t.state = 'failed'"),
        }
        let total: i64 = transaction
            .query_row(
                &format!("SELECT count(*){clause}"),
                rusqlite::params_from_iter(values.clone()),
                |row| row.get(0),
            )
            .map_err(|error| sqlite_error("count tool-call samples", &error))?;
        values.push(rusqlite::types::Value::Integer(i64::from(query.limit)));
        values.push(rusqlite::types::Value::Integer(i64::from(query.offset)));
        let samples = {
            let mut statement = transaction
                .prepare(&format!(
                    "SELECT t.tool_call_id, t.session_id, s.title, s.harness, t.state, t.started_at, t.ended_at, t.input_json, t.output_json{clause} ORDER BY t.started_at_epoch DESC, t.tool_call_id LIMIT ? OFFSET ?"
                ))
                .map_err(|error| sqlite_error("prepare tool-call samples", &error))?;
            let rows = statement
                .query_map(rusqlite::params_from_iter(values), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                    ))
                })
                .map_err(|error| sqlite_error("query tool-call samples", &error))?;
            rows.map(|row| {
                let row = row.map_err(|error| sqlite_error("read tool-call sample", &error))?;
                Ok(ToolCallSample {
                    tool_call_id: row.0,
                    session_id: SessionId::parse(row.1)?,
                    session_title: row.2,
                    harness: decode_harness(&row.3)?,
                    state: serde_json::from_value(Value::String(row.4)).map_err(|error| {
                        ContractError::new(
                            ErrorCode::InvalidContract,
                            format!("failed to decode tool-call state: {error}"),
                        )
                    })?,
                    started_at: Timestamp::parse(row.5)?,
                    ended_at: row.6.map(Timestamp::parse).transpose()?,
                    input: decode_json(row.7, "tool input")?,
                    output: decode_json(row.8, "tool output")?,
                })
            })
            .collect::<Result<Vec<_>>>()?
        };
        let total = count(total);
        let has_more = u64::from(query.offset) + (samples.len() as u64) < total;
        Ok(ToolCallSamplePage {
            samples,
            total,
            has_more,
        })
    }

    /// Runs one read-only `SELECT` over the derived projection tables.
    ///
    /// # Errors
    ///
    /// Returns invalid input for anything other than a single read of the allowed
    /// tables, for a query that exceeds the time budget, or for SQL errors.
    pub fn sql_query(&self, sql: &str, max_rows: u32) -> Result<SqlQueryResult> {
        if max_rows == 0 || max_rows > MAX_SQL_ROWS {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                format!("row limit must be 1 to {MAX_SQL_ROWS}"),
            ));
        }
        let connection = open_read_connection(&self.path)?;
        connection
            .authorizer(Some(authorize_projection_read))
            .map_err(|error| sqlite_error("install SQL authorizer", &error))?;
        let deadline = Instant::now() + SQL_QUERY_TIMEOUT;
        connection
            .progress_handler(10_000, Some(move || Instant::now() > deadline))
            .map_err(|error| sqlite_error("install SQL time budget", &error))?;
        let mut statement = connection.prepare(sql).map_err(|error| {
            let hint = if error.sqlite_error_code()
                == Some(rusqlite::ErrorCode::AuthorizationForStatementDenied)
            {
                format!(
                    "; only one SELECT over {} is allowed",
                    SQL_READABLE_TABLES.join(", ")
                )
            } else {
                String::new()
            };
            ContractError::new(
                ErrorCode::InvalidInput,
                format!("SQL was rejected: {error}{hint}"),
            )
        })?;
        if !statement.readonly() {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "only read-only SELECT statements are allowed",
            ));
        }
        let columns = statement
            .column_names()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let mut rows = statement
            .query(params![])
            .map_err(|error| query_error(&error))?;
        let mut result = SqlQueryResult {
            columns,
            rows: Vec::new(),
            truncated: false,
        };
        let mut bytes = 0_usize;
        while let Some(row) = rows.next().map_err(|error| query_error(&error))? {
            if result.rows.len() >= max_rows as usize {
                result.truncated = true;
                break;
            }
            let mut cells = Vec::with_capacity(result.columns.len());
            for index in 0..result.columns.len() {
                let value = row.get_ref(index).map_err(|error| query_error(&error))?;
                let (cell, cut) = json_cell(value);
                result.truncated |= cut;
                cells.push(cell);
            }
            bytes += serde_json::to_vec(&cells).map_or(0, |encoded| encoded.len());
            if bytes > MAX_SQL_RESULT_BYTES {
                result.truncated = true;
                break;
            }
            result.rows.push(cells);
        }
        Ok(result)
    }
}

fn query_error(error: &rusqlite::Error) -> ContractError {
    if matches!(
        error.sqlite_error_code(),
        Some(rusqlite::ErrorCode::OperationInterrupted)
    ) {
        return ContractError::new(
            ErrorCode::InvalidInput,
            format!(
                "SQL query exceeded the {} second budget; add filters or a LIMIT",
                SQL_QUERY_TIMEOUT.as_secs()
            ),
        );
    }
    ContractError::new(
        ErrorCode::InvalidInput,
        format!("SQL query failed: {error}"),
    )
}

fn authorize_projection_read(context: AuthContext<'_>) -> Authorization {
    match context.action {
        AuthAction::Select | AuthAction::Function { .. } | AuthAction::Recursive => {
            Authorization::Allow
        }
        AuthAction::Read { table_name, .. }
            if matches!(context.database_name, None | Some("main"))
                && (SQL_READABLE_TABLES.contains(&table_name)
                    || matches!(
                        table_name,
                        "sqlite_schema"
                            | "sqlite_master"
                            | "pragma_table_info"
                            | "pragma_table_xinfo"
                    )) =>
        {
            Authorization::Allow
        }
        AuthAction::Pragma {
            pragma_name: "table_info" | "table_xinfo",
            ..
        } => Authorization::Allow,
        _ => Authorization::Deny,
    }
}

fn json_cell(value: ValueRef<'_>) -> (Value, bool) {
    match value {
        ValueRef::Null => (Value::Null, false),
        ValueRef::Integer(number) => (Value::from(number), false),
        ValueRef::Real(number) => (Value::from(number), false),
        ValueRef::Text(bytes) => {
            let text = String::from_utf8_lossy(bytes);
            if text.chars().count() > MAX_SQL_CELL_CHARS {
                let kept = text.chars().take(MAX_SQL_CELL_CHARS).collect::<String>();
                (Value::String(format!("{kept}…")), true)
            } else {
                (Value::String(text.into_owned()), false)
            }
        }
        ValueRef::Blob(bytes) => (
            Value::String(format!("<blob {} bytes>", bytes.len())),
            false,
        ),
    }
}

#[cfg(test)]
#[path = "store_tool_calls_tests.rs"]
mod tests;
