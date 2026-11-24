use super::{Inheritance, Successor};
use crate::hot_restart::HandoverError;

pub fn successor() -> Result<Box<dyn Successor>, HandoverError> {
    Err(HandoverError::Unsupported(std::env::consts::OS))
}

#[allow(clippy::unnecessary_wraps)]
pub const fn inherit() -> Result<Option<Box<dyn Inheritance>>, HandoverError> {
    Ok(None)
}
