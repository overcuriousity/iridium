// md5.rs — MD5 StreamHasher impl.

use md5::{Digest as _, Md5};

use iridium_core::HashAlg;

use crate::{Digest, StreamHasher};

pub struct Md5Hasher(Md5);

impl Md5Hasher {
    pub fn new() -> Self {
        Self(Md5::new())
    }
}

impl StreamHasher for Md5Hasher {
    fn update(&mut self, data: &[u8]) {
        md5::Digest::update(&mut self.0, data);
    }

    fn finish(self: Box<Self>) -> Digest {
        let bytes = md5::Digest::finalize(self.0);
        Digest {
            algorithm: HashAlg::Md5,
            hex: hex_encode(&bytes),
        }
    }

    fn algorithm(&self) -> HashAlg {
        HashAlg::Md5
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(data: &[u8]) -> String {
        let mut h = Box::new(Md5Hasher::new());
        h.update(data);
        h.finish().hex
    }

    #[test]
    fn empty() {
        assert_eq!(hash(b""), "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[test]
    fn abc() {
        assert_eq!(hash(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
    }

    #[test]
    fn algorithm_field() {
        let mut h = Box::new(Md5Hasher::new());
        h.update(b"x");
        let d = h.finish();
        assert_eq!(d.algorithm, HashAlg::Md5);
    }
}
