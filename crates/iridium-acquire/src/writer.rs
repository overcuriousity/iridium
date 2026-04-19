// writer.rs — ImageWriter trait, RawWriter (flat image), and EwfWriter (EnCase EWF).

use std::{
    fs::{File, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
};

use iridium_ewf::{
    EwfHandle, LIBEWF_FORMAT_ENCASE6, LIBEWF_MEDIA_FLAG_PHYSICAL, LIBEWF_MEDIA_TYPE_FIXED,
};

use crate::AcquireError;

/// Sink that receives sequential chunks of device data.
///
/// On successful completion the pipeline calls: `write_chunk` (repeated) →
/// `embed_digests` → `finalize`. On cancellation or a read error the pipeline
/// calls `finalize` directly, without calling `embed_digests` first, so the
/// output may be incomplete. Implementations must be `Send` so the pipeline
/// can run on any thread.
pub trait ImageWriter: Send {
    /// Write one chunk of data to the output.
    fn write_chunk(&mut self, data: &[u8]) -> Result<(), AcquireError>;

    /// Embed hash digest metadata into the output before finalization.
    ///
    /// Called only on successful completion, after all chunks have been hashed
    /// and before [`finalize`]. Not called on cancellation or pipeline errors.
    /// The default is a no-op; EWF-style writers override this to store digest
    /// strings in image metadata.
    fn embed_digests(&mut self, _digests: &[iridium_hash::Digest]) {}

    /// Flush and close the output. Called exactly once as the last step.
    fn finalize(self: Box<Self>) -> Result<(), AcquireError>;
}

// ── RawWriter ─────────────────────────────────────────────────────────────────

/// Writes a flat raw image file (`.img`).
///
/// The file is created (or truncated) at construction time.
pub struct RawWriter {
    file: File,
    path: PathBuf,
}

impl RawWriter {
    /// Create (or truncate) `<dest_path>.img` and open it for writing.
    pub fn create(dest_path: &Path) -> Result<Self, AcquireError> {
        let path = dest_path.with_extension("img");
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .map_err(|e| AcquireError::WriterOpen {
                path: path.clone(),
                source: e,
            })?;
        Ok(Self { file, path })
    }

    /// Path of the output file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ImageWriter for RawWriter {
    fn write_chunk(&mut self, data: &[u8]) -> Result<(), AcquireError> {
        self.file.write_all(data).map_err(|e| AcquireError::Write {
            path: self.path.clone(),
            source: e,
        })
    }

    fn finalize(mut self: Box<Self>) -> Result<(), AcquireError> {
        self.file.flush().map_err(|e| AcquireError::Write {
            path: self.path.clone(),
            source: e,
        })?;
        self.file.sync_all().map_err(|e| AcquireError::Write {
            path: self.path.clone(),
            source: e,
        })
    }
}

// ── EwfWriter ─────────────────────────────────────────────────────────────────

/// Writes an EnCase 6 EWF image (`.E01`) via libewf.
///
/// Construct with [`EwfWriter::create`], then pass to [`crate::run_with_writer`]
/// or use the convenience wrapper [`crate::run_ewf`].
pub struct EwfWriter {
    handle: EwfHandle,
    /// Base path passed to `open_write`; carried for error messages only.
    path: PathBuf,
    /// First hash-embedding error from `embed_digests`, surfaced in `finalize`.
    embed_error: Option<AcquireError>,
}

impl EwfWriter {
    /// Create and open an EWF writer for `dest_path`.
    ///
    /// libewf appends the format-appropriate extension (`.E01` for EnCase 6)
    /// automatically — do not include an extension in `dest_path`.
    ///
    /// `size_bytes` must equal the total number of bytes that will be written
    /// (i.e. `Disk::size_bytes`). `sector_size` is the logical sector size for
    /// the bytes-per-sector metadata field.
    pub fn create(
        dest_path: &Path,
        size_bytes: u64,
        sector_size: u32,
    ) -> Result<Self, AcquireError> {
        if dest_path.extension().is_some() {
            return Err(AcquireError::EwfOpen {
                path: dest_path.to_path_buf(),
                source: iridium_ewf::EwfError::InvalidPath(
                    "dest_path must not include an extension; libewf appends .E01 automatically"
                        .into(),
                ),
            });
        }
        let open_err = |e| AcquireError::EwfOpen {
            path: dest_path.to_path_buf(),
            source: e,
        };
        let setup_err = |e| AcquireError::EwfWrite {
            path: dest_path.to_path_buf(),
            source: e,
        };

        let mut handle = EwfHandle::new().map_err(open_err)?;
        handle.open_write(dest_path).map_err(open_err)?;

        // These calls configure the already-open handle with format and media metadata.
        // The constants are stable libewf ABI values.
        handle
            .set_format(LIBEWF_FORMAT_ENCASE6)
            .map_err(setup_err)?;
        handle
            .set_media_type(LIBEWF_MEDIA_TYPE_FIXED)
            .map_err(setup_err)?;
        handle
            .set_media_flags(LIBEWF_MEDIA_FLAG_PHYSICAL)
            .map_err(setup_err)?;

        // Sector size and total size must be set before the first write_buffer call.
        handle
            .set_bytes_per_sector(sector_size)
            .map_err(setup_err)?;
        handle.set_media_size(size_bytes).map_err(setup_err)?;

        Ok(Self {
            handle,
            path: dest_path.to_path_buf(),
            embed_error: None,
        })
    }
}

impl ImageWriter for EwfWriter {
    fn write_chunk(&mut self, mut data: &[u8]) -> Result<(), AcquireError> {
        while !data.is_empty() {
            let n = self
                .handle
                .write_buffer(data)
                .map_err(|e| AcquireError::EwfWrite {
                    path: self.path.clone(),
                    source: e,
                })?;
            if n == 0 {
                // write_buffer returned 0 without an error — treat as a write stall.
                return Err(AcquireError::EwfWrite {
                    path: self.path.clone(),
                    source: iridium_ewf::EwfError::Library(
                        "write_buffer returned 0 (stall)".into(),
                    ),
                });
            }
            data = &data[n..];
        }
        Ok(())
    }

    fn embed_digests(&mut self, digests: &[iridium_hash::Digest]) {
        use iridium_core::HashAlg;
        for d in digests {
            let id: &[u8] = match d.algorithm {
                HashAlg::Md5 => b"MD5",
                HashAlg::Sha1 => b"SHA1",
                HashAlg::Sha256 => b"SHA256",
            };
            if let Err(e) = self.handle.set_hash_value(id, d.hex.as_bytes()) {
                // Store the first failure; finalize will surface it so the
                // caller can detect forensically-incomplete output.
                if self.embed_error.is_none() {
                    self.embed_error = Some(AcquireError::EwfWrite {
                        path: self.path.clone(),
                        source: e,
                    });
                }
            }
        }
    }

    fn finalize(mut self: Box<Self>) -> Result<(), AcquireError> {
        // Always seal and close the image so the container is structurally
        // valid even when hash embedding failed. Structural errors take
        // priority; embed_error is reported only if sealing succeeded.
        let embed_error = self.embed_error.take();
        self.handle
            .write_finalize()
            .map_err(|e| AcquireError::EwfWrite {
                path: self.path.clone(),
                source: e,
            })?;
        self.handle.close().map_err(|e| AcquireError::EwfWrite {
            path: self.path.clone(),
            source: e,
        })?;
        if let Some(e) = embed_error {
            return Err(e);
        }
        Ok(())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── RawWriter tests ───────────────────────────────────────────────────────

    #[test]
    fn raw_writer_creates_file_and_writes() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("test");

        let mut w = Box::new(RawWriter::create(&dest).unwrap());
        assert!(w.path().exists());

        w.write_chunk(b"hello ").unwrap();
        w.write_chunk(b"world").unwrap();
        w.finalize().unwrap();

        let content = std::fs::read(dest.with_extension("img")).unwrap();
        assert_eq!(content, b"hello world");
    }

    #[test]
    fn raw_writer_truncates_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("existing");
        let img = dest.with_extension("img");

        std::fs::write(&img, b"old content that is longer").unwrap();

        let mut w = Box::new(RawWriter::create(&dest).unwrap());
        w.write_chunk(b"new").unwrap();
        w.finalize().unwrap();

        assert_eq!(std::fs::read(&img).unwrap(), b"new");
    }

    // ── EwfWriter tests ───────────────────────────────────────────────────────

    /// Verify EwfWriter creates a .E01 segment file and the file is non-empty.
    #[test]
    fn ewf_writer_creates_e01_file() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("test_image");

        let data = vec![0xABu8; 64 * 1024]; // 64 KiB
        let mut w = Box::new(EwfWriter::create(&dest, data.len() as u64, 512).unwrap());
        w.write_chunk(&data).unwrap();
        w.finalize().unwrap();

        let e01 = dest.with_extension("E01");
        assert!(e01.exists(), ".E01 segment file must be created by libewf");
        assert!(
            e01.metadata().unwrap().len() > 0,
            ".E01 file must be non-empty"
        );
    }

    /// Verify embed_digests does not panic or error for all three algorithms.
    #[test]
    fn ewf_writer_embed_digests_does_not_panic() {
        use iridium_core::HashAlg;
        use iridium_hash::Digest;

        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("hash_test");

        let data = vec![0u8; 512];
        let mut w = EwfWriter::create(&dest, data.len() as u64, 512).unwrap();
        w.write_chunk(&data).unwrap();
        w.embed_digests(&[
            Digest {
                algorithm: HashAlg::Md5,
                hex: "d41d8cd98f00b204e9800998ecf8427e".into(),
            },
            Digest {
                algorithm: HashAlg::Sha1,
                hex: "da39a3ee5e6b4b0d3255bfef95601890afd80709".into(),
            },
            Digest {
                algorithm: HashAlg::Sha256,
                hex: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into(),
            },
        ]);
        Box::new(w).finalize().unwrap();
    }

    /// Verify pipeline calls embed_digests before finalize using a spy writer.
    #[test]
    fn pipeline_embed_digests_called_before_finalize() {
        use crate::{AcquireError, AcquireJob, run_with_writer};
        use iridium_core::HashAlg;
        use iridium_device::Disk;
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct CallOrder(Vec<&'static str>);

        struct SpyWriter(Arc<Mutex<CallOrder>>);

        impl ImageWriter for SpyWriter {
            fn write_chunk(&mut self, _data: &[u8]) -> Result<(), AcquireError> {
                self.0.lock().unwrap().0.push("write_chunk");
                Ok(())
            }
            fn embed_digests(&mut self, _digests: &[iridium_hash::Digest]) {
                self.0.lock().unwrap().0.push("embed_digests");
            }
            fn finalize(self: Box<Self>) -> Result<(), AcquireError> {
                self.0.lock().unwrap().0.push("finalize");
                Ok(())
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.img");
        std::fs::write(&src, vec![0u8; 512]).unwrap();

        let disk = Disk {
            path: src,
            model: String::new(),
            serial: String::new(),
            size_bytes: 512,
            logical_sector_size: 512,
            sector_size: 512,
            hpa_size_bytes: None,
            dco_restricted: false,
            removable: false,
            rotational: false,
            read_only: true,
            partition_of: None,
        };

        let order = Arc::new(Mutex::new(CallOrder::default()));
        let spy = Box::new(SpyWriter(Arc::clone(&order)));
        let job = AcquireJob::new(disk, dir.path().join("out"), vec![HashAlg::Md5]);
        run_with_writer(job, spy).unwrap();

        let calls = order.lock().unwrap();
        let embed_pos = calls.0.iter().position(|&s| s == "embed_digests").unwrap();
        let finalize_pos = calls.0.iter().position(|&s| s == "finalize").unwrap();
        assert!(
            embed_pos < finalize_pos,
            "embed_digests must be called before finalize; got: {:?}",
            calls.0
        );
    }
}
