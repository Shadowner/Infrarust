pub mod actor_pair;
pub mod builder;
mod disconnect;
mod health;
mod server_tracking;
pub mod types;

use std::{
    collections::HashMap,
    sync::Arc,
};
use tokio::{
    sync::{OnceCell, RwLock},
    task::JoinHandle,
};
use tracing::debug;
use infrarust_config::LogType;

use crate::server::manager::Manager;

pub use actor_pair::ActorPair;
pub use builder::ActorPairBuilder;
pub use types::{ActorStorage, SupervisorMessage, TaskInfo, TaskStats};

static GLOBAL_SUPERVISOR: OnceCell<Arc<ActorSupervisor>> = OnceCell::const_new();

#[derive(Debug)]
pub struct ActorSupervisor {
    pub(crate) actors: RwLock<ActorStorage>,
    pub(crate) tasks: RwLock<HashMap<String, Vec<JoinHandle<()>>>>,
    pub(crate) server_manager: Option<Arc<Manager>>,
}

impl Default for ActorSupervisor {
    fn default() -> Self {
        Self::new(None)
    }
}

impl ActorSupervisor {
    pub fn global() -> Arc<ActorSupervisor> {
        match GLOBAL_SUPERVISOR.get() {
            Some(supervisor) => supervisor.clone(),
            None => {
                debug!(
                    log_type = LogType::Supervisor.as_str(),
                    "Warning: Using temporary supervisor instance - global was not initialized"
                );
                Arc::new(ActorSupervisor::new(None))
            }
        }
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
        }
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
}
