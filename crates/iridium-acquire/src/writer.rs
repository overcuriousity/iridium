// writer.rs — ImageWriter trait and raw flat-file implementation.
//
// Phase 4 will add EwfWriter implementing ImageWriter.

use std::{
    fs::{File, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
};

use crate::AcquireError;

/// Sink that receives sequential chunks of device data.
///
/// The pipeline calls [`write_chunk`] for every block it reads, then
/// [`finalize`] exactly once when the acquisition completes or is cancelled.
/// Implementations must be `Send` so the pipeline can run on any thread.
///
/// # Phase 4 note
/// `EwfWriter` in `iridium-ewf` will implement this trait to enable EWF output.
pub trait ImageWriter: Send {
    /// Write one chunk of data to the output.
    fn write_chunk(&mut self, data: &[u8]) -> Result<(), AcquireError>;

    /// Flush and close the output.  Called exactly once, after the last chunk.
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
            .map_err(|e| AcquireError::WriterOpen { path: path.clone(), source: e })?;
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
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
}
