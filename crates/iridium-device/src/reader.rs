// reader.rs — O_DIRECT forensic read-only device access with internal alignment.

use std::{
    alloc::{Layout, alloc, dealloc},
    path::Path,
    ptr::NonNull,
};

use std::os::unix::fs::FileExt;

use crate::{DeviceError, Disk};

// ── Aligned buffer ────────────────────────────────────────────────────────────

/// A heap-allocated buffer whose base address is aligned to `align` bytes.
/// Required for O_DIRECT reads, which mandate kernel-page-aligned I/O.
struct AlignedBuf {
    ptr: NonNull<u8>,
    layout: Layout,
}

impl AlignedBuf {
    /// Allocate `size` bytes aligned to `align`. Panics if size or align is 0.
    fn new(size: usize, align: usize) -> Self {
        let layout = Layout::from_size_align(size, align).expect("invalid layout");
        // SAFETY: layout is non-zero and valid.
        let raw = unsafe { alloc(layout) };
        let ptr = NonNull::new(raw).expect("allocation failed");
        Self { ptr, layout }
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: ptr is valid and exclusively owned.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.layout.size()) }
    }

    fn len(&self) -> usize {
        self.layout.size()
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        // SAFETY: allocated with the same layout.
        unsafe { dealloc(self.ptr.as_ptr(), self.layout) };
    }
}

// SAFETY: AlignedBuf owns its allocation exclusively; no shared references.
unsafe impl Send for AlignedBuf {}

// ── DeviceReader ──────────────────────────────────────────────────────────────

/// A read-only file descriptor over a block device, with O_DIRECT support.
///
/// Reads are performed via `pread64` at explicit offsets. When opened with
/// `O_DIRECT` an internal sector-aligned buffer is used; the data is then
/// copied into the caller's (arbitrarily aligned) slice.
pub struct DeviceReader {
    file: std::fs::File,
    size_bytes: u64,
    /// Logical sector size used for O_DIRECT alignment (from `queue/logical_block_size`).
    logical_sector_size: u32,
    /// `Some` when opened with O_DIRECT; holds the aligned scratch buffer.
    aligned_buf: Option<AlignedBuf>,
}

impl DeviceReader {
    /// Read up to `buf.len()` bytes from the device at `offset` bytes from
    /// the start of the device.
    ///
    /// When O_DIRECT is active the read is rounded up to a full sector and the
    /// result is trimmed before copying into `buf`. This is transparent to the
    /// caller.
    ///
    /// Returns the number of bytes copied into `buf`. Returns `0` when
    /// `offset >= size_bytes`.
    pub fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, DeviceError> {
        if offset >= self.size_bytes || buf.is_empty() {
            return Ok(0);
        }

        let max_len = (self.size_bytes - offset).min(buf.len() as u64) as usize;

        match &mut self.aligned_buf {
            None => {
                // No O_DIRECT: read directly into caller's buffer.
                let n = self
                    .file
                    .read_at(&mut buf[..max_len], offset)
                    .map_err(|e| DeviceError::Read {
                        offset,
                        source: nix::Error::from(nix::errno::Errno::from_raw(
                            e.raw_os_error().unwrap_or(libc::EIO),
                        )),
                    })?;
                Ok(n)
            }
            Some(ab) => {
                let sector = self.logical_sector_size as usize;
                let sector_u64 = self.logical_sector_size as u64;

                // O_DIRECT requires offset, length, and buffer all aligned to sector.
                // Round offset down to the nearest sector boundary and read from there.
                let aligned_offset = (offset / sector_u64) * sector_u64;
                let prefix = (offset - aligned_offset) as usize;

                // Total span to read (prefix bytes before requested data + the data).
                let aligned_len = round_up(prefix + max_len, sector);

                // Grow the scratch buffer if needed.
                // next_power_of_two ensures Layout alignment is always valid.
                if ab.len() < aligned_len {
                    *ab = AlignedBuf::new(aligned_len, sector.next_power_of_two());
                }

                let n = self
                    .file
                    .read_at(&mut ab.as_mut_slice()[..aligned_len], aligned_offset)
                    .map_err(|e| DeviceError::Read {
                        offset,
                        source: nix::Error::from(nix::errno::Errno::from_raw(
                            e.raw_os_error().unwrap_or(libc::EIO),
                        )),
                    })?;

                // n covers [aligned_offset, aligned_offset + n); useful data starts at prefix.
                let available = n.saturating_sub(prefix);
                let copy_len = available.min(max_len);
                buf[..copy_len].copy_from_slice(&ab.as_mut_slice()[prefix..prefix + copy_len]);
                Ok(copy_len)
            }
        }
    }

