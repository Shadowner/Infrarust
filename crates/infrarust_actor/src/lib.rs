use tokio::sync::mpsc;
pub mod client;
pub mod server;
pub mod supervisor;

pub trait Actor<T> {
    fn new(sender: mpsc::Receiver<T>, id: String) -> Self
    where
        Self: Sized;
    fn handle_message(&mut self, message: T);
}
