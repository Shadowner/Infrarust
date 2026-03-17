//! Filter metadata and priority types for ordering.

/// Metadata describing a filter, used for ordering within a chain.
#[derive(Debug, Clone)]
pub struct FilterMetadata {
    /// Unique identifier for this filter.
    pub id: &'static str,
    /// Base priority for ordering.
    pub priority: FilterPriority,
    /// This filter must execute AFTER these filters (by id).
    pub after: Vec<&'static str>,
    /// This filter must execute BEFORE these filters (by id).
    pub before: Vec<&'static str>,
}

/// Priority of a filter within its chain.
///
/// Lower values execute first. When two filters have no dependency
/// relationship, priority is used as a tiebreaker.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FilterPriority {
    /// Executes first (e.g. security filters).
    First = 0,
    /// Executes early.
    Early = 1,
    /// Default priority.
    #[default]
    Normal = 2,
    /// Executes late.
    Late = 3,
    /// Executes last (e.g. logging filters).
    Last = 4,
}

impl FilterMetadata {
    /// Creates metadata with only an id and default priority.
    #[must_use]
    pub fn new(id: &'static str) -> Self {
        Self {
            id,
            priority: FilterPriority::Normal,
            after: Vec::new(),
            before: Vec::new(),
        }
    }
}
