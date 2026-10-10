use std::sync::Arc;

use crate::command::{CommandContext, CommandSource};
use crate::permissions::{AllPermissionsChecker, PermissionChecker};
use crate::player::Player;

use super::MockPlayer;

#[must_use]
pub fn console() -> CommandSource {
    CommandSource::console(Arc::new(AllPermissionsChecker))
}

#[must_use]
pub fn console_with(permissions: Arc<dyn PermissionChecker>) -> CommandSource {
    CommandSource::console(permissions)
}

#[must_use]
pub fn player_source(player: &Arc<MockPlayer>) -> CommandSource {
    CommandSource::Player(Arc::clone(player) as Arc<dyn Player>)
}

#[must_use]
pub fn command_context(source: CommandSource, label: &str, args: &str) -> CommandContext {
    CommandContext::new(source, label, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::DefaultPermissionChecker;
    use crate::types::Component;

    #[test]
    fn sources_route_replies_and_permissions() {
        let steve = Arc::new(MockPlayer::new(1, "Steve").with_permission("hub.use"));
        let source = player_source(&steve);
        source.send_message(Component::text("hi"));
        assert_eq!(steve.sent_text(), "hi");
        assert!(source.has_permission("hub.use"));
        assert_eq!(source.name(), "Steve");

        assert!(console().has_permission("anything"));
        assert!(!console_with(Arc::new(DefaultPermissionChecker)).has_permission("anything"));

        let ctx = command_context(console(), "hub", "a b");
        assert_eq!(ctx.args, ["a", "b"]);
    }
}
