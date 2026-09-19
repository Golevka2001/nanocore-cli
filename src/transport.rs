//! Transport abstraction.
//!
//! Implement this for your MIDI backend (e.g. `midir`, `btleplug`) or for a
//! test harness that records/injects raw bytes.

use std::error::Error;

/// A bidirectional byte transport carrying whole Nanocore messages.
pub trait Transport {
    type Error: Error + 'static;

    /// Send raw bytes (a fully encoded [`crate::Frame`]).
    fn send(&mut self, data: &[u8]) -> Result<(), Self::Error>;

    /// Receive any bytes currently available. May return an empty vec.
    fn receive(&mut self) -> Result<Vec<u8>, Self::Error>;
}

/// In-memory transport useful for tests and for replaying captured traffic.
#[derive(Default)]
pub struct LoopbackTransport {
    pub sent: Vec<Vec<u8>>,
    pub incoming: Vec<u8>,
}

impl LoopbackTransport {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Transport for LoopbackTransport {
    type Error = std::convert::Infallible;

    fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        self.sent.push(data.to_vec());
        Ok(())
    }

    fn receive(&mut self) -> Result<Vec<u8>, Self::Error> {
        Ok(std::mem::take(&mut self.incoming))
    }
}
