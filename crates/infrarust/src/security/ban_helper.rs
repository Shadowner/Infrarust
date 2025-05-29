use crate::FilterError;
use crate::{FilterRegistry, security::BanSystemAdapter, with_filter_or};
use infrarust_config::LogType;
use std::sync::Arc;
use tracing::debug;

pub struct BanHelper;

impl BanHelper {
    pub async fn is_username_banned(
        registry: &Arc<FilterRegistry>,
        username: &str,
    ) -> Option<String> {
        debug!(log_type = LogType::BanSystem.as_str(), "Checking if username '{}' is banned", username);

        let is_banned = matches!(
            with_filter_or!(
                registry,
                "global_ban_system",
                BanSystemAdapter,
                async |filter: &BanSystemAdapter| { filter.is_username_banned(username).await },
                false
            ),
            Ok(true)
        );

        if is_banned {
            if let Ok(reason) = with_filter_or!(
                registry,
                "global_ban_system",
                BanSystemAdapter,
                async |filter: &BanSystemAdapter| {
                    filter.get_ban_reason_for_username(username).await
                },
                None
            ) {
                return reason;
            }
        }
        None
    }
}
