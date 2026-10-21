//! Join passwords in the handshake.
//!
//! The password never travels, and neither does anything that could be
//! replayed: the client sends `proof = SipHash(secret, cookie ‖ nonce)` in
//! its `ChallengeResponse`, where `secret` is a keyed hash of the password,
//! `nonce` the client's random handshake nonce and `cookie` the server's
//! challenge (bound to the client's address, the nonce and a 10 s time
//! bucket, and accepted once). A recorded `ChallengeResponse` is useless
//! from another address, after ~20 s, or a second time (the server answers
//! a repeated cookie with its earlier decision and opens nothing new).
//!
//! What it does not do: an eavesdropper who records a handshake can still
//! test password guesses offline against it (64-bit SipHash is fast), so a
//! join password should not be a password used anywhere else. There is no
//! encryption of the session itself (§6, "encryption").

use crate::rng::siphash24;

/// The per-password secret (0 = no password).
pub fn secret(password: &str) -> u64 {
    if password.is_empty() {
        0
    } else {
        siphash24(0x5454_5253_7077_6432, 0x7468_696e_6b74_616e, password.as_bytes()) | 1
    }
}

/// The proof for one handshake (0 when `secret` is 0).
pub fn proof(secret: u64, cookie: u64, nonce: u64) -> u64 {
    if secret == 0 {
        return 0;
    }
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&cookie.to_le_bytes());
    b[8..].copy_from_slice(&nonce.to_le_bytes());
    siphash24(secret, secret.rotate_left(32) ^ 0x7061_7373_776f_7264, &b) | 1
}

/// What the server learns about a handshake's password proof
/// ([`crate::ServerEvent::ConnectRequest`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Handshake {
    pub cookie: u64,
    pub nonce: u64,
    pub proof: u64,
}

impl Handshake {
    /// Does the client know `password`? Always true for an empty password.
    pub fn verify(&self, password: &str) -> bool {
        let s = secret(password);
        s == 0 || self.proof == proof(s, self.cookie, self.nonce)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proofs_are_bound_to_the_handshake() {
        let s = secret("hunter2");
        let h = Handshake { cookie: 11, nonce: 22, proof: proof(s, 11, 22) };
        assert!(h.verify("hunter2"));
        assert!(!h.verify("hunter3"));
        assert!(Handshake::default().verify(""));
        assert!(!Handshake::default().verify("x"));
        // Another cookie (address, time) or nonce: another proof.
        assert_ne!(proof(s, 12, 22), h.proof);
        assert_ne!(proof(s, 11, 23), h.proof);
        assert!(!Handshake { cookie: 12, ..h }.verify("hunter2"));
        assert_eq!((secret(""), proof(0, 1, 2)), (0, 0));
    }
}
