// iridium-ewf: safe Rust wrapper around libewf.
//
// `EwfHandle` owns a `*mut libewf_handle_t` and exposes a typed, panic-free
// API. All libewf errors are captured into `EwfError` and returned as `Result`.

use std::{
    ffi::{CString, c_char},
    path::Path,
};

use iridium_ewf_sys as sys;
use thiserror::Error;

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

// ── Helper: harvest a libewf_error_t into EwfError ───────────────────────────

unsafe fn harvest_error(mut raw: *mut sys::libewf_error_t) -> EwfError {
    if raw.is_null() {
        return EwfError::Library("(no error detail)".into());
    }
    let mut buf = vec![0u8; 512];
    let n = unsafe {
        sys::libewf_error_sprint(raw, buf.as_mut_ptr() as *mut c_char, buf.len())
    };
    let msg = if n > 0 {
        String::from_utf8_lossy(&buf[..n as usize]).trim_end_matches('\0').to_owned()
    } else {
        "(libewf_error_sprint failed)".into()
    };
    unsafe { sys::libewf_error_free(&mut raw) };
    EwfError::Library(msg)
}

// ── EwfHandle ─────────────────────────────────────────────────────────────────

/// An owned libewf handle. Automatically closes and frees on drop.
pub struct EwfHandle {
    inner: *mut sys::libewf_handle_t,
}

// SAFETY: libewf handles are not thread-local; we never share the raw pointer
// across threads without synchronization.
unsafe impl Send for EwfHandle {}

impl EwfHandle {
    // ── Constructor ──────────────────────────────────────────────────────

    /// Allocates a new handle without opening any files.
    pub fn new() -> Result<Self, EwfError> {
        let mut handle: *mut sys::libewf_handle_t = std::ptr::null_mut();
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();

        let rc = unsafe { sys::libewf_handle_initialize(&mut handle, &mut error) };
        if rc != 1 {
            return Err(unsafe { harvest_error(error) });
        }
        Ok(Self { inner: handle })
    }

    // ── Metadata (call before open_write) ────────────────────────────────

    /// Sets the total image size in bytes. Must be called before the first write.
    pub fn set_media_size(&mut self, size: u64) -> Result<(), EwfError> {
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe { sys::libewf_handle_set_media_size(self.inner, size, &mut error) };
        if rc != 1 {
            return Err(unsafe { harvest_error(error) });
        }
        Ok(())
    }

    /// Sets the media type. Use the `LIBEWF_MEDIA_TYPE_*` constants from
    /// `iridium_ewf_sys`.
    pub fn set_media_type(&mut self, media_type: u8) -> Result<(), EwfError> {
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe { sys::libewf_handle_set_media_type(self.inner, media_type, &mut error) };
        if rc != 1 {
            return Err(unsafe { harvest_error(error) });
        }
        Ok(())
    }

    /// Sets the media flags. Use the `LIBEWF_MEDIA_FLAG_*` constants.
    pub fn set_media_flags(&mut self, flags: u8) -> Result<(), EwfError> {
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe { sys::libewf_handle_set_media_flags(self.inner, flags, &mut error) };
        if rc != 1 {
            return Err(unsafe { harvest_error(error) });
        }
        Ok(())
    }

    /// Sets the output format. Use the `LIBEWF_FORMAT_*` constants.
    /// Defaults to EnCase 6 if not called.
    pub fn set_format(&mut self, format: u8) -> Result<(), EwfError> {
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe { sys::libewf_handle_set_format(self.inner, format, &mut error) };
        if rc != 1 {
            return Err(unsafe { harvest_error(error) });
        }
        Ok(())
    }

    /// Sets the bytes-per-sector value (default 512).
    pub fn set_bytes_per_sector(&mut self, bps: u32) -> Result<(), EwfError> {
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc =
            unsafe { sys::libewf_handle_set_bytes_per_sector(self.inner, bps, &mut error) };
        if rc != 1 {
            return Err(unsafe { harvest_error(error) });
        }
        Ok(())
    }

