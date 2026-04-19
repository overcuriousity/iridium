// iridium-ewf: safe Rust wrapper around libewf.
//
// `EwfHandle` owns a `*mut libewf_handle_t` and exposes a typed, panic-free
// API. All libewf errors are captured into `EwfError` and returned as `Result`.

#[rustfmt::skip]
use std::{
    ffi::{CString, c_char},
    marker::PhantomData,
    path::Path,
    sync::Mutex,
};

use iridium_ewf_sys as sys;
use thiserror::Error;

// Re-export the libewf format/media constants so callers do not need to depend
// on iridium-ewf-sys directly.
pub use sys::{
    LIBEWF_FORMAT_ENCASE1, LIBEWF_FORMAT_ENCASE2, LIBEWF_FORMAT_ENCASE3, LIBEWF_FORMAT_ENCASE4,
    LIBEWF_FORMAT_ENCASE5, LIBEWF_FORMAT_ENCASE6, LIBEWF_FORMAT_ENCASE7, LIBEWF_FORMAT_EWF,
    LIBEWF_FORMAT_EWFX, LIBEWF_FORMAT_FTK_IMAGER, LIBEWF_FORMAT_LINEN5, LIBEWF_FORMAT_LINEN6,
    LIBEWF_FORMAT_LINEN7, LIBEWF_FORMAT_SMART, LIBEWF_FORMAT_UNKNOWN, LIBEWF_FORMAT_V2_ENCASE7,
    LIBEWF_MEDIA_FLAG_FASTBLOC, LIBEWF_MEDIA_FLAG_PHYSICAL, LIBEWF_MEDIA_FLAG_TABLEAU,
    LIBEWF_MEDIA_TYPE_FIXED, LIBEWF_MEDIA_TYPE_MEMORY, LIBEWF_MEDIA_TYPE_OPTICAL,
    LIBEWF_MEDIA_TYPE_REMOVABLE, LIBEWF_MEDIA_TYPE_SINGLE_FILES,
};

// ── Global serialization lock ─────────────────────────────────────────────────

// libewf makes no thread-safety guarantees. This lock serializes every libewf
// FFI call process-wide, making `unsafe impl Send for EwfHandle` sound even
// when multiple handles exist on different threads.
static LIBEWF_LOCK: Mutex<()> = Mutex::new(());

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum EwfError {
    #[error("libewf error: {0}")]
    Library(String),

    #[error("invalid path: {0}")]
    InvalidPath(String),

    #[error("null pointer returned by libewf")]
    NullPointer,
}

// ── Lock helper ───────────────────────────────────────────────────────────────

fn lock_libewf() -> std::sync::MutexGuard<'static, ()> {
    LIBEWF_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

// ── Helper: harvest a libewf_error_t into EwfError ───────────────────────────

// Accepting `_guard` proves at the call site that `LIBEWF_LOCK` is held,
// preventing accidental calls outside a locked section.
unsafe fn harvest_error(
    _guard: &std::sync::MutexGuard<'static, ()>,
    mut raw: *mut sys::libewf_error_t,
) -> EwfError {
    if raw.is_null() {
        return EwfError::Library("(no error detail)".into());
    }
    let mut buf = vec![0u8; 512];
    let n = unsafe { sys::libewf_error_sprint(raw, buf.as_mut_ptr() as *mut c_char, buf.len()) };
    let msg = if n > 0 {
        String::from_utf8_lossy(&buf[..n as usize])
            .trim_end_matches('\0')
            .to_owned()
    } else {
        "(libewf_error_sprint failed)".into()
    };
    unsafe { sys::libewf_error_free(&mut raw) };
    EwfError::Library(msg)
}

// ── EwfHandle ─────────────────────────────────────────────────────────────────

