//! Transports deliver an emitted job to a plotter or to files.

pub mod file;

use std::path::PathBuf;

use thiserror::Error;

use crate::bytelog::ByteLog;
use crate::gpgl::{CheckedJob, SafetyError};

pub use file::FileTransport;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendReport {
    /// Files written, if the transport writes files.
    pub files: Vec<PathBuf>,
    pub bytes_sent: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TransportError {
    #[error("the job failed the safety check, so nothing was sent: {0}")]
    Unsafe(SafetyError),
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    NotAvailable(String),
}

pub trait Transport {
    /// Sends a complete job. Only a [`CheckedJob`] can be sent, so every
    /// job passed `check_safe` before its first byte leaves.
    fn send_job(
        &mut self,
        job: &CheckedJob,
        name: &str,
        log: &mut ByteLog,
    ) -> Result<SendReport, TransportError>;
}

/// Splits checked job bytes into commands: `ESC` plus one byte, or the bytes
/// up to and including ETX. Trailing bytes without ETX form a last piece.
pub fn split_commands(bytes: &[u8]) -> Vec<&[u8]> {
    const ESC: u8 = 0x1b;
    const ETX: u8 = 0x03;
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let end = if bytes[i] == ESC {
            (i + 2).min(bytes.len())
        } else {
            bytes[i..]
                .iter()
                .position(|&b| b == ETX)
                .map_or(bytes.len(), |p| i + p + 1)
        };
        out.push(&bytes[i..end]);
        i = end;
    }
    out
}
