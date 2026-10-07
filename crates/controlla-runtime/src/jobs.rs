use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const DEFAULT_JOB_DEADLINE_MS: u64 = 60_000;
const MAX_JOB_DEADLINE_MS: u64 = 60_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Accepted,
    Running,
    Completed,
    Failed,
    Cancelled,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    NotSent,
    Sent,
    Unknown,
}

impl Delivery {
    fn parse(value: &str) -> Result<Self, rusqlite::Error> {
        match value {
            "not_sent" => Ok(Self::NotSent),
            "sent" => Ok(Self::Sent),
            "unknown" => Ok(Self::Unknown),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

impl JobStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Unknown => "unknown",
        }
    }
    fn parse(value: &str) -> Result<Self, JournalError> {
        match value {
            "accepted" => Ok(Self::Accepted),
            "running" => Ok(Self::Running),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "unknown" => Ok(Self::Unknown),
            _ => Err(JournalError::InvalidStatus(value.into())),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Operation {
    pub id: String,
    pub status: JobStatus,
    pub revision: u64,
    pub result: Option<Value>,
    pub dispatch_correlation: Option<String>,
    pub deadline_at_ms: u64,
    pub delivery: Delivery,
    pub dispatch_count: u64,
    pub deadline_error: Option<String>,
}

pub struct DispatchClaim {
    pub operation: Operation,
    /// Only the caller that receives true may send the correlated remote request.
    pub acquired: bool,
}

pub struct Admission {
    pub operation: Operation,
    pub replayed: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum JournalError {
    #[error("journal database error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("journal serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("random operation ID generation failed")]
    Random,
    #[error("idempotency key conflicts with a different request")]
    IdempotencyConflict,
    #[error("principal, session, and idempotency key must be non-empty")]
    InvalidIdentity,
    #[error("job deadline must be between 1 and 60000 milliseconds")]
    InvalidDeadline,
    #[error("operation is not in a state that allows {0}")]
    InvalidTransition(&'static str),
    #[error("invalid stored job status: {0}")]
    InvalidStatus(String),
}

#[derive(Clone)]
pub struct Journal(Arc<Mutex<Connection>>);

impl Journal {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, JournalError> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS operations (
                id TEXT PRIMARY KEY, principal TEXT NOT NULL, session TEXT NOT NULL,
                idem_key TEXT NOT NULL, request_digest TEXT NOT NULL, status TEXT NOT NULL,
                revision INTEGER NOT NULL, result TEXT, dispatch_correlation TEXT,
                deadline_at_ms INTEGER NOT NULL, deadline_error TEXT, delivery TEXT NOT NULL,
                UNIQUE(principal, session, idem_key)
            );",
        )?;
        let columns = {
            let mut statement = connection.prepare("PRAGMA table_info(operations)")?;
            statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()?
        };
        if !columns.iter().any(|column| column == "dispatch_count") {
            connection.execute(
                "ALTER TABLE operations ADD COLUMN dispatch_count INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        Ok(Self(Arc::new(Mutex::new(connection))))
    }

    pub fn admit(
        &self,
        principal: &str,
        session: &str,
        key: &str,
        request: &Value,
    ) -> Result<Admission, JournalError> {
        self.admit_with_deadline(principal, session, key, request, DEFAULT_JOB_DEADLINE_MS)
    }

    pub fn admit_with_deadline(
        &self,
        principal: &str,
        session: &str,
        key: &str,
        request: &Value,
        deadline_ms: u64,
    ) -> Result<Admission, JournalError> {
        if principal.is_empty() || session.is_empty() || key.is_empty() {
            return Err(JournalError::InvalidIdentity);
        }
        if deadline_ms == 0 || deadline_ms > MAX_JOB_DEADLINE_MS {
            return Err(JournalError::InvalidDeadline);
        }
        let digest = hex(&Sha256::digest(serde_json::to_vec(
            &serde_json::json!({"request":request,"deadline_ms":deadline_ms}),
        )?));
        let deadline_at_ms = now_ms().saturating_add(deadline_ms);
        let mut connection = self.0.lock().expect("journal mutex poisoned");
        let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(String, String)> = tx.query_row(
            "SELECT id, request_digest FROM operations WHERE principal=?1 AND session=?2 AND idem_key=?3",
            params![principal, session, key], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        if let Some((id, prior_digest)) = existing {
            if prior_digest != digest {
                return Err(JournalError::IdempotencyConflict);
            }
            let operation = get_tx(&tx, principal, session, &id)?.expect("operation exists");
            tx.commit()?;
            return Ok(Admission {
                operation,
                replayed: true,
            });
        }
        let mut random = [0u8; 16];
        getrandom::fill(&mut random).map_err(|_| JournalError::Random)?;
        let id = hex(&random);
        tx.execute("INSERT INTO operations (id, principal, session, idem_key, request_digest, status, revision, deadline_at_ms, delivery) VALUES (?1, ?2, ?3, ?4, ?5, 'accepted', 1, ?6, 'not_sent')", params![id, principal, session, key, digest, deadline_at_ms as i64])?;
        let operation = get_tx(&tx, principal, session, &id)?.expect("inserted operation exists");
        tx.commit()?;
        Ok(Admission {
            operation,
            replayed: false,
        })
    }

    pub fn record_dispatch(
        &self,
        principal: &str,
        session: &str,
        id: &str,
        correlation: &str,
    ) -> Result<DispatchClaim, JournalError> {
        let connection = self.0.lock().expect("journal mutex poisoned");
        expire_one(&connection, principal, session, id)?;
        let changed = connection.execute("UPDATE operations SET status='running', dispatch_correlation=?4, delivery='unknown', dispatch_count=dispatch_count+1, revision=revision+1 WHERE id=?1 AND principal=?2 AND session=?3 AND (status='accepted' OR (status='running' AND delivery IN ('not_sent','sent'))) AND deadline_at_ms>?5", params![id, principal, session, correlation, now_ms() as i64])?;
        if changed == 0 {
            expire_one(&connection, principal, session, id)?;
            let current = get_conn(&connection, principal, session, id)?;
            if let Some(op) = current
                && op.status == JobStatus::Running
                && op.dispatch_correlation.as_deref() == Some(correlation)
            {
                return Ok(DispatchClaim {
                    operation: op,
                    acquired: false,
                });
            }
            return Err(JournalError::InvalidTransition("dispatch"));
        }
        Ok(DispatchClaim {
            operation: get_conn(&connection, principal, session, id)?
                .ok_or(JournalError::InvalidTransition("dispatch"))?,
            acquired: true,
        })
    }

    /// Atomically appoint one runner for an admitted job without claiming external delivery.
    pub fn start(&self, principal: &str, session: &str, id: &str) -> Result<bool, JournalError> {
        let connection = self.0.lock().expect("journal mutex poisoned");
        expire_one(&connection, principal, session, id)?;
        let changed = connection.execute(
            "UPDATE operations SET status='running', revision=revision+1 WHERE id=?1 AND principal=?2 AND session=?3 AND status='accepted' AND deadline_at_ms>?4",
            params![id, principal, session, now_ms() as i64],
        )?;
        Ok(changed == 1)
    }

    pub fn complete(
        &self,
        principal: &str,
        session: &str,
        id: &str,
        result: Value,
    ) -> Result<Operation, JournalError> {
        self.finish(principal, session, id, JobStatus::Completed, Some(result))
    }

    /// Persist a resumable receipt after a completed step while leaving the operation running.
    pub fn checkpoint(
        &self,
        principal: &str,
        session: &str,
        id: &str,
        result: Value,
    ) -> Result<Operation, JournalError> {
        let encoded = serde_json::to_string(&result)?;
        let connection = self.0.lock().expect("journal mutex poisoned");
        expire_one(&connection, principal, session, id)?;
        let changed = connection.execute(
            "UPDATE operations SET result=?4, revision=revision+1 WHERE id=?1 AND principal=?2 AND session=?3 AND status='running' AND deadline_at_ms>?5",
            params![id, principal, session, encoded, now_ms() as i64],
        )?;
        if changed == 0 {
            expire_one(&connection, principal, session, id)?;
            return Err(JournalError::InvalidTransition("checkpoint"));
        }
        get_conn(&connection, principal, session, id)?
            .ok_or(JournalError::InvalidTransition("checkpoint"))
    }

    /// Preserve an operation receipt when dispatch outcome cannot be established.
    pub fn mark_unknown(
        &self,
        principal: &str,
        session: &str,
        id: &str,
        result: Value,
    ) -> Result<Operation, JournalError> {
        let encoded = serde_json::to_string(&result)?;
        let connection = self.0.lock().expect("journal mutex poisoned");
        let changed = connection.execute(
            "UPDATE operations SET status='unknown', result=?4, revision=revision+1 WHERE id=?1 AND principal=?2 AND session=?3 AND status='running'",
            params![id, principal, session, encoded],
        )?;
        if changed == 0 {
            return Err(JournalError::InvalidTransition("unknown outcome"));
        }
        get_conn(&connection, principal, session, id)?
            .ok_or(JournalError::InvalidTransition("unknown outcome"))
    }

    pub fn fail(
        &self,
        principal: &str,
        session: &str,
        id: &str,
        result: Value,
    ) -> Result<Operation, JournalError> {
        self.finish(principal, session, id, JobStatus::Failed, Some(result))
    }

    fn finish(
        &self,
        principal: &str,
        session: &str,
        id: &str,
        status: JobStatus,
        result: Option<Value>,
    ) -> Result<Operation, JournalError> {
        let encoded = result.map(|v| serde_json::to_string(&v)).transpose()?;
        let connection = self.0.lock().expect("journal mutex poisoned");
        expire_one(&connection, principal, session, id)?;
        let changed = connection.execute("UPDATE operations SET status=?4, result=?5, revision=revision+1 WHERE id=?1 AND principal=?2 AND session=?3 AND status='running' AND deadline_at_ms>?6 AND (?4!='completed' OR delivery='sent')", params![id, principal, session, status.as_str(), encoded, now_ms() as i64])?;
        if changed == 0 {
            expire_one(&connection, principal, session, id)?;
            return Err(JournalError::InvalidTransition("completion"));
        }
        get_conn(&connection, principal, session, id)?
            .ok_or(JournalError::InvalidTransition("completion"))
    }

    /// Call only when the transport confirms the correlated request was sent.
    pub fn acknowledge_dispatch(
        &self,
        principal: &str,
        session: &str,
        id: &str,
        correlation: &str,
    ) -> Result<bool, JournalError> {
        let connection = self.0.lock().expect("journal mutex poisoned");
        expire_one(&connection, principal, session, id)?;
        let changed = connection.execute("UPDATE operations SET delivery='sent', revision=revision+1 WHERE id=?1 AND principal=?2 AND session=?3 AND status='running' AND dispatch_correlation=?4 AND delivery='unknown' AND deadline_at_ms>?5", params![id, principal, session, correlation, now_ms() as i64])?;
        Ok(changed == 1)
    }

    /// Use only when the adapter can prove transport send never began.
    pub fn fail_before_send(
        &self,
        principal: &str,
        session: &str,
        id: &str,
        error: Value,
    ) -> Result<Operation, JournalError> {
        let encoded = serde_json::to_string(&error)?;
        let connection = self.0.lock().expect("journal mutex poisoned");
        expire_one(&connection, principal, session, id)?;
        let changed = connection.execute("UPDATE operations SET status='failed', result=?4, delivery='not_sent', revision=revision+1 WHERE id=?1 AND principal=?2 AND session=?3 AND status IN ('accepted','running') AND delivery IN ('not_sent','unknown') AND deadline_at_ms>?5", params![id, principal, session, encoded, now_ms() as i64])?;
        if changed == 0 {
            return Err(JournalError::InvalidTransition(
                "confirmed pre-send failure",
            ));
        }
        get_conn(&connection, principal, session, id)?.ok_or(JournalError::InvalidTransition(
            "confirmed pre-send failure",
        ))
    }

    /// Cancellation prevents dispatch only. Once a remote request is in flight, its outcome must be reconciled.
    pub fn cancel(&self, principal: &str, session: &str, id: &str) -> Result<bool, JournalError> {
        let connection = self.0.lock().expect("journal mutex poisoned");
        expire_one(&connection, principal, session, id)?;
        let changed = connection.execute("UPDATE operations SET status='cancelled', revision=revision+1 WHERE id=?1 AND principal=?2 AND session=?3 AND status='accepted' AND deadline_at_ms>?4", params![id, principal, session, now_ms() as i64])?;
        if changed == 0 {
            expire_one(&connection, principal, session, id)?;
        }
        Ok(changed == 1)
    }

    /// Call only after this process has recovered a database from a prior runtime; dispatched effects stay uncertain.
    pub fn recover_uncertain(&self, principal: &str, session: &str) -> Result<usize, JournalError> {
        let connection = self.0.lock().expect("journal mutex poisoned");
        let mut count = connection.execute(
            "UPDATE operations SET status='unknown', deadline_error=CASE WHEN deadline_at_ms<=?3 THEN 'deadline_exceeded' ELSE deadline_error END, revision=revision+1 WHERE principal=?1 AND session=?2 AND status='running'",
            params![principal, session, now_ms() as i64],
        )?;
        count += connection.execute("UPDATE operations SET status='failed', deadline_error='deadline_exceeded', revision=revision+1 WHERE principal=?1 AND session=?2 AND status='accepted' AND deadline_at_ms<=?3", params![principal, session, now_ms() as i64])?;
        Ok(count)
    }

    /// Reconcile durable records immediately after opening a journal from a prior process.
    /// Running jobs become unknown and keep their latest checkpoint; accepted jobs are safe to
    /// fail because the runner has not yet claimed any browser dispatch.
    pub fn recover_after_restart(&self) -> Result<usize, JournalError> {
        let connection = self.0.lock().expect("journal mutex poisoned");
        Ok(connection.execute(
            "UPDATE operations SET status=CASE status WHEN 'running' THEN 'unknown' ELSE 'failed' END, deadline_error='process_restarted', revision=revision+1 WHERE status IN ('accepted','running')",
            [],
        )?)
    }

    pub fn reconcile(
        &self,
        principal: &str,
        session: &str,
        id: &str,
    ) -> Result<Option<Operation>, JournalError> {
        self.get(principal, session, id)
    }

    pub fn get(
        &self,
        principal: &str,
        session: &str,
        id: &str,
    ) -> Result<Option<Operation>, JournalError> {
        let connection = self.0.lock().expect("journal mutex poisoned");
        expire_one(&connection, principal, session, id)?;
        get_conn(&connection, principal, session, id)
    }

    /// Wait only bounds this caller's response time. The accepted job remains in SQLite after timeout/disconnect.
    pub async fn wait(
        &self,
        principal: &str,
        session: &str,
        id: &str,
        after_revision: u64,
        max_wait_ms: u64,
    ) -> Result<Operation, JournalError> {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(max_wait_ms);
        loop {
            let op = self
                .get(principal, session, id)?
                .ok_or(JournalError::InvalidTransition("wait"))?;
            if op.revision > after_revision
                || matches!(
                    op.status,
                    JobStatus::Completed
                        | JobStatus::Failed
                        | JobStatus::Cancelled
                        | JobStatus::Unknown
                )
                || tokio::time::Instant::now() >= deadline
            {
                return Ok(op);
            }
            tokio::time::sleep(
                Duration::from_millis(10)
                    .min(deadline.saturating_duration_since(tokio::time::Instant::now())),
            )
            .await;
        }
    }
}

fn get_conn(
    connection: &Connection,
    principal: &str,
    session: &str,
    id: &str,
) -> Result<Option<Operation>, JournalError> {
    Ok(connection
        .query_row(
            "SELECT id,status,revision,result,dispatch_correlation,deadline_at_ms,deadline_error,delivery,dispatch_count FROM operations WHERE id=?1 AND principal=?2 AND session=?3",
            params![id, principal, session],
            row_operation,
        )
        .optional()?)
}
fn get_tx(
    tx: &rusqlite::Transaction<'_>,
    principal: &str,
    session: &str,
    id: &str,
) -> Result<Option<Operation>, JournalError> {
    Ok(tx
        .query_row(
            "SELECT id,status,revision,result,dispatch_correlation,deadline_at_ms,deadline_error,delivery,dispatch_count FROM operations WHERE id=?1 AND principal=?2 AND session=?3",
            params![id, principal, session],
            row_operation,
        )
        .optional()?)
}
fn row_operation(row: &rusqlite::Row<'_>) -> Result<Operation, rusqlite::Error> {
    let result: Option<String> = row.get(3)?;
    let status =
        JobStatus::parse(&row.get::<_, String>(1)?).map_err(|_| rusqlite::Error::InvalidQuery)?;
    Ok(Operation {
        id: row.get(0)?,
        status,
        revision: row.get::<_, i64>(2)? as u64,
        result: result
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        dispatch_correlation: row.get(4)?,
        deadline_at_ms: row.get::<_, i64>(5)? as u64,
        delivery: Delivery::parse(&row.get::<_, String>(7)?)?,
        dispatch_count: row.get::<_, i64>(8)? as u64,
        deadline_error: row.get(6)?,
    })
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn expire_one(
    connection: &Connection,
    principal: &str,
    session: &str,
    id: &str,
) -> Result<(), JournalError> {
    connection.execute("UPDATE operations SET status=CASE status WHEN 'accepted' THEN 'failed' ELSE 'unknown' END, deadline_error='deadline_exceeded', delivery=CASE status WHEN 'accepted' THEN 'not_sent' ELSE delivery END, revision=revision+1 WHERE id=?1 AND principal=?2 AND session=?3 AND status IN ('accepted','running') AND deadline_at_ms<=?4", params![id, principal, session, now_ms() as i64])?;
    Ok(())
}
