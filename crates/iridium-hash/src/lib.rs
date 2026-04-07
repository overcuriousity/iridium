// iridium-hash: streaming hash computation.
// Phase 3 will implement the fan-out hasher feeding the acquisition pipeline.

use iridium_core::HashAlg;

/// Opaque hex-encoded digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digest(pub String);

/// Feed byte slices into this and call finish() for the final digest.
pub trait StreamHasher: Send {
    fn update(&mut self, data: &[u8]);
    fn finish(self: Box<Self>) -> Digest;
    fn algorithm(&self) -> HashAlg;
}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        assert!(true);
    }
}
