use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;

use crate::auth::authority::AuthService;
use crate::auth::config::AuthConfig;
use crate::engine::chat_engine::ChatEngine;

use super::atproto::AtprotoOAuth;

/// Shared application state available to all HTTP/WebSocket handlers.
pub struct AppState {
    pub engine: Arc<ChatEngine>,
    pub db: SqlitePool,
    pub auth_config: AuthConfig,
    pub auth: AuthService,
    pub atproto: AtprotoOAuth,
    pub secret_vault: Arc<crate::secrets::SecretVault>,
    pub egress: Arc<crate::egress::EgressServices>,
    pub max_file_size: u64,
    pub max_media_per_user: u64,
    pub max_media_total: u64,
    pub upload_admission: Arc<tokio::sync::Semaphore>,
    pub upload_idle_timeout: std::time::Duration,
    pub upload_total_timeout: std::time::Duration,
    pub max_message_length: usize,
    pub admin_user_ids: Arc<[String]>,
    pub health: Arc<HealthState>,
    pub shutdown: CancellationToken,
    pub media_dir: std::path::PathBuf,
}

#[derive(Default)]
pub struct HealthState {
    accepting_requests: AtomicBool,
    web_listener_bound: AtomicBool,
    irc_listener_bound: AtomicBool,
    dependency_probe_active: AtomicBool,
    metrics_collection_active: AtomicBool,
    metrics_cache: Mutex<Option<MetricsCache>>,
}

impl HealthState {
    /// Enable or revoke service admission after all supervised tasks exist.
    pub fn set_ready(&self, ready: bool) {
        self.accepting_requests.store(ready, Ordering::Release);
    }

    pub fn set_web_listener_bound(&self, bound: bool) {
        self.web_listener_bound.store(bound, Ordering::Release);
    }

    pub fn set_irc_listener_bound(&self, bound: bool) {
        self.irc_listener_bound.store(bound, Ordering::Release);
    }

    pub fn is_ready(&self) -> bool {
        let snapshot = self.snapshot();
        snapshot.accepting_requests && snapshot.web_listener_bound && snapshot.irc_listener_bound
    }

    pub fn snapshot(&self) -> HealthSnapshot {
        HealthSnapshot {
            accepting_requests: self.accepting_requests.load(Ordering::Acquire),
            web_listener_bound: self.web_listener_bound.load(Ordering::Acquire),
            irc_listener_bound: self.irc_listener_bound.load(Ordering::Acquire),
        }
    }

    /// Admit one dependency probe at a time so health polling cannot build a
    /// second write queue beside normal application admission.
    pub fn begin_dependency_probe(self: &Arc<Self>) -> Option<DependencyProbe> {
        self.dependency_probe_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| DependencyProbe {
                health: self.clone(),
            })
    }

    pub fn begin_metrics_collection(self: &Arc<Self>) -> Option<MetricsCollection> {
        self.metrics_collection_active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()
            .map(|_| MetricsCollection {
                health: self.clone(),
            })
    }

    pub fn cached_metrics(&self, maximum_age: Duration) -> Option<(u16, String)> {
        self.metrics_cache
            .lock()
            .expect("metrics cache poisoned")
            .as_ref()
            .filter(|cached| cached.created_at.elapsed() <= maximum_age)
            .map(|cached| (cached.status, cached.body.clone()))
    }

    pub fn store_metrics(&self, status: u16, body: String) {
        *self.metrics_cache.lock().expect("metrics cache poisoned") = Some(MetricsCache {
            created_at: Instant::now(),
            status,
            body,
        });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HealthSnapshot {
    pub accepting_requests: bool,
    pub web_listener_bound: bool,
    pub irc_listener_bound: bool,
}

struct MetricsCache {
    created_at: Instant,
    status: u16,
    body: String,
}

pub struct DependencyProbe {
    health: Arc<HealthState>,
}

impl Drop for DependencyProbe {
    fn drop(&mut self) {
        self.health
            .dependency_probe_active
            .store(false, Ordering::Release);
    }
}

pub struct MetricsCollection {
    health: Arc<HealthState>,
}

impl Drop for MetricsCollection {
    fn drop(&mut self) {
        self.health
            .metrics_collection_active
            .store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::HealthState;

    #[test]
    fn readiness_is_false_until_explicitly_enabled_and_can_be_revoked() {
        let health = Arc::new(HealthState::default());
        assert!(!health.is_ready());
        health.set_ready(true);
        assert!(!health.is_ready());
        health.set_web_listener_bound(true);
        assert!(!health.is_ready());
        health.set_irc_listener_bound(true);
        assert!(health.is_ready());
        health.set_ready(false);
        assert!(!health.is_ready());
    }

    #[test]
    fn dependency_probe_admission_is_bounded_and_released_by_drop() {
        let health = Arc::new(HealthState::default());
        let probe = health.begin_dependency_probe().unwrap();
        assert!(health.begin_dependency_probe().is_none());
        drop(probe);
        assert!(health.begin_dependency_probe().is_some());
    }

    #[test]
    fn metrics_collection_admission_is_bounded_and_cache_is_age_limited() {
        let health = Arc::new(HealthState::default());
        let collection = health.begin_metrics_collection().unwrap();
        assert!(health.begin_metrics_collection().is_none());
        assert!(
            health
                .cached_metrics(std::time::Duration::from_secs(1))
                .is_none()
        );
        health.store_metrics(200, "snapshot".into());
        assert_eq!(
            health.cached_metrics(std::time::Duration::from_secs(1)),
            Some((200, "snapshot".into()))
        );
        drop(collection);
        assert!(health.begin_metrics_collection().is_some());
    }
}
