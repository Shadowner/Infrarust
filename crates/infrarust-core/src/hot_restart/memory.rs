use std::sync::{Arc, Mutex, PoisonError};

use super::HandoverError;
use super::package::HandoverPackage;
use super::platform::Inheritance;
use super::state::{decode, encode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InheritanceOutcome {
    Pending,
    Committed,
    RolledBack(String),
}

#[derive(Debug, Clone)]
pub struct InheritanceRecord(Arc<Mutex<InheritanceOutcome>>);

impl InheritanceRecord {
    pub fn outcome(&self) -> InheritanceOutcome {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn settle(&self, outcome: InheritanceOutcome) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = outcome;
    }
}

#[derive(Debug)]
pub struct MemoryInheritance {
    package: Option<HandoverPackage>,
    record: InheritanceRecord,
}

impl MemoryInheritance {
    pub fn new(package: HandoverPackage) -> Self {
        Self {
            package: Some(package),
            record: InheritanceRecord(Arc::new(Mutex::new(InheritanceOutcome::Pending))),
        }
    }

    pub fn record(&self) -> InheritanceRecord {
        self.record.clone()
    }
}

impl Inheritance for MemoryInheritance {
    fn take_package(&mut self) -> Result<HandoverPackage, HandoverError> {
        let package = self
            .package
            .take()
            .ok_or_else(|| HandoverError::Malformed("the package was already taken".into()))?;
        let wire = encode(&package.state, &package.tails).and_then(|bytes| decode(&bytes));
        match wire {
            Ok((state, tails)) => Ok(HandoverPackage {
                state,
                tails,
                handles: package.handles,
            }),
            Err(error) => {
                self.package = Some(package);
                Err(error)
            }
        }
    }

    fn commit(self: Box<Self>) -> Result<(), HandoverError> {
        self.record.settle(InheritanceOutcome::Committed);
        Ok(())
    }

    fn roll_back(self: Box<Self>, reason: &str) -> HandoverError {
        self.record
            .settle(InheritanceOutcome::RolledBack(reason.to_string()));
        HandoverError::RolledBack(reason.to_string())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    use super::*;
    use crate::hot_restart::package::ListenerHandover;

    fn package() -> (HandoverPackage, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let listener = ListenerHandover {
            bind: listener.local_addr().unwrap(),
            socket: listener,
        };
        let (package, dropped) =
            HandoverPackage::build("2.0.0-beta.6", 12, Some(listener), Vec::new()).unwrap();
        assert!(dropped.is_empty());
        (package, client)
    }

    #[test]
    fn the_package_crosses_the_wire_format_with_live_handles() {
        let (sent, mut client) = package();
        let state = sent.state.clone();
        let mut inheritance: Box<dyn Inheritance> = Box::new(MemoryInheritance::new(sent));

        let received = inheritance.take_package().unwrap();

        assert_eq!(received.state, state);
        let unpacked = received.unpack().unwrap();
        assert_eq!(unpacked.from_version, "2.0.0-beta.6");
        assert_eq!(unpacked.next_player_id, 12);
        let listener = unpacked.listener.unwrap();
        let (mut accepted, _) = listener.socket.accept().unwrap();
        client.write_all(b"ping").unwrap();
        let mut buf = [0; 4];
        accepted.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"ping");
    }

    #[test]
    fn the_package_can_only_be_taken_once() {
        let (sent, _client) = package();
        let mut inheritance = MemoryInheritance::new(sent);
        inheritance.take_package().unwrap();

        assert!(matches!(
            inheritance.take_package(),
            Err(HandoverError::Malformed(_))
        ));
    }

    #[test]
    fn a_commit_is_recorded() {
        let (sent, _client) = package();
        let inheritance = MemoryInheritance::new(sent);
        let record = inheritance.record();
        assert_eq!(record.outcome(), InheritanceOutcome::Pending);

        Box::new(inheritance).commit().unwrap();

        assert_eq!(record.outcome(), InheritanceOutcome::Committed);
    }

    #[test]
    fn a_roll_back_is_recorded_with_its_reason() {
        let (sent, _client) = package();
        let mut inheritance = MemoryInheritance::new(sent);
        let record = inheritance.record();
        let _held = inheritance.take_package().unwrap();

        let error = Box::new(inheritance).roll_back("the new config is invalid");

        assert!(
            matches!(&error, HandoverError::RolledBack(reason) if reason == "the new config is invalid")
        );
        assert_eq!(
            record.outcome(),
            InheritanceOutcome::RolledBack("the new config is invalid".into())
        );
    }
}
