use crate::FilterError;
use crate::{FilterRegistry, security::BanSystemAdapter, with_filter_or};
use std::sync::Arc;

pub struct BanHelper;

impl BanHelper {
    pub async fn is_username_banned(
        registry: &Arc<FilterRegistry>,
        username: &str,
    ) -> Option<String> {
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
