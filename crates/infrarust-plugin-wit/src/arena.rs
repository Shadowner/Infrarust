use std::fmt;

pub const MAX_NODES: usize = 4096;

pub const MAX_DEPTH: usize = 64;

pub const MAX_TEXT_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ArenaError {
    Empty,
    TooManyNodes { count: usize },
    OutOfRange { node: u32, reference: u32 },
    BackwardReference { node: u32, reference: u32 },
    SharedNode { node: u32 },
    Unreferenced { node: u32 },
    TooDeep { depth: usize },
    TooMuchText { bytes: usize },
    InvalidValue { node: u32, reason: String },
}

impl fmt::Display for ArenaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("the component has no root node"),
            Self::TooManyNodes { count } => {
                write!(
                    f,
                    "the component has {count} nodes, at most {MAX_NODES} are allowed"
                )
            }
            Self::OutOfRange { node, reference } => {
                write!(
                    f,
                    "node {node} references node {reference}, which does not exist"
                )
            }
            Self::BackwardReference { node, reference } => write!(
                f,
                "node {node} references node {reference}; references must point to a later node"
            ),
            Self::SharedNode { node } => write!(f, "node {node} is referenced more than once"),
            Self::Unreferenced { node } => write!(f, "node {node} is not referenced by any node"),
            Self::TooDeep { depth } => write!(
                f,
                "the component nests {depth} levels deep, at most {MAX_DEPTH} are allowed"
            ),
            Self::TooMuchText { bytes } => write!(
                f,
                "the component carries {bytes} bytes of text, at most {MAX_TEXT_BYTES} are allowed"
            ),
            Self::InvalidValue { node, reason } => write!(f, "node {node}: {reason}"),
        }
    }
}

impl std::error::Error for ArenaError {}

pub trait ArenaNode {
    fn references(&self, visit: &mut dyn FnMut(u32));

    fn text_bytes(&self) -> usize;
}

pub fn validate<N: ArenaNode>(nodes: &[N]) -> Result<(), ArenaError> {
    if nodes.is_empty() {
        return Err(ArenaError::Empty);
    }
    if nodes.len() > MAX_NODES {
        return Err(ArenaError::TooManyNodes { count: nodes.len() });
    }
    let mut depth = vec![0_usize; nodes.len()];
    depth[0] = 1;
    let mut text = 0_usize;
    for (index, node) in nodes.iter().enumerate() {
        let here = depth[index];
        let node_index = index_u32(index);
        if here == 0 {
            return Err(ArenaError::Unreferenced { node: node_index });
        }
        text = text.saturating_add(node.text_bytes());
        if text > MAX_TEXT_BYTES {
            return Err(ArenaError::TooMuchText { bytes: text });
        }
        let mut failure = None;
        node.references(&mut |reference| {
            if failure.is_some() {
                return;
            }
            let target = reference as usize;
            failure = if target <= index {
                Some(ArenaError::BackwardReference {
                    node: node_index,
                    reference,
                })
            } else if target >= nodes.len() {
                Some(ArenaError::OutOfRange {
                    node: node_index,
                    reference,
                })
            } else if depth[target] != 0 {
                Some(ArenaError::SharedNode { node: reference })
            } else if here >= MAX_DEPTH {
                Some(ArenaError::TooDeep { depth: here + 1 })
            } else {
                depth[target] = here + 1;
                None
            };
        });
        if let Some(failure) = failure {
            return Err(failure);
        }
    }
    Ok(())
}

fn index_u32(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Node {
        refs: Vec<u32>,
        text: usize,
    }

    impl ArenaNode for Node {
        fn references(&self, visit: &mut dyn FnMut(u32)) {
            for reference in &self.refs {
                visit(*reference);
            }
        }

        fn text_bytes(&self) -> usize {
            self.text
        }
    }

    fn node(refs: &[u32]) -> Node {
        Node {
            refs: refs.to_vec(),
            text: 1,
        }
    }

    fn chain(len: usize) -> Vec<Node> {
        (0..len)
            .map(|i| {
                if i + 1 < len {
                    node(&[index_u32(i + 1)])
                } else {
                    node(&[])
                }
            })
            .collect()
    }

    #[test]
    fn a_tree_is_valid() {
        let nodes = [node(&[1, 3]), node(&[2]), node(&[]), node(&[])];
        assert_eq!(validate(&nodes), Ok(()));
    }

    #[test]
    fn an_empty_arena_has_no_root() {
        assert_eq!(validate::<Node>(&[]), Err(ArenaError::Empty));
    }

    #[test]
    fn a_self_reference_or_cycle_is_refused() {
        assert_eq!(
            validate(&[node(&[0])]),
            Err(ArenaError::BackwardReference {
                node: 0,
                reference: 0
            })
        );
        assert_eq!(
            validate(&[node(&[1]), node(&[0])]),
            Err(ArenaError::BackwardReference {
                node: 1,
                reference: 0
            })
        );
    }

    #[test]
    fn a_shared_node_is_refused() {
        assert_eq!(
            validate(&[node(&[1, 2]), node(&[2]), node(&[])]),
            Err(ArenaError::SharedNode { node: 2 })
        );
        assert_eq!(
            validate(&[node(&[1, 1]), node(&[])]),
            Err(ArenaError::SharedNode { node: 1 })
        );
    }

    #[test]
    fn an_orphan_or_dangling_reference_is_refused() {
        assert_eq!(
            validate(&[node(&[]), node(&[])]),
            Err(ArenaError::Unreferenced { node: 1 })
        );
        assert_eq!(
            validate(&[node(&[5])]),
            Err(ArenaError::OutOfRange {
                node: 0,
                reference: 5
            })
        );
    }

    #[test]
    fn depth_is_capped() {
        assert_eq!(validate(&chain(MAX_DEPTH)), Ok(()));
        assert_eq!(
            validate(&chain(MAX_DEPTH + 1)),
            Err(ArenaError::TooDeep {
                depth: MAX_DEPTH + 1
            })
        );
    }

    #[test]
    fn node_count_is_capped() {
        let mut wide = vec![node(&(1..index_u32(MAX_NODES)).collect::<Vec<_>>())];
        wide.extend((1..MAX_NODES).map(|_| node(&[])));
        assert_eq!(validate(&wide), Ok(()));
        wide[0].refs.push(index_u32(MAX_NODES));
        wide.push(node(&[]));
        assert_eq!(
            validate(&wide),
            Err(ArenaError::TooManyNodes {
                count: MAX_NODES + 1
            })
        );
    }

    #[test]
    fn text_is_capped() {
        let big = |text| Node { refs: vec![], text };
        assert_eq!(validate(&[big(MAX_TEXT_BYTES)]), Ok(()));
        assert_eq!(
            validate(&[big(MAX_TEXT_BYTES + 1)]),
            Err(ArenaError::TooMuchText {
                bytes: MAX_TEXT_BYTES + 1
            })
        );
    }
}
