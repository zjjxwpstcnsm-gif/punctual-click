use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

use chrono::{DateTime, Utc};
use punctual_core::{
    ClickMode, ClickTask, CompletionSignal, ExecutionLog, ExecutionOutcome, ExecutionResult,
    TargetRule, TaskStatus,
};
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction};
use thiserror::Error;
use url::Url;
use uuid::Uuid;

const MIGRATION_0001: &str = include_str!("../migrations/0001_init.sql");
const MIGRATION_0002: &str = include_str!("../migrations/0002_execution_logs.sql");

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid UUID in database: {0}")]
    InvalidUuid(#[from] uuid::Error),
    #[error("invalid URL in database: {0}")]
    InvalidUrl(#[from] url::ParseError),
    #[error("invalid millisecond timestamp in database: {0}")]
    InvalidTimestamp(i64),
    #[error("database mutex was poisoned")]
    Poisoned,
}

pub type StorageResult<T> = Result<T, StorageError>;

pub struct SqliteTaskRepository {
    connection: Mutex<Connection>,
}

impl SqliteTaskRepository {
    pub fn open(path: impl AsRef<Path>) -> StorageResult<Self> {
        let connection = Connection::open(path)?;
        Self::from_connection(connection)
    }

    pub fn open_in_memory() -> StorageResult<Self> {
        let connection = Connection::open_in_memory()?;
        Self::from_connection(connection)
    }

    fn from_connection(connection: Connection) -> StorageResult<Self> {
        connection.pragma_update(None, "foreign_keys", "ON")?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.execute_batch(MIGRATION_0001)?;
        connection.execute_batch(MIGRATION_0002)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn connection(&self) -> StorageResult<MutexGuard<'_, Connection>> {
        self.connection.lock().map_err(|_| StorageError::Poisoned)
    }

    pub fn save(&self, task: &ClickTask) -> StorageResult<()> {
        let connection = self.connection()?;
        upsert_task(&connection, task)
    }

    pub fn save_task_and_log(&self, task: &ClickTask, log: &ExecutionLog) -> StorageResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        upsert_task_transaction(&transaction, task)?;
        insert_execution_log_transaction(&transaction, log)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn get(&self, id: Uuid) -> StorageResult<Option<ClickTask>> {
        let connection = self.connection()?;
        let stored = connection
            .query_row(
                "SELECT * FROM click_tasks WHERE id = ?1",
                [id.to_string()],
                StoredTask::from_row,
            )
            .optional()?;
        stored.map(TryInto::try_into).transpose()
    }

    pub fn list(&self) -> StorageResult<Vec<ClickTask>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT * FROM click_tasks ORDER BY scheduled_at_ms ASC, created_at_ms ASC",
        )?;
        let rows = statement.query_map([], StoredTask::from_row)?;
        rows.map(|row| row.map_err(StorageError::from).and_then(TryInto::try_into))
            .collect()
    }

    pub fn list_non_terminal(&self) -> StorageResult<Vec<ClickTask>> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|task| !task.status.is_terminal())
            .collect())
    }

    pub fn delete(&self, id: Uuid) -> StorageResult<bool> {
        let connection = self.connection()?;
        let affected = connection.execute(
            "DELETE FROM click_tasks WHERE id = ?1",
            [id.to_string()],
        )?;
        Ok(affected > 0)
    }

    pub fn replace_all(&self, tasks: &[ClickTask]) -> StorageResult<()> {
        let mut connection = self.connection()?;
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM click_tasks", [])?;
        for task in tasks {
            upsert_task_transaction(&transaction, task)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn insert_execution_log(&self, log: &ExecutionLog) -> StorageResult<()> {
        let connection = self.connection()?;
        insert_execution_log(&connection, log)
    }

    pub fn list_execution_logs(&self, task_id: Uuid) -> StorageResult<Vec<ExecutionLog>> {
        let connection = self.connection()?;
        let mut statement = connection.prepare(
            "SELECT * FROM execution_logs WHERE task_id = ?1 ORDER BY created_at_ms DESC",
        )?;
        let rows = statement.query_map([task_id.to_string()], StoredExecutionLog::from_row)?;
        rows.map(|row| row.map_err(StorageError::from).and_then(TryInto::try_into))
            .collect()
    }
}

struct SerializedTask {
    id: String,
    title: String,
    url: String,
    scheduled_at_ms: i64,
    timezone: String,
    click_mode_json: String,
    target_json: String,
    completion_json: String,
    status_json: String,
    result_json: Option<String>,
    created_at_ms: i64,
    updated_at_ms: i64,
}

impl SerializedTask {
    fn from_task(task: &ClickTask) -> StorageResult<Self> {
        Ok(Self {
            id: task.id.to_string(),
            title: task.title.clone(),
            url: task.url.to_string(),
            scheduled_at_ms: task.scheduled_at_utc.timestamp_millis(),
            timezone: task.timezone.clone(),
            click_mode_json: serde_json::to_string(&task.click_mode)?,
            target_json: serde_json::to_string(&task.target)?,
            completion_json: serde_json::to_string(&task.completion_signals)?,
            status_json: serde_json::to_string(&task.status)?,
            result_json: task
                .result
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
            created_at_ms: task.created_at.timestamp_millis(),
            updated_at_ms: task.updated_at.timestamp_millis(),
        })
    }
}

const UPSERT_TASK_SQL: &str = r#"
    INSERT INTO click_tasks (
        id, title, url, scheduled_at_ms, timezone,
        click_mode_json, target_json, completion_json,
        status_json, result_json, created_at_ms, updated_at_ms
    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
    ON CONFLICT(id) DO UPDATE SET
        title = excluded.title,
        url = excluded.url,
        scheduled_at_ms = excluded.scheduled_at_ms,
        timezone = excluded.timezone,
        click_mode_json = excluded.click_mode_json,
        target_json = excluded.target_json,
        completion_json = excluded.completion_json,
        status_json = excluded.status_json,
        result_json = excluded.result_json,
        updated_at_ms = excluded.updated_at_ms
"#;

fn upsert_task(connection: &Connection, task: &ClickTask) -> StorageResult<()> {
    let task = SerializedTask::from_task(task)?;
    connection.execute(
        UPSERT_TASK_SQL,
        params![
            task.id,
            task.title,
            task.url,
            task.scheduled_at_ms,
            task.timezone,
            task.click_mode_json,
            task.target_json,
            task.completion_json,
            task.status_json,
            task.result_json,
            task.created_at_ms,
            task.updated_at_ms,
        ],
    )?;
    Ok(())
}

fn upsert_task_transaction(transaction: &Transaction<'_>, task: &ClickTask) -> StorageResult<()> {
    let task = SerializedTask::from_task(task)?;
    transaction.execute(
        UPSERT_TASK_SQL,
        params![
            task.id,
            task.title,
            task.url,
            task.scheduled_at_ms,
            task.timezone,
            task.click_mode_json,
            task.target_json,
            task.completion_json,
            task.status_json,
            task.result_json,
            task.created_at_ms,
            task.updated_at_ms,
        ],
    )?;
    Ok(())
}

fn insert_execution_log(connection: &Connection, log: &ExecutionLog) -> StorageResult<()> {
    connection.execute(
        r#"
        INSERT INTO execution_logs (
            id, task_id, scheduled_at_ms, dispatched_at_ms,
            observed_click_at_ms, dispatch_delay_ms, observed_delay_ms,
            outcome_json, final_url, message, error_code,
            screenshot_path, created_at_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            log.id.to_string(),
            log.task_id.to_string(),
            log.scheduled_at.timestamp_millis(),
            log.dispatched_at.map(|value| value.timestamp_millis()),
            log.observed_click_at.map(|value| value.timestamp_millis()),
            log.dispatch_delay_ms,
            log.observed_delay_ms,
            serde_json::to_string(&log.outcome)?,
            log.final_url.as_ref().map(Url::as_str),
            log.message.as_str(),
            log.error_code.as_deref(),
            log.screenshot_path.as_deref(),
            log.created_at.timestamp_millis(),
        ],
    )?;
    Ok(())
}

fn insert_execution_log_transaction(
    transaction: &Transaction<'_>,
    log: &ExecutionLog,
) -> StorageResult<()> {
    transaction.execute(
        r#"
        INSERT INTO execution_logs (
            id, task_id, scheduled_at_ms, dispatched_at_ms,
            observed_click_at_ms, dispatch_delay_ms, observed_delay_ms,
            outcome_json, final_url, message, error_code,
            screenshot_path, created_at_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        "#,
        params![
            log.id.to_string(),
            log.task_id.to_string(),
            log.scheduled_at.timestamp_millis(),
            log.dispatched_at.map(|value| value.timestamp_millis()),
            log.observed_click_at.map(|value| value.timestamp_millis()),
            log.dispatch_delay_ms,
            log.observed_delay_ms,
            serde_json::to_string(&log.outcome)?,
            log.final_url.as_ref().map(Url::as_str),
            log.message.as_str(),
            log.error_code.as_deref(),
            log.screenshot_path.as_deref(),
            log.created_at.timestamp_millis(),
        ],
    )?;
    Ok(())
}

#[derive(Debug)]
struct StoredTask {
    id: String,
    title: String,
    url: String,
    scheduled_at_ms: i64,
    timezone: String,
    click_mode_json: String,
    target_json: String,
    completion_json: String,
    status_json: String,
    result_json: Option<String>,
    created_at_ms: i64,
    updated_at_ms: i64,
}

impl StoredTask {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            title: row.get("title")?,
            url: row.get("url")?,
            scheduled_at_ms: row.get("scheduled_at_ms")?,
            timezone: row.get("timezone")?,
            click_mode_json: row.get("click_mode_json")?,
            target_json: row.get("target_json")?,
            completion_json: row.get("completion_json")?,
            status_json: row.get("status_json")?,
            result_json: row.get("result_json")?,
            created_at_ms: row.get("created_at_ms")?,
            updated_at_ms: row.get("updated_at_ms")?,
        })
    }
}

