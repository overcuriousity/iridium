// Post-acquire hash-verification pass.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use iridium_core::{HashAlg, ImageFormat};
use iridium_hash::{Digest, new_hasher};

use crate::error::VerifyError;

const VERIFY_CHUNK: usize = 1024 * 1024; // 1 MiB

/// Outcome of a verify pass: `Verified` if digests matched, `Cancelled` if the
/// cancel flag was set before completion. Mismatches and I/O errors return `Err`.
#[derive(Debug, Clone, Copy)]
pub enum VerifyOutcome {
    Verified,
    Cancelled,
}

/// `image_path` is the actual on-disk file: `<dest>.img` for Raw, `<dest>.E01`
/// for EWF. Polls `cancel` between chunk reads so the worker can be aborted
/// without re-hashing the whole image.
pub fn verify_image(
    image_path: &Path,
    format: ImageFormat,
    algorithms: &[HashAlg],
    expected: &[Digest],
    cancel: &Arc<AtomicBool>,
    progress_cb: impl FnMut(u64, u64),
) -> Result<VerifyOutcome, VerifyError> {
    match format {
        ImageFormat::Raw => verify_raw(image_path, algorithms, expected, cancel, progress_cb),
        ImageFormat::Ewf => verify_ewf(image_path, algorithms, expected, cancel, progress_cb),
        ImageFormat::Aff => Err(VerifyError::Io(std::io::Error::other(
            "AFF format verify not implemented",
        ))),
    }
}

fn verify_raw(
    path: &Path,
    algorithms: &[HashAlg],
    expected: &[Digest],
    cancel: &AtomicBool,
    mut progress_cb: impl FnMut(u64, u64),
) -> Result<VerifyOutcome, VerifyError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let total = file.metadata()?.len();
    let mut hashers: Vec<_> = algorithms.iter().map(|a| new_hasher(*a)).collect();
    let mut buf = vec![0u8; VERIFY_CHUNK];
    let mut done: u64 = 0;

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Ok(VerifyOutcome::Cancelled);
        }
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        for h in &mut hashers {
            h.update(&buf[..n]);
        }
        done += n as u64;
        progress_cb(done, total);
    }

    check_digests(hashers, expected).map(|()| VerifyOutcome::Verified)
}

fn verify_ewf(
    ewf_path: &Path,
    algorithms: &[HashAlg],
    expected: &[Digest],
    cancel: &AtomicBool,
    mut progress_cb: impl FnMut(u64, u64),
) -> Result<VerifyOutcome, VerifyError> {
    let mut handle = iridium_ewf::EwfHandle::new()?;
    handle.open_read(&[ewf_path])?;
    let total = handle.media_size()?;
    let mut hashers: Vec<_> = algorithms.iter().map(|a| new_hasher(*a)).collect();
    let mut buf = vec![0u8; VERIFY_CHUNK];
    let mut done: u64 = 0;

    loop {
        if cancel.load(Ordering::Relaxed) {
            handle.close()?;
            return Ok(VerifyOutcome::Cancelled);
        }
        let n = handle.read_buffer(&mut buf)?;
        if n == 0 {
            break;
        }
        for h in &mut hashers {
            h.update(&buf[..n]);
        }
        done += n as u64;
        progress_cb(done, total);
    }

    handle.close()?;
    check_digests(hashers, expected).map(|()| VerifyOutcome::Verified)
}

fn check_digests(
    hashers: Vec<Box<dyn iridium_hash::StreamHasher>>,
    expected: &[Digest],
) -> Result<(), VerifyError> {
    for (h, exp) in hashers.into_iter().zip(expected) {
        let got = h.finish();
        if got.hex != exp.hex {
            return Err(VerifyError::Mismatch {
                algorithm: format!("{:?}", exp.algorithm),
                expected: exp.hex.clone(),
                actual: got.hex,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use iridium_core::HashAlg;
    use iridium_hash::{Digest, new_hasher};

    use super::*;

    fn make_raw_image(data: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.img");
        std::fs::write(&path, data).unwrap();
        (dir, path)
    }

    #[test]
    fn verify_raw_matching_digest() {
        let data = b"the quick brown fox";
        let (_dir, path) = make_raw_image(data);

        let algs = vec![HashAlg::Sha256];
        let mut h = new_hasher(HashAlg::Sha256);
        h.update(data);
        let expected = vec![h.finish()];

        let cancel = AtomicBool::new(false);
        let outcome = verify_raw(&path, &algs, &expected, &cancel, |_, _| {}).unwrap();
        assert!(matches!(outcome, VerifyOutcome::Verified));
    }

    #[test]
    fn verify_raw_mismatched_digest_errors() {
        let data = b"correct data";
        let (_dir, path) = make_raw_image(data);

        let algs = vec![HashAlg::Sha256];
        let bad = vec![Digest {
            algorithm: HashAlg::Sha256,
            hex: "0000000000000000000000000000000000000000000000000000000000000000".into(),
        }];

        let cancel = AtomicBool::new(false);
        let err = verify_raw(&path, &algs, &bad, &cancel, |_, _| {}).unwrap_err();
        assert!(matches!(err, VerifyError::Mismatch { .. }));
    }

    #[test]
    fn verify_raw_observes_cancel() {
        // 8 MiB so multiple chunks are needed.
        let data = vec![0u8; 8 * 1024 * 1024];
        let (_dir, path) = make_raw_image(&data);

        let algs = vec![HashAlg::Sha256];
        let mut h = new_hasher(HashAlg::Sha256);
        h.update(&data);
        let expected = vec![h.finish()];

        let cancel = AtomicBool::new(true); // pre-set so first iteration aborts
        let outcome = verify_raw(&path, &algs, &expected, &cancel, |_, _| {}).unwrap();
        assert!(matches!(outcome, VerifyOutcome::Cancelled));
    }
}
