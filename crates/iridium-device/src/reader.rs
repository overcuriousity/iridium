// reader.rs — O_DIRECT forensic read-only device access with internal alignment.

use std::{
    alloc::{Layout, alloc, dealloc},
    path::Path,
    ptr::NonNull,
};

use std::os::unix::fs::FileExt;

use crate::{DeviceError, Disk};

/// Minimum buffer alignment for O_DIRECT: one memory page.
/// Stricter than the bare-minimum (logical sector size) but unambiguous,
/// architecture-safe, and the standard choice in forensic tooling.
const PAGE_SIZE: usize = 4096;

// ── Aligned buffer ────────────────────────────────────────────────────────────

/// A heap-allocated buffer whose base address is aligned to `align` bytes.
///
/// For O_DIRECT, the kernel requires buffer alignment to at least the logical
/// sector size. iridium uses `max(logical_sector_size, PAGE_SIZE)` — page
/// alignment (≥4096 bytes) is stricter than strictly necessary but is the
/// most defensible choice for forensic use: it is unambiguous in documentation,
/// satisfies every architecture's O_DIRECT requirement, and eliminates any risk
/// of an EINVAL from under-alignment.
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

                // Grow the scratch buffer if needed, keeping the same
                // max(sector, PAGE_SIZE) alignment invariant from open_inner.
                if ab.len() < aligned_len {
                    *ab = AlignedBuf::new(aligned_len, sector.max(PAGE_SIZE));
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
    // O_DIRECT requires the buffer address to be aligned to the logical sector
    // size. Layout::from_size_align demands a power-of-two alignment, so O_DIRECT
    // is only safe when the sector size itself is a power of two. All mainstream
    // devices (512-byte, 4096-byte) satisfy this; exotic sizes (520, 528) fall
    // through to buffered reads.
    if logical_sector_size.is_power_of_two() {
        match try_open(path, true) {
            Ok(file) => {
                // Align to max(logical_sector_size, PAGE_SIZE) for forensic soundness.
                // Both values are powers of two, so max is also a valid Layout alignment.
                let sz = logical_sector_size as usize;
                let align = sz.max(PAGE_SIZE);
                let aligned_buf = Some(AlignedBuf::new(sz, align));
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
    } else {
        eprintln!(
            "iridium-device: non-power-of-two sector size {} for {:?}, \
             O_DIRECT not safe — using buffered reads",
            logical_sector_size, path
        );
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

    // ── O_DIRECT alignment math ───────────────────────────────────────────────

    /// Compute (aligned_offset, prefix, aligned_len) for a given read request,
    /// matching the logic in DeviceReader::read_at.
    fn odirect_params(offset: u64, max_len: usize, sector: usize) -> (u64, usize, usize) {
        let sector_u64 = sector as u64;
        let aligned_offset = (offset / sector_u64) * sector_u64;
        let prefix = (offset - aligned_offset) as usize;
        let aligned_len = round_up(prefix + max_len, sector);
        (aligned_offset, prefix, aligned_len)
    }

    #[test]
    fn odirect_aligned_offset_already_aligned() {
        // offset already on a sector boundary → no prefix, no extra span
        let (ao, prefix, len) = odirect_params(512, 512, 512);
        assert_eq!(ao, 512);
        assert_eq!(prefix, 0);
        assert_eq!(len, 512);
    }

    #[test]
    fn odirect_unaligned_offset_mid_sector() {
        // offset = 768 (512 + 256), sector = 512
        // aligned_offset = 512, prefix = 256, span = 256 + 512 = 768 → rounds to 1024
        let (ao, prefix, len) = odirect_params(768, 512, 512);
        assert_eq!(ao, 512);
        assert_eq!(prefix, 256);
        assert_eq!(len, 1024);
    }

    #[test]
    fn odirect_read_crosses_two_sectors() {
        // offset = 256, reading 512 bytes → spans byte 256..768 → needs sectors 0 and 1
        let (ao, prefix, len) = odirect_params(256, 512, 512);
        assert_eq!(ao, 0);
        assert_eq!(prefix, 256);
        assert_eq!(len, 1024); // must cover both sectors
    }

    #[test]
    fn odirect_small_read_within_one_sector() {
        // offset = 100, read 10 bytes → all within first sector
        let (ao, prefix, len) = odirect_params(100, 10, 512);
        assert_eq!(ao, 0);
        assert_eq!(prefix, 100);
        assert_eq!(len, 512);
    }

    #[test]
    fn odirect_4k_sector() {
        // 4096-byte sectors, unaligned offset = 5000
        // aligned_offset = 4096, prefix = 904, span = 904+1000 = 1904 → one sector
        let (ao, prefix, len) = odirect_params(5000, 1000, 4096);
        assert_eq!(ao, 4096);
        assert_eq!(prefix, 5000 - 4096);
        assert_eq!(len, 4096);
    }

    #[test]
    fn odirect_4k_sector_crosses_boundary() {
        // offset = 5000, reading 3500 bytes → 5000+3500 = 8500 → crosses into third sector
        // prefix = 904, span = 904+3500 = 4404 → rounds up to 8192
        let (ao, prefix, len) = odirect_params(5000, 3500, 4096);
        assert_eq!(ao, 4096);
        assert_eq!(prefix, 904);
        assert_eq!(len, 4096 * 2);
    }
}
