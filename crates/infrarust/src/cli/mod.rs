//! Command-line interface module for user input handling.

pub mod command;
pub mod commands;
pub mod format;
pub mod shutdown;

pub use command::CommandProcessor;
pub use shutdown::ShutdownController;
