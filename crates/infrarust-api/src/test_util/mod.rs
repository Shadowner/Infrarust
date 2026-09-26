#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod ban;
mod ban_provider;
mod command;
mod config;
mod gate;
mod load_balancer;
mod permissions;
mod player;
mod plugin_registry;
mod registry;
mod server_manager;

pub use ban::MockBanService;
pub use ban_provider::MemoryBanProvider;
pub use command::{command_context, console, console_with, player_source};
pub use config::MockConfigService;
pub use gate::Gate;
pub use load_balancer::MockLoadBalancerService;
pub use permissions::MockPermissionChecker;
pub use player::MockPlayer;
pub use plugin_registry::MockPluginRegistry;
pub use registry::MockPlayerRegistry;
pub use server_manager::MockServerManager;

pub use crate::limbo::test_util::RecordingLimboSession;

fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
fn block_on<F: Future>(future: F) -> F::Output {
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    let mut future = std::pin::pin!(future);
    loop {
        if let std::task::Poll::Ready(out) = future.as_mut().poll(&mut cx) {
            return out;
        }
    }
}
