// Post-acquire hash-verification pass.

use std::path::PathBuf;

use iridium_core::{HashAlg, ImageFormat};
use iridium_hash::{Digest, new_hasher};

use crate::error::VerifyError;

const VERIFY_CHUNK: usize = 1024 * 1024; // 1 MiB

pub fn verify_image(
    image_path: &PathBuf,
    format: ImageFormat,
    algorithms: &[HashAlg],
    expected: &[Digest],
    progress_cb: impl FnMut(u64, u64),
) -> Result<(), VerifyError> {
    match format {
        ImageFormat::Raw => verify_raw(image_path, algorithms, expected, progress_cb),
        ImageFormat::Ewf => verify_ewf(image_path, algorithms, expected, progress_cb),
        ImageFormat::Aff => Err(VerifyError::Io(std::io::Error::other(
            "AFF format verify not implemented",
        ))),
    }
}

fn verify_raw(
    path: &PathBuf,
    algorithms: &[HashAlg],
    expected: &[Digest],
    mut progress_cb: impl FnMut(u64, u64),
) -> Result<(), VerifyError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let total = file.metadata()?.len();
    let mut hashers: Vec<_> = algorithms.iter().map(|a| new_hasher(*a)).collect();
    let mut buf = vec![0u8; VERIFY_CHUNK];
    let mut done: u64 = 0;

    loop {
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

    check_digests(hashers, expected)
}

fn verify_ewf(
    dest_path: &PathBuf,
    algorithms: &[HashAlg],
    expected: &[Digest],
    mut progress_cb: impl FnMut(u64, u64),
) -> Result<(), VerifyError> {
    // The dest_path for EWF is without extension; libewf wrote "<dest>.E01".
    let ewf_path = {
        let name = dest_path
            .file_name()
            .map(|n| format!("{}.E01", n.to_string_lossy()))
            .unwrap_or_else(|| "image.E01".into());
        dest_path.with_file_name(name)
    };

    let mut handle = iridium_ewf::EwfHandle::new()?;
    handle.open_read(&[ewf_path.as_path()])?;
    let total = handle.media_size()?;
    let mut hashers: Vec<_> = algorithms.iter().map(|a| new_hasher(*a)).collect();
    let mut buf = vec![0u8; VERIFY_CHUNK];
    let mut done: u64 = 0;

    loop {
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
    check_digests(hashers, expected)
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

        verify_raw(&path, &algs, &expected, |_, _| {}).unwrap();
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

        let err = verify_raw(&path, &algs, &bad, |_, _| {}).unwrap_err();
        assert!(matches!(err, VerifyError::Mismatch { .. }));
    }
}
