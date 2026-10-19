//! Error types.

use std::fmt;

/// A malformed packet or message. Decoders return this instead of panicking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// Ran out of input.
    Truncated,
    /// Unknown packet kind byte.
    BadKind(u8),
    /// Wrong protocol magic.
    BadMagic,
    /// Over a size limit (packet, message, padding rules).
    TooLarge,
    /// Too short for its kind (anti-amplification padding missing).
    TooSmall,
    /// Any other inconsistency.
    Invalid(&'static str),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Truncated => write!(f, "truncated"),
            DecodeError::BadKind(k) => write!(f, "unknown packet kind {k}"),
            DecodeError::BadMagic => write!(f, "bad magic"),
            DecodeError::TooLarge => write!(f, "too large"),
            DecodeError::TooSmall => write!(f, "too small"),
            DecodeError::Invalid(s) => write!(f, "invalid: {s}"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Why a [`send`](crate::Server::send) was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// No such connection (or not connected).
    NotConnected,
    /// The message is over the lane's size limit.
    TooLarge,
    /// The unacked reliable data passed the per-connection cap; the connection
    /// was dropped with [`DisconnectReason::Overflow`](crate::DisconnectReason::Overflow).
    Overflow,
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::NotConnected => write!(f, "not connected"),
            SendError::TooLarge => write!(f, "message too large"),
            SendError::Overflow => write!(f, "reliable overflow"),
        }
    }
}

impl std::error::Error for SendError {}