    /// Sets a UTF-8 header value (case number, examiner, description, …).
    ///
    /// `identifier` is a byte-string key such as `b"case_number"` or
    /// `b"examiner_name"`.
    pub fn set_header_value(&mut self, identifier: &[u8], value: &[u8]) -> Result<(), EwfError> {
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
            return Err(unsafe { harvest_error(error) });
        }
        Ok(())
    }

    /// Stores a hash value as a UTF-8 hex string in the EWF metadata.
    ///
    /// `identifier` is `b"MD5"` or `b"SHA1"`.
    /// `hex_digest` is the lowercase hex string.
    pub fn set_hash_value(&mut self, identifier: &[u8], hex_digest: &[u8]) -> Result<(), EwfError> {
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
            return Err(unsafe { harvest_error(error) });
        }
        Ok(())
    }

    /// Embeds the raw 16-byte MD5 digest into the image.
    pub fn set_md5_hash(&mut self, digest: &[u8; 16]) -> Result<(), EwfError> {
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe {
            sys::libewf_handle_set_md5_hash(self.inner, digest.as_ptr(), 16, &mut error)
        };
        if rc != 1 {
            return Err(unsafe { harvest_error(error) });
        }
        Ok(())
    }

    /// Embeds the raw 20-byte SHA-1 digest into the image.
    pub fn set_sha1_hash(&mut self, digest: &[u8; 20]) -> Result<(), EwfError> {
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe {
            sys::libewf_handle_set_sha1_hash(self.inner, digest.as_ptr(), 20, &mut error)
        };
        if rc != 1 {
            return Err(unsafe { harvest_error(error) });
        }
        Ok(())
    }

    // ── Open ─────────────────────────────────────────────────────────────

    /// Opens an EWF file for writing.
    ///
    /// `base_path` is the output filename without extension.
    /// libewf will add `.e01`, `.e02`, … automatically.
    pub fn open_write(&mut self, base_path: &Path) -> Result<(), EwfError> {
        let s = base_path
            .to_str()
            .ok_or_else(|| EwfError::InvalidPath(base_path.display().to_string()))?;
        let c = CString::new(s)
            .map_err(|_| EwfError::InvalidPath(s.to_owned()))?;
        let mut ptr = c.into_raw();

        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe {
            sys::libewf_handle_open(
                self.inner,
                &mut ptr,
                1,
                sys::LIBEWF_OPEN_WRITE,
                &mut error,
            )
        };
        // Reclaim the pointer regardless of outcome to avoid leaking.
        let _ = unsafe { CString::from_raw(ptr) };

        if rc != 1 {
            return Err(unsafe { harvest_error(error) });
        }
        Ok(())
    }

    /// Opens one or more EWF segment files for reading.
    pub fn open_read(&mut self, paths: &[&Path]) -> Result<(), EwfError> {
        let cstrings: Vec<CString> = paths
            .iter()
            .map(|p| {
                let s = p.to_str().ok_or_else(|| EwfError::InvalidPath(p.display().to_string()))?;
                CString::new(s).map_err(|_| EwfError::InvalidPath(s.to_owned()))
            })
            .collect::<Result<_, _>>()?;

        let mut ptrs: Vec<*mut c_char> = cstrings.iter().map(|c| c.as_ptr() as *mut c_char).collect();

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
            return Err(unsafe { harvest_error(error) });
        }
        Ok(())
    }

    // ── I/O ──────────────────────────────────────────────────────────────

    /// Writes a buffer at the current position.
    /// Returns the number of bytes actually written.
    pub fn write_buffer(&mut self, data: &[u8]) -> Result<usize, EwfError> {
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
            return Err(unsafe { harvest_error(error) });
        }
        Ok(n as usize)
    }

    /// Finalises the write. Must be called after the last [`write_buffer`].
    pub fn write_finalize(&mut self) -> Result<(), EwfError> {
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe { sys::libewf_handle_write_finalize(self.inner, &mut error) };
        if rc < 0 {
            return Err(unsafe { harvest_error(error) });
        }
        Ok(())
    }

    /// Reads up to `buf.len()` bytes at the current position.
    /// Returns the number of bytes read (0 = EOF).
    pub fn read_buffer(&mut self, buf: &mut [u8]) -> Result<usize, EwfError> {
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
            return Err(unsafe { harvest_error(error) });
        }
        Ok(n as usize)
    }

    /// Returns the total image size as stored in the EWF metadata.
    pub fn media_size(&mut self) -> Result<u64, EwfError> {
        let mut size: u64 = 0;
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe { sys::libewf_handle_get_media_size(self.inner, &mut size, &mut error) };
        if rc != 1 {
            return Err(unsafe { harvest_error(error) });
        }
        Ok(size)
    }

    /// Returns the stored MD5 hash as 16 raw bytes, or `None` if not set.
    pub fn md5_hash(&mut self) -> Result<Option<[u8; 16]>, EwfError> {
        let mut buf = [0u8; 16];
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe {
            sys::libewf_handle_get_md5_hash(self.inner, buf.as_mut_ptr(), 16, &mut error)
        };
        match rc {
            1 => Ok(Some(buf)),
            0 => Ok(None),
            _ => Err(unsafe { harvest_error(error) }),
        }
    }

    // ── Explicit close ────────────────────────────────────────────────────

    /// Closes the underlying file(s). Also called automatically on `Drop`.
    pub fn close(&mut self) -> Result<(), EwfError> {
        if self.inner.is_null() {
            return Ok(());
        }
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        let rc = unsafe { sys::libewf_handle_close(self.inner, &mut error) };
        if rc != 0 {
            return Err(unsafe { harvest_error(error) });
        }
        Ok(())
    }
}

impl Drop for EwfHandle {
    fn drop(&mut self) {
        if self.inner.is_null() {
            return;
        }
        // Best-effort close; ignore errors on drop.
        let mut error: *mut sys::libewf_error_t = std::ptr::null_mut();
        unsafe { sys::libewf_handle_close(self.inner, &mut error) };
        if !error.is_null() {
            unsafe { sys::libewf_error_free(&mut error) };
        }
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
