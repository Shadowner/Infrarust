pub(crate) mod chat;
pub(crate) mod engine;
pub(crate) mod handler_chain;
pub(crate) mod keepalive;
pub(crate) mod login;
pub(crate) mod registry;
pub(crate) mod registry_cache;
pub(crate) mod registry_nbt;
pub(crate) mod session;
pub(crate) mod spawn;
pub(crate) mod virtual_session;

#[cfg(test)]
pub(crate) mod test_helpers;

pub(crate) const LIMBO_SWITCH_TARGET: &str = "$limbo";