impl TryFrom<StoredTask> for ClickTask {
    type Error = StorageError;

    fn try_from(value: StoredTask) -> Result<Self, Self::Error> {
        Ok(Self {
            id: Uuid::parse_str(&value.id)?,
            title: value.title,
            url: Url::parse(&value.url)?,
            scheduled_at_utc: datetime_from_millis(value.scheduled_at_ms)?,
            timezone: value.timezone,
            click_mode: serde_json::from_str::<ClickMode>(&value.click_mode_json)?,
            target: serde_json::from_str::<TargetRule>(&value.target_json)?,
            completion_signals: serde_json::from_str::<Vec<CompletionSignal>>(
                &value.completion_json,
            )?,
            status: serde_json::from_str::<TaskStatus>(&value.status_json)?,
            result: value
                .result_json
                .as_deref()
                .map(serde_json::from_str::<ExecutionResult>)
                .transpose()?,
            created_at: datetime_from_millis(value.created_at_ms)?,
            updated_at: datetime_from_millis(value.updated_at_ms)?,
        })
    }
}

#[derive(Debug)]
struct StoredExecutionLog {
    id: String,
    task_id: String,
    scheduled_at_ms: i64,
    dispatched_at_ms: Option<i64>,
    observed_click_at_ms: Option<i64>,
    dispatch_delay_ms: Option<i64>,
    observed_delay_ms: Option<i64>,
    outcome_json: String,
    final_url: Option<String>,
    message: String,
    error_code: Option<String>,
    screenshot_path: Option<String>,
    created_at_ms: i64,
}

