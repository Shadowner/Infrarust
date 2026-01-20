use std::net::IpAddr;

use infrarust_ban_system::BanEntry;

use crate::{
    Infrarust,
    security::{
        ban_system_adapter::BanSystemAdapter, filter::FilterError, with_filter, with_filter_or,
    },
};

impl Infrarust {
    pub async fn has_ban_filter(&self) -> Result<bool, FilterError> {
        let registry = self.shared.filter_registry();

        with_filter_or!(
            registry,
            "global_ban_system",
            BanSystemAdapter,
            async |_: &BanSystemAdapter| { Ok(true) },
            false
        )
    }

    pub async fn add_ban(&self, ban: BanEntry) -> Result<(), FilterError> {
        let registry = self.shared.filter_registry();

        with_filter!(
            registry,
            "global_ban_system",
            BanSystemAdapter,
            async |filter: &BanSystemAdapter| { filter.add_ban(ban).await }
        )
    }

    pub async fn remove_ban_by_ip(&self, ip: IpAddr) -> Result<bool, FilterError> {
        let registry = self.shared.filter_registry();
        with_filter!(
            registry,
            "global_ban_system",
            BanSystemAdapter,
            async |filter: &BanSystemAdapter| { filter.remove_ban_by_ip(&ip, "system").await }
        )
    }

    pub async fn remove_ban_by_username(&self, username: &str) -> Result<bool, FilterError> {
        let registry = self.shared.filter_registry();

        with_filter!(
            registry,
            "global_ban_system",
            BanSystemAdapter,
            async |filter: &BanSystemAdapter| {
                filter.remove_ban_by_username(username, "system").await
            }
        )
    }

    pub async fn remove_ban_by_uuid(&self, uuid: &str) -> Result<bool, FilterError> {
        let registry = self.shared.filter_registry();

        with_filter!(
            registry,
            "global_ban_system",
            BanSystemAdapter,
            async |filter: &BanSystemAdapter| { filter.remove_ban_by_uuid(uuid, "system").await }
        )
    }

    pub async fn get_all_bans(&self) -> Result<Vec<BanEntry>, FilterError> {
        let registry = self.shared.filter_registry();

        with_filter!(
            registry,
            "global_ban_system",
            BanSystemAdapter,
            async |filter: &BanSystemAdapter| { filter.get_all_bans().await }
        )
    }

    pub async fn clear_expired_bans(&self) -> Result<usize, FilterError> {
        let registry = self.shared.filter_registry();

        with_filter!(
            registry,
            "global_ban_system",
            BanSystemAdapter,
            async |filter: &BanSystemAdapter| { filter.clear_expired_bans().await }
        )
    }

    pub async fn get_ban_file_path(&self) -> Option<String> {
        self.shared
            .config()
            .filters
            .as_ref()
            .and_then(|f| f.ban.file_path.clone())
    }

    pub async fn has_ban_system_adapter(&self) -> Result<bool, FilterError> {
        let registry = self.shared.filter_registry();

        with_filter_or!(
            registry,
            "global_ban_system",
            BanSystemAdapter,
            async |_: &BanSystemAdapter| { Ok(true) },
            false
        )
    }
}
