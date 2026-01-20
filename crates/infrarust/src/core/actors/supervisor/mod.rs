pub mod actor_pair;
pub mod builder;
mod disconnect;
mod health;
mod server_tracking;
pub mod types;

use infrarust_config::LogType;
use std::{collections::HashMap, sync::Arc};
use tokio::{
    sync::{OnceCell, RwLock},
    task::JoinHandle,
};
use tracing::debug;

use crate::core::config::service::ConfigurationService;
use crate::server::manager::Manager;

pub use actor_pair::ActorPair;
pub use builder::ActorPairBuilder;
pub use types::{ActorStorage, ActorSupervisorMetrics, SupervisorMessage, TaskInfo, TaskStats};

static GLOBAL_SUPERVISOR: OnceCell<Arc<ActorSupervisor>> = OnceCell::const_new();

#[derive(Debug)]
pub struct ActorSupervisor {
    pub(crate) actors: RwLock<ActorStorage>,
    pub(crate) tasks: RwLock<HashMap<String, Vec<JoinHandle<()>>>>,
    pub(crate) server_manager: Option<Arc<Manager>>,
    pub(crate) configuration_service: RwLock<Option<Arc<ConfigurationService>>>,
}

impl Default for ActorSupervisor {
    fn default() -> Self {
        Self::new(None)
    }
}

impl ActorSupervisor {
    pub fn global() -> Arc<ActorSupervisor> {
        GLOBAL_SUPERVISOR
            .get()
            .cloned()
            .expect("ActorSupervisor::global() called before initialization. Ensure initialize_global() is called during startup.")
    }

    pub async fn get_task_statistics(&self) -> HashMap<String, TaskStats> {
        let tasks = self.tasks.read().await;
        let actors = self.actors.read().await;
        let mut stats = HashMap::new();

        for (config_id, handles) in tasks.iter() {
            let actor_count = actors.get(config_id).map_or(0, |pairs| {
                pairs
                    .iter()
                    .filter(|p| !p.shutdown.load(std::sync::atomic::Ordering::SeqCst))
                    .count()
            });

            let running_count = handles.iter().filter(|h| !h.is_finished()).count();

            let completed_count = handles.iter().filter(|h| h.is_finished()).count();

            let handles_info: Vec<TaskInfo> = handles
                .iter()
                .enumerate()
                .map(|(idx, handle)| TaskInfo {
                    id: idx,
                    is_finished: handle.is_finished(),
                    is_aborted: handle.is_finished(),
                })
                .collect();

            stats.insert(
                config_id.clone(),
                TaskStats {
                    config_id: config_id.clone(),
                    active_actor_count: actor_count,
                    task_count: handles.len(),
                    running_count,
                    completed_count,
                    orphaned_count: if actor_count == 0 { handles.len() } else { 0 },
                    task_handles: handles_info,
                },
            );
        }

        stats
    }

    pub fn initialize_global(
        server_manager: Option<Arc<Manager>>,
    ) -> Result<(), tokio::sync::SetError<Arc<ActorSupervisor>>> {
        debug!(
            log_type = LogType::Supervisor.as_str(),
            "Initializing global supervisor instance"
        );
        GLOBAL_SUPERVISOR.set(Arc::new(ActorSupervisor::new(server_manager)))
    }

    pub fn new(server_manager: Option<Arc<Manager>>) -> Self {
        Self {
            actors: RwLock::new(HashMap::new()),
            tasks: RwLock::new(HashMap::new()),
            server_manager,
            configuration_service: RwLock::new(None),
        }
    }

    pub async fn set_configuration_service(&self, config_service: Arc<ConfigurationService>) {
        let mut lock = self.configuration_service.write().await;
        *lock = Some(config_service);
    }

    pub fn builder(&self) -> ActorPairBuilder<'_> {
        ActorPairBuilder::new(self)
    }

    pub async fn find_actor_pairs_by_session_id(
        &self,
        session_id: uuid::Uuid,
    ) -> Option<Vec<Arc<RwLock<ActorPair>>>> {
        let actors = self.actors.read().await;
        let mut result = Vec::new();

        for pairs in actors.values() {
            for pair in pairs {
                if pair.session_id == session_id {
                    let pair_clone = Arc::new(RwLock::new(pair.clone()));
                    result.push(pair_clone);
                }
            }
        }

        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }

    pub async fn get_all_actors(&self) -> HashMap<String, Vec<ActorPair>> {
        let actors = self.actors.read().await;
        let mut result = HashMap::new();

        for (config_id, pairs) in actors.iter() {
            // Only include pairs that aren't shut down
            let active_pairs: Vec<ActorPair> = pairs
                .iter()
                .filter(|pair| !pair.shutdown.load(std::sync::atomic::Ordering::SeqCst))
                .cloned()
                .collect();

            if !active_pairs.is_empty() {
                result.insert(config_id.clone(), active_pairs);
            }
        }

        result
    }

    pub fn get_memory_metrics(&self) -> Option<ActorSupervisorMetrics> {
        let actors = self.actors.try_read().ok()?;
        let tasks = self.tasks.try_read().ok()?;

        let mut actors_by_config = HashMap::new();
        let mut total_actors = 0;

        for (config_id, pairs) in actors.iter() {
            let active_count = pairs
                .iter()
                .filter(|p| !p.shutdown.load(std::sync::atomic::Ordering::SeqCst))
                .count();
            actors_by_config.insert(config_id.clone(), active_count);
            total_actors += active_count;
        }

        let mut tasks_by_config = HashMap::new();
        let mut total_tasks = 0;
        let mut orphaned_tasks = 0;

        for (config_id, handles) in tasks.iter() {
            let running_count = handles.iter().filter(|h| !h.is_finished()).count();
            tasks_by_config.insert(config_id.clone(), running_count);
            total_tasks += running_count;

            // Check for orphaned tasks (tasks without actors)
            let actor_count = actors.get(config_id).map_or(0, |pairs| {
                pairs
                    .iter()
                    .filter(|p| !p.shutdown.load(std::sync::atomic::Ordering::SeqCst))
                    .count()
            });
            if actor_count == 0 && running_count > 0 {
                orphaned_tasks += running_count;
            }
        }

        Some(ActorSupervisorMetrics {
            total_actor_entries: total_actors,
            actors_by_config,
            total_task_entries: total_tasks,
            tasks_by_config,
            orphaned_tasks,
        })
    }
}
