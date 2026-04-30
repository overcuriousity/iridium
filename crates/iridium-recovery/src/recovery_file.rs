// recovery_file.rs — pwrite-based output file for out-of-order recovery writes.

use std::{
    fs::{File, OpenOptions},
    io,
    os::unix::fs::FileExt as _,
    path::{Path, PathBuf},
};

/// A pre-allocated output image file that supports writes at arbitrary offsets.
///
/// Opened with `O_WRONLY | O_CREAT | O_TRUNC`.  `set_len(size_bytes)` is
/// called immediately so the file is sparse-ready: unwritten regions read as
/// zeros on any POSIX filesystem that supports sparse files (ext4, xfs, btrfs,
/// tmpfs).  On filesystems without sparse support (e.g. FAT32) the pre-alloc
/// blocks for the entire `size_bytes` upfront.
///
/// Reading the completed image is done by the hash pass, which opens the file
/// by path via a separate `File::open` handle.
pub struct RecoveryFile {
    file: File,
    path: PathBuf,
    size_bytes: u64,
}

impl RecoveryFile {
    /// Create (or truncate) the output image at `path` and pre-allocate
    /// `size_bytes` bytes.
    pub fn create(path: &Path, size_bytes: u64) -> io::Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.set_len(size_bytes)?;
        Ok(Self {
            file,
            path: path.to_owned(),
            size_bytes,
        })
    }

    /// Write `data` starting at `offset` bytes from the beginning of the file.
    ///
    /// Uses `pwrite` internally so multiple callers at different offsets do not
    /// interfere.  Returns an error if the write does not cover all of `data`.
    pub fn write_at(&self, offset: u64, data: &[u8]) -> io::Result<()> {
        self.file.write_all_at(data, offset)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_preallocates_correct_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("img");
        let rf = RecoveryFile::create(&path, 4096).unwrap();
        assert_eq!(rf.size_bytes(), 4096);
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(meta.len(), 4096);
    }

    #[test]
    fn write_at_arbitrary_offset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("img");
        let rf = RecoveryFile::create(&path, 512).unwrap();
        let payload = b"hello";
        rf.write_at(100, payload).unwrap();
        let f = std::fs::File::open(&path).unwrap();
        let mut buf = [0u8; 5];
        f.read_exact_at(&mut buf, 100).unwrap();
        assert_eq!(&buf, payload);
    }

    #[test]
    fn unwritten_regions_read_as_zeros() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("img");
        let rf = RecoveryFile::create(&path, 512).unwrap();
        // write only one byte at offset 0
        rf.write_at(0, &[0xABu8]).unwrap();
        let f = std::fs::File::open(&path).unwrap();
        let mut buf = [0xFFu8; 4];
        f.read_exact_at(&mut buf, 508).unwrap();
        assert_eq!(buf, [0u8; 4]);
    }

    #[test]
    fn truncate_on_create() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("img");
        // create once with a large size and distinct data
        {
            let rf = RecoveryFile::create(&path, 512).unwrap();
            rf.write_at(0, &[0xFFu8; 512]).unwrap();
        }
        // re-create with smaller size — must truncate old content
        let rf = RecoveryFile::create(&path, 256).unwrap();
        assert_eq!(rf.size_bytes(), 256);
        let meta = std::fs::metadata(&path).unwrap();
        assert_eq!(meta.len(), 256);
    }
}