impl StoredExecutionLog {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            task_id: row.get("task_id")?,
            scheduled_at_ms: row.get("scheduled_at_ms")?,
            dispatched_at_ms: row.get("dispatched_at_ms")?,
            observed_click_at_ms: row.get("observed_click_at_ms")?,
            dispatch_delay_ms: row.get("dispatch_delay_ms")?,
            observed_delay_ms: row.get("observed_delay_ms")?,
            outcome_json: row.get("outcome_json")?,
            final_url: row.get("final_url")?,
            message: row.get("message")?,
            error_code: row.get("error_code")?,
            screenshot_path: row.get("screenshot_path")?,
            created_at_ms: row.get("created_at_ms")?,
        })
    }
}

impl TryFrom<StoredExecutionLog> for ExecutionLog {
    type Error = StorageError;

    fn try_from(value: StoredExecutionLog) -> Result<Self, Self::Error> {
        Ok(Self {
            id: Uuid::parse_str(&value.id)?,
            task_id: Uuid::parse_str(&value.task_id)?,
            scheduled_at: datetime_from_millis(value.scheduled_at_ms)?,
            dispatched_at: value
                .dispatched_at_ms
                .map(datetime_from_millis)
                .transpose()?,
            observed_click_at: value
                .observed_click_at_ms
                .map(datetime_from_millis)
                .transpose()?,
            dispatch_delay_ms: value.dispatch_delay_ms,
            observed_delay_ms: value.observed_delay_ms,
            outcome: serde_json::from_str::<ExecutionOutcome>(&value.outcome_json)?,
            final_url: value.final_url.as_deref().map(Url::parse).transpose()?,
            message: value.message,
            error_code: value.error_code,
            screenshot_path: value.screenshot_path,
            created_at: datetime_from_millis(value.created_at_ms)?,
        })
    }
}

