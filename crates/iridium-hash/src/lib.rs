// iridium-hash: streaming hash computation (MD5, SHA-1, SHA-256).

mod md5;
mod sha1;
mod sha256;

/// Encode `bytes` as a lowercase hex string.
///
/// Pre-allocates the exact capacity needed to avoid per-byte `String`
/// allocations.
pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(s, "{b:02x}").unwrap();
    }
    s
}

pub use md5::Md5Hasher;
pub use sha1::Sha1Hasher;
pub use sha256::Sha256Hasher;

use iridium_core::HashAlg;

/// Hex-encoded digest produced by a [`StreamHasher`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digest {
    /// The algorithm that produced this digest.
    pub algorithm: HashAlg,
    /// Lowercase hex-encoded hash value.
    pub hex: String,
}

/// Feed byte slices into this and call [`finish`](StreamHasher::finish) for
/// the final digest.
pub trait StreamHasher: Send {
    fn update(&mut self, data: &[u8]);
    fn finish(self: Box<Self>) -> Digest;
    fn algorithm(&self) -> HashAlg;
}

/// Create a boxed [`StreamHasher`] for the given algorithm.
pub fn new_hasher(alg: HashAlg) -> Box<dyn StreamHasher> {
    match alg {
        HashAlg::Md5 => Box::new(Md5Hasher::new()),
        HashAlg::Sha1 => Box::new(Sha1Hasher::new()),
        HashAlg::Sha256 => Box::new(Sha256Hasher::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_hasher_returns_correct_algorithm() {
        for alg in [HashAlg::Md5, HashAlg::Sha1, HashAlg::Sha256] {
            let mut h = new_hasher(alg);
            assert_eq!(h.algorithm(), alg);
            h.update(b"test");
            let d = h.finish();
            assert_eq!(d.algorithm, alg);
        }
    }

    #[test]
    fn multi_update_same_as_single() {
        // Feeding data in two calls must produce the same digest as one call.
        for alg in [HashAlg::Md5, HashAlg::Sha1, HashAlg::Sha256] {
            let mut h1 = new_hasher(alg);
            h1.update(b"hello ");
            h1.update(b"world");
            let d1 = h1.finish();

            let mut h2 = new_hasher(alg);
            h2.update(b"hello world");
            let d2 = h2.finish();

            assert_eq!(d1.hex, d2.hex, "algorithm {alg:?}");
        }
    }
}
