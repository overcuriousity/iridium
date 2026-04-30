// hash_pass.rs — post-acquisition sequential hashing of the completed image.
//
// Streaming hashes during out-of-order recovery writes are meaningless because
// the hash depends on the data order.  Instead, after the recovery pipeline
// completes, we re-read the output image sequentially and compute hashes in a
// single forward pass.

use std::{fs::File, io, io::Read as _, path::Path};

use iridium_core::HashAlg;
use iridium_hash::{Digest, new_hasher};

const HASH_BUF: usize = 1024 * 1024; // 1 MiB read buffer

/// Re-read `image_path` from start to end and return one [`Digest`] per
/// algorithm in the same order as `algorithms`.
///
/// Returns an empty `Vec` if `algorithms` is empty.
pub fn hash_pass(image_path: &Path, algorithms: &[HashAlg]) -> io::Result<Vec<Digest>> {
    if algorithms.is_empty() {
        return Ok(vec![]);
    }

    let mut file = File::open(image_path)?;
    let mut hashers: Vec<_> = algorithms.iter().copied().map(new_hasher).collect();
    let mut buf = vec![0u8; HASH_BUF];

    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for h in &mut hashers {
            h.update(&buf[..n]);
        }
    }

    Ok(hashers.into_iter().map(|h| h.finish()).collect())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use iridium_hash::new_hasher;

    fn expected_digest(data: &[u8], alg: HashAlg) -> String {
        let mut h = new_hasher(alg);
        h.update(data);
        h.finish().hex
    }

    #[test]
    fn hash_pass_matches_known_digest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("img");
        let data: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
        std::fs::write(&path, &data).unwrap();

        let algs = vec![HashAlg::Md5, HashAlg::Sha1, HashAlg::Sha256];
        let digests = hash_pass(&path, &algs).unwrap();

        assert_eq!(digests.len(), 3);
        for (alg, digest) in algs.iter().zip(&digests) {
            assert_eq!(digest.algorithm, *alg);
            assert_eq!(digest.hex, expected_digest(&data, *alg), "algorithm {alg:?}");
        }
    }

    #[test]
    fn hash_pass_empty_algorithms_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("img");
        std::fs::write(&path, b"data").unwrap();
        let digests = hash_pass(&path, &[]).unwrap();
        assert!(digests.is_empty());
    }

    #[test]
    fn hash_pass_multipart_same_as_singlepart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("img");
        // Larger than HASH_BUF to exercise multi-read path.
        let data: Vec<u8> = (0u8..=255).cycle().take(HASH_BUF + 512).collect();
        std::fs::write(&path, &data).unwrap();

        let algs = vec![HashAlg::Sha256];
        let digests = hash_pass(&path, &algs).unwrap();
        assert_eq!(digests[0].hex, expected_digest(&data, HashAlg::Sha256));
    }
}