/// An owned libewf handle. Automatically closes and frees on drop.
///
/// **Thread safety:** `EwfHandle` is `Send` but `!Sync`.
///
/// A handle may be moved to another thread; it must not be accessed from
/// multiple threads simultaneously (`&mut self` access prevents this at the
/// type level). All libewf FFI calls are serialized process-wide via an
/// internal global lock (`LIBEWF_LOCK`), so multiple handles on different
/// threads cannot race inside libewf even if libewf has global state. This
/// is what makes the `Send` impl sound — the serialization invariant is
/// enforced here, not assumed from caller behaviour.
///
/// `_not_send_sync` (`PhantomData<*mut ()>`) keeps the type `!Sync` and
/// documents the intent explicitly, even if the struct fields are refactored.
pub struct EwfHandle {
    inner: *mut sys::libewf_handle_t,
    /// True after a successful explicit `close()` call. `Drop` checks this
    /// flag to avoid a double-close while still freeing the handle allocation.
    closed: bool,
    _not_send_sync: PhantomData<*mut ()>,
}

// SAFETY: Every libewf FFI call in this type is guarded by LIBEWF_LOCK, which
// serializes all libewf access process-wide. Combined with `&mut self`
// preventing concurrent use of a single handle, no two libewf calls can
// execute simultaneously regardless of how many handles or threads exist.
unsafe impl Send for EwfHandle {}

impl EwfHandle {
    // ── Constructor ──────────────────────────────────────────────────────

    /// Allocates a new handle without opening any files.
    pub fn new() -> Result<Self, EwfError> {
        let _g = lock_libewf();
        let mut handle: *mut sys::libewf_handle_t = std::ptr::null_mut();
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();

        let rc = unsafe { sys::libewf_handle_initialize(&mut handle, &mut error) };
        if rc != 1 {
            return Err(unsafe { harvest_error(&_g, error) });
        }
        if handle.is_null() {
            return Err(EwfError::NullPointer);
        }
        Ok(Self {
            inner: handle,
            closed: false,
            _not_send_sync: PhantomData,
        })
    }

    // ── Metadata (call before open_write) ────────────────────────────────

    /// Sets the total image size in bytes. Must be called before the first write.
    pub fn set_media_size(&mut self, size: u64) -> Result<(), EwfError> {
        let _g = lock_libewf();
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe { sys::libewf_handle_set_media_size(self.inner, size, &mut error) };
        if rc != 1 {
            return Err(unsafe { harvest_error(&_g, error) });
        }
        Ok(())
    }

    /// Sets the media type. Use the `LIBEWF_MEDIA_TYPE_*` constants from
    /// `iridium_ewf_sys`.
    pub fn set_media_type(&mut self, media_type: u8) -> Result<(), EwfError> {
        let _g = lock_libewf();
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe { sys::libewf_handle_set_media_type(self.inner, media_type, &mut error) };
        if rc != 1 {
            return Err(unsafe { harvest_error(&_g, error) });
        }
        Ok(())
    }

    /// Sets the media flags. Use the `LIBEWF_MEDIA_FLAG_*` constants.
    pub fn set_media_flags(&mut self, flags: u8) -> Result<(), EwfError> {
        let _g = lock_libewf();
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe { sys::libewf_handle_set_media_flags(self.inner, flags, &mut error) };
        if rc != 1 {
            return Err(unsafe { harvest_error(&_g, error) });
        }
        Ok(())
    }

    /// Sets the output format. Use the `LIBEWF_FORMAT_*` constants.
    /// Defaults to EnCase 6 if not called.
    pub fn set_format(&mut self, format: u8) -> Result<(), EwfError> {
        let _g = lock_libewf();
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe { sys::libewf_handle_set_format(self.inner, format, &mut error) };
        if rc != 1 {
            return Err(unsafe { harvest_error(&_g, error) });
        }
        Ok(())
    }

    /// Sets the bytes-per-sector value (default 512).
    pub fn set_bytes_per_sector(&mut self, bps: u32) -> Result<(), EwfError> {
        let _g = lock_libewf();
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe { sys::libewf_handle_set_bytes_per_sector(self.inner, bps, &mut error) };
        if rc != 1 {
            return Err(unsafe { harvest_error(&_g, error) });
        }
        Ok(())
    }

    /// Sets a UTF-8 header value (case number, examiner, description, …).
    ///
    /// `identifier` is a byte-string key such as `b"case_number"` or
    /// `b"examiner_name"`.
    pub fn set_header_value(&mut self, identifier: &[u8], value: &[u8]) -> Result<(), EwfError> {
        let _g = lock_libewf();
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe {
            sys::libewf_handle_set_header_value(
                self.inner,
                identifier.as_ptr(),
                identifier.len(),
                value.as_ptr(),
                value.len(),
                &mut error,
            )
        };
        if rc != 1 {
            return Err(unsafe { harvest_error(&_g, error) });
        }
        Ok(())
    }

