// sha1.rs — SHA-1 StreamHasher impl.

use sha1::{Digest as _, Sha1};

use iridium_core::HashAlg;

use crate::{Digest, StreamHasher};

pub struct Sha1Hasher(Sha1);

impl Sha1Hasher {
    pub fn new() -> Self {
        Self(Sha1::new())
    }
}

impl StreamHasher for Sha1Hasher {
    fn update(&mut self, data: &[u8]) {
        sha1::Digest::update(&mut self.0, data);
    }

    fn finish(self: Box<Self>) -> Digest {
        let bytes = sha1::Digest::finalize(self.0);
        Digest {
            algorithm: HashAlg::Sha1,
            hex: hex_encode(&bytes),
        }
    }

    fn algorithm(&self) -> HashAlg {
        HashAlg::Sha1
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(data: &[u8]) -> String {
        let mut h = Box::new(Sha1Hasher::new());
        h.update(data);
        h.finish().hex
    }

    #[test]
    fn empty() {
        assert_eq!(hash(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn abc() {
        assert_eq!(hash(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    #[test]
    fn algorithm_field() {
        let mut h = Box::new(Sha1Hasher::new());
        h.update(b"x");
        let d = h.finish();
        assert_eq!(d.algorithm, HashAlg::Sha1);
    }
}
