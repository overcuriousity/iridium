// writer.rs — ImageWriter trait, RawWriter (flat image), and EwfWriter (EnCase EWF).

use std::{
    fs::{File, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
};

use iridium_ewf::{EwfHandle, LIBEWF_MEDIA_FLAG_PHYSICAL, LIBEWF_MEDIA_TYPE_FIXED};

use crate::AcquireError;

/// Sink that receives sequential chunks of device data.
///
/// On successful completion the pipeline calls: `write_chunk` (repeated) →
/// `embed_digests` → `finalize`. On cancellation the pipeline calls
/// [`discard`](Self::discard) instead of `finalize`, so the writer can drop
/// any partial output. Read errors do not cause an immediate call to either
/// terminator; the pipeline zero-fills the affected chunk and continues.
/// Implementations must be `Send` so the pipeline can run on any thread.
pub trait ImageWriter: Send {
    /// Write one chunk of data to the output.
    fn write_chunk(&mut self, data: &[u8]) -> Result<(), AcquireError>;

    /// Embed hash digest metadata into the output before finalization.
    ///
    /// Called only on successful completion, after all chunks have been hashed
    /// and before [`finalize`](Self::finalize). The pipeline always calls
    /// `finalize` afterwards, even when this returns an error, so the output
    /// container is sealed; the embed error is surfaced to the caller after
    /// sealing succeeds. The default is a no-op; EWF-style writers override
    /// this to store digest strings in image metadata.
    fn embed_digests(&mut self, _digests: &[iridium_hash::Digest]) -> Result<(), AcquireError> {
        Ok(())
    }

    /// Flush and seal the output. Called exactly once on successful completion.
    fn finalize(self: Box<Self>) -> Result<(), AcquireError>;

    /// Drop any partial output. Called exactly once on cancellation, in place
    /// of `finalize`. The default delegates to `finalize`, which preserves any
    /// partial bytes already written (correct for flat-file writers like
    /// [`RawWriter`]). Container-format writers (e.g. EWF) override this to
    /// skip sealing — a partial container with mismatched metadata is worse
    /// than no container — and best-effort delete their on-disk artefacts.
    fn discard(self: Box<Self>) -> Result<(), AcquireError> {
        self.finalize()
    }
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
    /// Base path passed to `open_write`; carried for error messages and for
    /// best-effort cleanup of segment files in [`discard`](Self::discard).
    path: PathBuf,
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

        // Configure media metadata on the already-open handle. The format
        // defaults to EnCase 6 inside libewf, so we don't set it explicitly.
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
        })
    }

    /// Best-effort delete every libewf segment file produced for this image.
    /// libewf names segments `<base>.E01`, `<base>.E02`, …; this enumerates the
    /// containing directory and removes any file whose stem matches the base
    /// path and whose extension starts with `E`. Errors are logged and ignored
    /// — the cancel path must not fail just because cleanup couldn't complete.
    fn delete_segments(&self) {
        let Some(parent) = self.path.parent() else {
            return;
        };
        let Some(stem) = self.path.file_name() else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(parent) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.file_stem() == Some(stem)
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.starts_with('E') || e.starts_with('e'))
                && let Err(e) = std::fs::remove_file(&p)
            {
                log::warn!(
                    "iridium-acquire: failed to delete partial EWF segment {}: {e}",
                    p.display()
                );
            }
        }
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

    fn embed_digests(&mut self, digests: &[iridium_hash::Digest]) -> Result<(), AcquireError> {
        use iridium_core::HashAlg;
        for d in digests {
            let id: &[u8] = match d.algorithm {
                HashAlg::Md5 => b"MD5",
                HashAlg::Sha1 => b"SHA1",
                HashAlg::Sha256 => b"SHA256",
            };
            self.handle
                .set_hash_value(id, d.hex.as_bytes())
                .map_err(|e| AcquireError::EwfWrite {
                    path: self.path.clone(),
                    source: e,
                })?;
        }
        Ok(())
    }

    fn finalize(mut self: Box<Self>) -> Result<(), AcquireError> {
        self.handle
            .write_finalize()
            .map_err(|e| AcquireError::EwfWrite {
                path: self.path.clone(),
                source: e,
            })?;
        self.handle.close().map_err(|e| AcquireError::EwfWrite {
            path: self.path.clone(),
            source: e,
        })
    }

    fn discard(mut self: Box<Self>) -> Result<(), AcquireError> {
        // Skip write_finalize: the bytes written so far do not match the
        // declared media_size, so sealing would either fail or produce a
        // structurally-invalid container. Close the handle to release the
        // FFI allocation, then best-effort delete the partial segments.
        let close_result = self.handle.close().map_err(|e| AcquireError::EwfWrite {
            path: self.path.clone(),
            source: e,
        });
        self.delete_segments();
        close_result
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

    /// Verify embed_digests succeeds for all three supported algorithms.
    #[test]
    fn ewf_writer_embed_digests_succeeds() {
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
        ])
        .unwrap();
        Box::new(w).finalize().unwrap();
    }

    /// EwfWriter::create must reject a destination path that already has any
    /// extension — libewf appends `.E01` itself and would otherwise produce
    /// `<base>.<ext>.E01`.
    #[test]
    fn ewf_writer_create_rejects_path_with_extension() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("image.tmp");

        match EwfWriter::create(&dest, 4096, 512) {
            Err(AcquireError::EwfOpen { .. }) => {}
            Err(other) => panic!("expected EwfOpen error, got: {other:?}"),
            Ok(_) => panic!("expected EwfOpen error, got Ok"),
        }
    }

    /// `discard` must close the libewf handle without sealing and best-effort
    /// remove the partial `.E01` segment so a cancelled acquisition does not
    /// leave a structurally-invalid image on disk.
    #[test]
    fn ewf_writer_discard_removes_partial_segment() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("partial");

        // Declare 64 KiB but only write 4 KiB, then discard.
        let mut w = Box::new(EwfWriter::create(&dest, 64 * 1024, 512).unwrap());
        w.write_chunk(&vec![0xCDu8; 4096]).unwrap();
        w.discard().unwrap();

        let e01 = dest.with_extension("E01");
        assert!(
            !e01.exists(),
            "partial .E01 segment must be removed by discard, but {} still exists",
            e01.display()
        );
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
            fn embed_digests(
                &mut self,
                _digests: &[iridium_hash::Digest],
            ) -> Result<(), AcquireError> {
                self.0.lock().unwrap().0.push("embed_digests");
                Ok(())
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

    /// On cancellation the pipeline must call `discard` (not `finalize`) so
    /// container-format writers can drop the partial output without sealing
    /// it into a structurally-invalid image.
    #[test]
    fn pipeline_calls_discard_on_cancellation() {
        use crate::{AcquireError, AcquireJob, run_with_writer};
        use iridium_core::HashAlg;
        use iridium_device::Disk;
        use std::sync::atomic::Ordering;
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct CallOrder(Vec<&'static str>);

        struct SpyWriter(Arc<Mutex<CallOrder>>);

        impl ImageWriter for SpyWriter {
            fn write_chunk(&mut self, _data: &[u8]) -> Result<(), AcquireError> {
                self.0.lock().unwrap().0.push("write_chunk");
                Ok(())
            }
            fn embed_digests(
                &mut self,
                _digests: &[iridium_hash::Digest],
            ) -> Result<(), AcquireError> {
                self.0.lock().unwrap().0.push("embed_digests");
                Ok(())
            }
            fn finalize(self: Box<Self>) -> Result<(), AcquireError> {
                self.0.lock().unwrap().0.push("finalize");
                Ok(())
            }
            fn discard(self: Box<Self>) -> Result<(), AcquireError> {
                self.0.lock().unwrap().0.push("discard");
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
        // Cancel before the loop runs so the very first iteration takes the
        // cancel branch.
        job.cancel.store(true, Ordering::Relaxed);
        let result = run_with_writer(job, spy).unwrap();

        assert!(!result.complete);
        let calls = order.lock().unwrap();
        assert!(
            calls.0.contains(&"discard"),
            "cancellation must call discard; got: {:?}",
            calls.0
        );
        assert!(
            !calls.0.contains(&"finalize"),
            "cancellation must NOT call finalize; got: {:?}",
            calls.0
        );
    }
}