fn datetime_from_millis(value: i64) -> StorageResult<DateTime<Utc>> {
    DateTime::<Utc>::from_timestamp_millis(value).ok_or(StorageError::InvalidTimestamp(value))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Duration;
    use punctual_core::{TargetFingerprint, TargetRule, TaskStatus};

    use super::*;

    fn target() -> TargetFingerprint {
        TargetFingerprint {
            role: "button".into(),
            accessible_name: "立即购买".into(),
            visible_text: "立即购买".into(),
            stable_attributes: BTreeMap::new(),
            context_text: None,
            selector_hint: Some("#buy".into()),
            shadow_path: Vec::new(),
            frame_path: Vec::new(),
        }
    }

    fn sample_task() -> ClickTask {
        let mut task = ClickTask::new(
            "立即购买",
            Url::parse("https://example.com/product/42").unwrap(),
            Utc::now() + Duration::minutes(10),
            "Asia/Tokyo",
            TargetRule::Auto {
                selected: Some(target()),
            },
        )
        .unwrap();
        task.transition(TaskStatus::Pending).unwrap();
        task
    }

    #[test]
    fn saves_and_loads_a_task_without_losing_fields() {
        let repository = SqliteTaskRepository::open_in_memory().unwrap();
        let task = sample_task();
        repository.save(&task).unwrap();

        let loaded = repository.get(task.id).unwrap().unwrap();
        assert_eq!(loaded, task);
    }

    #[test]
    fn upsert_replaces_mutable_fields() {
        let repository = SqliteTaskRepository::open_in_memory().unwrap();
        let mut task = sample_task();
        repository.save(&task).unwrap();
        task.title = "提交订单".into();
        task.transition(TaskStatus::Preparing).unwrap();
        repository.save(&task).unwrap();

        let loaded = repository.get(task.id).unwrap().unwrap();
        assert_eq!(loaded.title, "提交订单");
        assert_eq!(loaded.status, TaskStatus::Preparing);
    }

    #[test]
    fn lists_tasks_in_scheduled_order() {
        let repository = SqliteTaskRepository::open_in_memory().unwrap();
        let first = sample_task();
        let mut second = sample_task();
        second.scheduled_at_utc = first.scheduled_at_utc + Duration::minutes(5);
        repository.save(&second).unwrap();
        repository.save(&first).unwrap();

        let tasks = repository.list().unwrap();
        assert_eq!(tasks[0].id, first.id);
        assert_eq!(tasks[1].id, second.id);
    }

    #[test]
    fn saves_task_and_execution_log_atomically() {
        let repository = SqliteTaskRepository::open_in_memory().unwrap();
        let mut task = sample_task();
        task.transition(TaskStatus::Preparing).unwrap();
        let result = ExecutionResult {
            outcome: ExecutionOutcome::Failed,
            scheduled_at: task.scheduled_at_utc,
            dispatched_at: None,
            observed_click_at: None,
            dispatch_delay_ms: None,
            observed_delay_ms: None,
            final_url: Some(task.url.clone()),
            message: "浏览器连接失败".into(),
            error_code: Some("browser_disconnected".into()),
            screenshot_path: None,
        };
        task.finish(result.clone()).unwrap();
        let log = ExecutionLog::from_result(task.id, &result);

        repository.save_task_and_log(&task, &log).unwrap();
        let logs = repository.list_execution_logs(task.id).unwrap();
        assert_eq!(logs, vec![log]);
    }
}
