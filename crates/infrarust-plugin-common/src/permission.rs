use std::borrow::Cow;

#[must_use]
pub fn normalize_node(node: &str) -> Cow<'_, str> {
    let node = node.trim();
    if node.chars().any(char::is_uppercase) {
        Cow::Owned(node.to_lowercase())
    } else {
        Cow::Borrowed(node)
    }
}