    /// Total device size in bytes as reported by the kernel.
    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    /// Logical sector size in bytes (used for O_DIRECT alignment).
    pub fn logical_sector_size(&self) -> u32 {
        self.logical_sector_size
    }
}

// ── open_read_only ────────────────────────────────────────────────────────────

pub(crate) fn open_read_only(disk: &Disk) -> Result<DeviceReader, DeviceError> {
    open_inner(&disk.path, disk.size_bytes, disk.logical_sector_size)
}

fn open_inner(
    path: &Path,
    size_bytes: u64,
    logical_sector_size: u32,
) -> Result<DeviceReader, DeviceError> {
    // Try O_RDONLY | O_DIRECT | O_NOATIME first.
    match try_open(path, true) {
        Ok(file) => {
            // Layout::from_size_align requires a power-of-two alignment. Use
            // next_power_of_two so non-standard sector sizes (520, 528 bytes) work.
            let alloc_align = (logical_sector_size as usize).next_power_of_two();
            let aligned_buf = Some(AlignedBuf::new(logical_sector_size as usize, alloc_align));
            return Ok(DeviceReader {
                file,
                size_bytes,
                logical_sector_size,
                aligned_buf,
            });
        }
        Err(nix::Error::EINVAL) => {
            // Device or filesystem does not support O_DIRECT — fall through.
            eprintln!(
                "iridium-device: O_DIRECT not supported for {:?}, \
                 falling back to buffered reads (forensic integrity may be reduced)",
                path
            );
        }
        Err(e) => {
            return Err(DeviceError::Open {
                path: path.to_path_buf(),
                source: e,
            });
        }
    }

    // Fallback: open without O_DIRECT.
    let file = try_open(path, false).map_err(|e| DeviceError::Open {
        path: path.to_path_buf(),
        source: e,
    })?;

    Ok(DeviceReader {
        file,
        size_bytes,
        logical_sector_size,
        aligned_buf: None,
    })
}

fn try_open(path: &Path, direct: bool) -> Result<std::fs::File, nix::Error> {
    let mut flags = libc::O_RDONLY | libc::O_NOATIME;
    if direct {
        flags |= libc::O_DIRECT;
    }
    match open_with_flags(path, flags) {
        Ok(f) => Ok(f),
        // O_NOATIME requires file ownership or CAP_FOWNER; retry without it so
        // non-root callers can still open devices for which they have read access.
        Err(nix::Error::EPERM) | Err(nix::Error::EACCES) => {
            open_with_flags(path, flags & !libc::O_NOATIME)
        }
        Err(e) => Err(e),
    }
}

fn open_with_flags(path: &Path, flags: i32) -> Result<std::fs::File, nix::Error> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(flags)
        .open(path)
        .map_err(|e| {
            e.raw_os_error()
                .map(|raw| nix::Error::from(nix::errno::Errno::from_raw(raw)))
                .unwrap_or(nix::Error::ENODEV)
        })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

#[inline]
fn round_up(n: usize, align: usize) -> usize {
    // Works for any positive `align`, including non-power-of-two sector sizes
    // (e.g. 520- or 528-byte sectors used by some SAS/FC drives).
    n.div_ceil(align) * align
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_up_to_sector() {
        assert_eq!(round_up(0, 512), 0);
        assert_eq!(round_up(1, 512), 512);
        assert_eq!(round_up(512, 512), 512);
        assert_eq!(round_up(513, 512), 1024);
        assert_eq!(round_up(1024, 512), 1024);
    }

    #[test]
    fn aligned_buf_allocates_and_drops() {
        let buf = AlignedBuf::new(4096, 512);
        assert_eq!(buf.len(), 4096);
        drop(buf); // must not crash
    }

    #[test]
    fn aligned_buf_ptr_is_aligned() {
        let buf = AlignedBuf::new(4096, 512);
        assert_eq!(buf.ptr.as_ptr() as usize % 512, 0);
    }
}
