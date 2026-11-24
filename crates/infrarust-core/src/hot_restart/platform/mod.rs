#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(not(target_os = "linux"))]
mod unsupported;

use std::path::PathBuf;
use std::sync::Arc;

use super::HandoverError;
use super::package::HandoverPackage;
use crate::binary::RunningBinary;

#[derive(Debug)]
pub struct Handed;

#[derive(Debug, Clone)]
pub struct RestartPlan {
    pub binary: Arc<RunningBinary>,
    pub target: PathBuf,
}

pub trait Successor: Send + 'static {
    fn platform(&self) -> &'static str;

    fn hand_over(
        self: Box<Self>,
        package: HandoverPackage,
        plan: &RestartPlan,
    ) -> Result<Handed, HandoverError>;
}

pub trait Inheritance: Send + 'static {
    fn take_package(&mut self) -> Result<HandoverPackage, HandoverError>;

    fn commit(self: Box<Self>) -> Result<(), HandoverError>;

    fn roll_back(self: Box<Self>, reason: &str) -> HandoverError;
}

pub fn successor() -> Result<Box<dyn Successor>, HandoverError> {
    #[cfg(target_os = "linux")]
    {
        linux::successor()
    }
    #[cfg(not(target_os = "linux"))]
    {
        unsupported::successor()
    }
}

pub fn inherit() -> Result<Option<Box<dyn Inheritance>>, HandoverError> {
    #[cfg(target_os = "linux")]
    {
        linux::inherit()
    }
    #[cfg(not(target_os = "linux"))]
    {
        unsupported::inherit()
    }
}
