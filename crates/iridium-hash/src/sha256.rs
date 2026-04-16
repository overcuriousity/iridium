// sha256.rs — SHA-256 StreamHasher impl.

use sha2::{Digest as _, Sha256};

use iridium_core::HashAlg;

use crate::{Digest, StreamHasher};

pub struct Sha256Hasher(Sha256);

impl Sha256Hasher {
    pub fn new() -> Self {
        Self(Sha256::new())
    }
}

impl Default for Sha256Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamHasher for Sha256Hasher {
    fn update(&mut self, data: &[u8]) {
        sha2::Digest::update(&mut self.0, data);
    }

    fn finish(self: Box<Self>) -> Digest {
        let bytes = sha2::Digest::finalize(self.0);
        Digest {
            algorithm: HashAlg::Sha256,
            hex: crate::hex_encode(&bytes),
        }
    }

    fn algorithm(&self) -> HashAlg {
        HashAlg::Sha256
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(data: &[u8]) -> String {
        let mut h = Box::new(Sha256Hasher::new());
        h.update(data);
        h.finish().hex
    }

    #[test]
    fn empty() {
        assert_eq!(
            hash(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn abc() {
        assert_eq!(
            hash(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn algorithm_field() {
        let mut h = Box::new(Sha256Hasher::new());
        h.update(b"x");
        let d = h.finish();
        assert_eq!(d.algorithm, HashAlg::Sha256);
    }
}