    /// Stores a hash value as a UTF-8 hex string in the EWF metadata.
    ///
    /// `identifier` is `b"MD5"` or `b"SHA1"`.
    /// `hex_digest` is the lowercase hex string.
    pub fn set_hash_value(&mut self, identifier: &[u8], hex_digest: &[u8]) -> Result<(), EwfError> {
        let _g = lock_libewf();
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe {
            sys::libewf_handle_set_hash_value(
                self.inner,
                identifier.as_ptr(),
                identifier.len(),
                hex_digest.as_ptr(),
                hex_digest.len(),
                &mut error,
            )
        };
        if rc != 1 {
            return Err(unsafe { harvest_error(&_g, error) });
        }
        Ok(())
    }

    /// Embeds the raw 16-byte MD5 digest into the image.
    pub fn set_md5_hash(&mut self, digest: &[u8; 16]) -> Result<(), EwfError> {
        let _g = lock_libewf();
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc =
            unsafe { sys::libewf_handle_set_md5_hash(self.inner, digest.as_ptr(), 16, &mut error) };
        if rc != 1 {
            return Err(unsafe { harvest_error(&_g, error) });
        }
        Ok(())
    }

    /// Embeds the raw 20-byte SHA-1 digest into the image.
    pub fn set_sha1_hash(&mut self, digest: &[u8; 20]) -> Result<(), EwfError> {
        let _g = lock_libewf();
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe {
            sys::libewf_handle_set_sha1_hash(self.inner, digest.as_ptr(), 20, &mut error)
        };
        if rc != 1 {
            return Err(unsafe { harvest_error(&_g, error) });
        }
        Ok(())
    }

    // ── Open ─────────────────────────────────────────────────────────────

    /// Opens an EWF file for writing.
    ///
    /// `base_path` is the output filename **without** extension.
    /// libewf appends the format-appropriate extension automatically
    /// (e.g. `image.E01`, `image.E02`, … for EnCase formats).
    pub fn open_write(&mut self, base_path: &Path) -> Result<(), EwfError> {
        let s = base_path
            .to_str()
            .ok_or_else(|| EwfError::InvalidPath(base_path.display().to_string()))?;
        let c = CString::new(s).map_err(|_| EwfError::InvalidPath(s.to_owned()))?;
        // SAFETY: `c` remains alive for the duration of the FFI call.
        // The C signature takes `*mut *mut c_char` but libewf only reads the
        // filename string — it does not modify the pointer value or the string
        // contents. The `*mut` cast is required to match the C signature.
        let mut ptr = c.as_ptr() as *mut c_char;

        let _g = lock_libewf();
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe {
            sys::libewf_handle_open(self.inner, &mut ptr, 1, sys::LIBEWF_OPEN_WRITE, &mut error)
        };
        // `c` is dropped here after the call.

        if rc != 1 {
            return Err(unsafe { harvest_error(&_g, error) });
        }
        Ok(())
    }

    /// Opens one or more EWF segment files for reading.
    pub fn open_read(&mut self, paths: &[&Path]) -> Result<(), EwfError> {
        let cstrings: Vec<CString> = paths
            .iter()
            .map(|p| {
                let s = p
                    .to_str()
                    .ok_or_else(|| EwfError::InvalidPath(p.display().to_string()))?;
                CString::new(s).map_err(|_| EwfError::InvalidPath(s.to_owned()))
            })
            .collect::<Result<_, _>>()?;

        // SAFETY: `cstrings` remains alive for the duration of the FFI call,
        // keeping each pointer valid. The C API signature uses `*mut *mut c_char`
        // but libewf only reads the filenames — it does not write through the
        // pointers. The `*mut` cast is required to match the C signature.
        let mut ptrs: Vec<*mut c_char> =
            cstrings.iter().map(|c| c.as_ptr() as *mut c_char).collect();

        let _g = lock_libewf();
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe {
            sys::libewf_handle_open(
                self.inner,
                ptrs.as_mut_ptr(),
                ptrs.len() as i32,
                sys::LIBEWF_OPEN_READ,
                &mut error,
            )
        };
        if rc != 1 {
            return Err(unsafe { harvest_error(&_g, error) });
        }
        Ok(())
    }

