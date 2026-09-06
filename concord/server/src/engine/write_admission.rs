use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[cfg(not(test))]
const WRITE_ADMISSION_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const WRITE_ADMISSION_TIMEOUT: Duration = Duration::from_millis(200);
const MAX_PENDING_WRITES: usize = 128;
const MAX_ACTIVE_WRITES: usize = 32;

#[derive(Debug)]
pub enum WriteAdmissionError {
    Unavailable,
    Database(sqlx::Error),
}

impl fmt::Display for WriteAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("write dependency unavailable"),
            Self::Database(error) => write!(formatter, "write transaction failed: {error}"),
        }
    }
}

impl std::error::Error for WriteAdmissionError {}

/// Shared process-wide admission and deadline for durable user-triggered writes.
#[derive(Clone)]
pub struct WriteAdmission {
    pool: SqlitePool,
    active: Arc<Semaphore>,
    pending: Arc<Semaphore>,
}

impl WriteAdmission {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            active: Arc::new(Semaphore::new(MAX_ACTIVE_WRITES)),
            pending: Arc::new(Semaphore::new(MAX_PENDING_WRITES)),
        }
    }

    pub async fn begin(
        &self,
    ) -> Result<(OwnedSemaphorePermit, Transaction<'static, Sqlite>), WriteAdmissionError> {
        let mut metric =
            crate::runtime_metrics::Timer::start(crate::runtime_metrics::Operation::DatabaseWrite);
        let waiter = self
            .pending
            .clone()
            .try_acquire_owned()
            .map_err(|_| WriteAdmissionError::Unavailable)?;
        let admitted = tokio::time::timeout(WRITE_ADMISSION_TIMEOUT, async {
            let permit = self
                .active
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| WriteAdmissionError::Unavailable)?;
            let transaction = self
                .pool
                .begin_with("BEGIN IMMEDIATE")
                .await
                .map_err(WriteAdmissionError::Database)?;
            drop(waiter);
            Ok((permit, transaction))
        })
        .await
        .map_err(|_| WriteAdmissionError::Unavailable)??;
        metric.succeed();
        Ok(admitted)
    }

    #[cfg(test)]
    pub(crate) async fn hold_active_capacity_for_test(
        &self,
    ) -> Vec<tokio::sync::OwnedSemaphorePermit> {
        let mut permits = Vec::with_capacity(MAX_ACTIVE_WRITES);
        for _ in 0..MAX_ACTIVE_WRITES {
            permits.push(
                self.active
                    .clone()
                    .acquire_owned()
                    .await
                    .expect("write admission semaphore remains open during tests"),
            );
        }
        permits
    }

    #[cfg(test)]
    pub(crate) fn pending_available_permits_for_test(&self) -> usize {
        self.pending.available_permits()
    }

    /// Return a currently valid client operation epoch, rotating it under the
    /// same admitted IMMEDIATE transaction used by domain writes.
    pub async fn current_operation_generation(&self) -> Result<String, WriteAdmissionError> {
        let (_permit, mut transaction) = self.begin().await?;
        let row = sqlx::query(
            "SELECT s.current_generation,g.expires_at \
             FROM operation_generation_state s \
             JOIN operation_generations g ON g.generation=s.current_generation \
             WHERE s.singleton=1",
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(WriteAdmissionError::Database)?;
        let current: String = row.get(0);
        let expires_at: i64 = row.get(1);
        let now: i64 = sqlx::query_scalar("SELECT unixepoch()")
            .fetch_one(&mut *transaction)
            .await
            .map_err(WriteAdmissionError::Database)?;
        let generation = if expires_at > now {
            current
        } else {
            let next: String = sqlx::query_scalar("SELECT lower(hex(randomblob(16)))")
                .fetch_one(&mut *transaction)
                .await
                .map_err(WriteAdmissionError::Database)?;
            sqlx::query(
                "INSERT INTO operation_generations(generation,issued_at,expires_at) \
                 VALUES(?,?,?)",
            )
            .bind(&next)
            .bind(now)
            .bind(now + 604_800)
            .execute(&mut *transaction)
            .await
            .map_err(WriteAdmissionError::Database)?;
            sqlx::query(
                "UPDATE operation_generation_state SET current_generation=? WHERE singleton=1",
            )
            .bind(&next)
            .execute(&mut *transaction)
            .await
            .map_err(WriteAdmissionError::Database)?;
            next
        };
        transaction
            .commit()
            .await
            .map_err(WriteAdmissionError::Database)?;
        Ok(generation)
    }
}