    // ── I/O ──────────────────────────────────────────────────────────────

    /// Writes a buffer at the current position.
    /// Returns the number of bytes actually written.
    pub fn write_buffer(&mut self, data: &[u8]) -> Result<usize, EwfError> {
        let _g = lock_libewf();
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let n = unsafe {
            sys::libewf_handle_write_buffer(
                self.inner,
                data.as_ptr() as *const _,
                data.len(),
                &mut error,
            )
        };
        if n < 0 {
            return Err(unsafe { harvest_error(&_g, error) });
        }
        Ok(n as usize)
    }

    /// Finalises the write. Must be called after the last [`write_buffer`].
    pub fn write_finalize(&mut self) -> Result<(), EwfError> {
        let _g = lock_libewf();
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe { sys::libewf_handle_write_finalize(self.inner, &mut error) };
        if rc < 0 {
            return Err(unsafe { harvest_error(&_g, error) });
        }
        Ok(())
    }

    /// Reads up to `buf.len()` bytes at the current position.
    /// Returns the number of bytes read (0 = EOF).
    pub fn read_buffer(&mut self, buf: &mut [u8]) -> Result<usize, EwfError> {
        let _g = lock_libewf();
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let n = unsafe {
            sys::libewf_handle_read_buffer(
                self.inner,
                buf.as_mut_ptr() as *mut _,
                buf.len(),
                &mut error,
            )
        };
        if n < 0 {
            return Err(unsafe { harvest_error(&_g, error) });
        }
        Ok(n as usize)
    }

    /// Returns the total image size as stored in the EWF metadata.
    pub fn media_size(&mut self) -> Result<u64, EwfError> {
        let _g = lock_libewf();
        let mut size: u64 = 0;
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe { sys::libewf_handle_get_media_size(self.inner, &mut size, &mut error) };
        if rc != 1 {
            return Err(unsafe { harvest_error(&_g, error) });
        }
        Ok(size)
    }

    /// Returns the stored MD5 hash as 16 raw bytes, or `None` if not set.
    pub fn md5_hash(&mut self) -> Result<Option<[u8; 16]>, EwfError> {
        let _g = lock_libewf();
        let mut buf = [0u8; 16];
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe {
            sys::libewf_handle_get_md5_hash(self.inner, buf.as_mut_ptr(), 16, &mut error)
        };
        match rc {
            1 => Ok(Some(buf)),
            0 => Ok(None),
            _ => Err(unsafe { harvest_error(&_g, error) }),
        }
    }

    // ── Explicit close ────────────────────────────────────────────────────

    /// Closes the underlying file(s). Also called automatically on `Drop`.
    ///
    /// Sets `closed` to `true` after a successful close so that `Drop` does not
    /// attempt a second `libewf_handle_close`. libewf has process-global state
    /// and a double-close corrupts it. `inner` is kept non-null so `Drop` can
    /// still call `libewf_handle_free` to release the allocation.
    pub fn close(&mut self) -> Result<(), EwfError> {
        if self.inner.is_null() || self.closed {
            return Ok(());
        }
        let _g = lock_libewf();
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe { sys::libewf_handle_close(self.inner, &mut error) };
        if rc != 0 {
            return Err(unsafe { harvest_error(&_g, error) });
        }
        self.closed = true;
        Ok(())
    }
}

impl Drop for EwfHandle {
    fn drop(&mut self) {
        if self.inner.is_null() {
            return;
        }
        let _g = lock_libewf();
        if !self.closed {
            // Best-effort close; ignore errors on drop.
            let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
            unsafe { sys::libewf_handle_close(self.inner, &mut error) };
            if !error.is_null() {
                unsafe { sys::libewf_error_free(&mut error) };
            }
        }
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        unsafe { sys::libewf_handle_free(&mut self.inner, &mut error) };
        if !error.is_null() {
            unsafe { sys::libewf_error_free(&mut error) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_new_and_drop() {
        // Allocating and immediately dropping a handle must not crash or leak.
        let h = EwfHandle::new().expect("EwfHandle::new failed");
        drop(h);
    }
}
