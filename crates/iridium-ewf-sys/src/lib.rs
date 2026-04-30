// iridium-ewf-sys: raw C FFI declarations for libewf.
//
// These bindings are written by hand against the libewf 20240506 public API
// (vendor/libewf/include/libewf.h.in).  They cover the handle lifecycle,
// write path, metadata, and error reporting — sufficient for Phases 1–4.
//
// See vendor/libewf/libewf/libewf_definitions.h.in for all constant values.

#![allow(non_camel_case_types)]

use std::ffi::{c_char, c_int, c_void};

// ── Opaque types ──────────────────────────────────────────────────────────────

/// Opaque libewf handle. Always used through a pointer-to-pointer.
#[repr(C)]
pub struct libewf_handle_t {
    _private: [u8; 0],
}

/// Opaque libewf error object. Free with [`libewf_error_free`].
#[repr(C)]
pub struct libewf_error_t {
    _private: [u8; 0],
}

// ── Access flags (libewf_definitions.h.in: LIBEWF_ACCESS_FLAGS) ──────────────

pub const LIBEWF_ACCESS_FLAG_READ: c_int = 0x01;
pub const LIBEWF_ACCESS_FLAG_WRITE: c_int = 0x02;
pub const LIBEWF_ACCESS_FLAG_RESUME: c_int = 0x10;

pub const LIBEWF_OPEN_READ: c_int = LIBEWF_ACCESS_FLAG_READ;
pub const LIBEWF_OPEN_WRITE: c_int = LIBEWF_ACCESS_FLAG_WRITE;
pub const LIBEWF_OPEN_WRITE_RESUME: c_int = LIBEWF_ACCESS_FLAG_WRITE | LIBEWF_ACCESS_FLAG_RESUME;

// ── Format constants (LIBEWF_FORMAT) ─────────────────────────────────────────

pub const LIBEWF_FORMAT_UNKNOWN: u8 = 0x00;
pub const LIBEWF_FORMAT_ENCASE1: u8 = 0x01;
pub const LIBEWF_FORMAT_ENCASE2: u8 = 0x02;
pub const LIBEWF_FORMAT_ENCASE3: u8 = 0x03;
pub const LIBEWF_FORMAT_ENCASE4: u8 = 0x04;
pub const LIBEWF_FORMAT_ENCASE5: u8 = 0x05;
pub const LIBEWF_FORMAT_ENCASE6: u8 = 0x06;
pub const LIBEWF_FORMAT_ENCASE7: u8 = 0x07;
pub const LIBEWF_FORMAT_SMART: u8 = 0x0e;
pub const LIBEWF_FORMAT_FTK_IMAGER: u8 = 0x0f;
pub const LIBEWF_FORMAT_LINEN5: u8 = 0x25;
pub const LIBEWF_FORMAT_LINEN6: u8 = 0x26;
pub const LIBEWF_FORMAT_LINEN7: u8 = 0x27;
pub const LIBEWF_FORMAT_V2_ENCASE7: u8 = 0x37;
pub const LIBEWF_FORMAT_EWF: u8 = 0x70;
pub const LIBEWF_FORMAT_EWFX: u8 = 0x71;

// ── Media types (LIBEWF_MEDIA_TYPES) ─────────────────────────────────────────

pub const LIBEWF_MEDIA_TYPE_REMOVABLE: u8 = 0x00;
pub const LIBEWF_MEDIA_TYPE_FIXED: u8 = 0x01;
pub const LIBEWF_MEDIA_TYPE_OPTICAL: u8 = 0x03;
pub const LIBEWF_MEDIA_TYPE_SINGLE_FILES: u8 = 0x0e;
pub const LIBEWF_MEDIA_TYPE_MEMORY: u8 = 0x10;

// ── Media flags (LIBEWF_MEDIA_FLAGS) ─────────────────────────────────────────

pub const LIBEWF_MEDIA_FLAG_PHYSICAL: u8 = 0x02;
pub const LIBEWF_MEDIA_FLAG_FASTBLOC: u8 = 0x04;
pub const LIBEWF_MEDIA_FLAG_TABLEAU: u8 = 0x08;

// ── Compression methods / levels ─────────────────────────────────────────────

pub const LIBEWF_COMPRESSION_METHOD_NONE: u16 = 0;
pub const LIBEWF_COMPRESSION_METHOD_DEFLATE: u16 = 1;
pub const LIBEWF_COMPRESSION_METHOD_BZIP2: u16 = 2;

pub const LIBEWF_COMPRESSION_LEVEL_DEFAULT: i8 = -1;
pub const LIBEWF_COMPRESSION_LEVEL_NONE: i8 = 0;
pub const LIBEWF_COMPRESSION_LEVEL_FAST: i8 = 1;
pub const LIBEWF_COMPRESSION_LEVEL_BEST: i8 = 2;

// ── Foreign function declarations ─────────────────────────────────────────────

#[link(name = "ewf")]
unsafe extern "C" {
    // ── Support ────────────────────────────────────────────────────────────

    /// Returns the library version string (e.g. `"20240506"`).
    pub fn libewf_get_version() -> *const c_char;

    // ── Error ──────────────────────────────────────────────────────────────

    /// Frees an error object. Sets `*error` to NULL.
    pub fn libewf_error_free(error: *mut *mut libewf_error_t);

    /// Prints a human-readable error message to `string` (null-terminated).
    /// Returns the number of characters written, or -1 on error.
    pub fn libewf_error_sprint(
        error: *mut libewf_error_t,
        string: *mut c_char,
        size: usize,
    ) -> c_int;

    // ── Handle lifecycle ───────────────────────────────────────────────────

    /// Allocates and initialises a new handle. `*handle` must be NULL.
    /// Returns 1 on success, -1 on error.
    pub fn libewf_handle_initialize(
        handle: *mut *mut libewf_handle_t,
        error: *mut *mut libewf_error_t,
    ) -> c_int;

    /// Frees a handle. Sets `*handle` to NULL.
    /// Returns 1 on success, -1 on error.
    pub fn libewf_handle_free(
        handle: *mut *mut libewf_handle_t,
        error: *mut *mut libewf_error_t,
    ) -> c_int;

    // ── Open / close ───────────────────────────────────────────────────────

    /// Opens EWF segment file(s).
    ///
    /// For reading: `filenames` lists all segment files (e.g. `["image.E01"]`).
    /// For writing: `filenames[0]` is the base name without extension; libewf
    /// appends the format-appropriate extension automatically (e.g. `.E01` for
    /// EnCase formats, `.s01` for SMART/EWF).
    ///
    /// Returns 1 on success, -1 on error.
    pub fn libewf_handle_open(
        handle: *mut libewf_handle_t,
        filenames: *mut *mut c_char,
        number_of_filenames: c_int,
        access_flags: c_int,
        error: *mut *mut libewf_error_t,
    ) -> c_int;

    /// Closes the handle. Returns 0 on success, -1 on error.
    pub fn libewf_handle_close(
        handle: *mut libewf_handle_t,
        error: *mut *mut libewf_error_t,
    ) -> c_int;

    // ── Write path ─────────────────────────────────────────────────────────

    /// Writes `buffer_size` bytes at the current write offset.
    /// Returns the number of bytes written, 0 when no more data can be
    /// written, or -1 on error.
    pub fn libewf_handle_write_buffer(
        handle: *mut libewf_handle_t,
        buffer: *const c_void,
        buffer_size: usize,
        error: *mut *mut libewf_error_t,
    ) -> isize;

    /// Finalises the write (corrects metadata in segment files).
    /// Required after streaming writes.
    /// Returns the number of bytes written or -1 on error.
    pub fn libewf_handle_write_finalize(
        handle: *mut libewf_handle_t,
        error: *mut *mut libewf_error_t,
    ) -> isize;

    // ── Metadata setters ───────────────────────────────────────────────────

    /// Sets the total image size (bytes). Must be called before any writes.
    /// Returns 1 on success, -1 on error.
    pub fn libewf_handle_set_media_size(
        handle: *mut libewf_handle_t,
        media_size: u64,
        error: *mut *mut libewf_error_t,
    ) -> c_int;

    /// Sets the media type (e.g. [`LIBEWF_MEDIA_TYPE_FIXED`]).
    /// Returns 1 on success, -1 on error.
    pub fn libewf_handle_set_media_type(
        handle: *mut libewf_handle_t,
        media_type: u8,
        error: *mut *mut libewf_error_t,
    ) -> c_int;

    /// Sets the media flags (e.g. [`LIBEWF_MEDIA_FLAG_PHYSICAL`]).
    /// Returns 1 on success, -1 on error.
    pub fn libewf_handle_set_media_flags(
        handle: *mut libewf_handle_t,
        media_flags: u8,
        error: *mut *mut libewf_error_t,
    ) -> c_int;

    /// Sets the output format (e.g. [`LIBEWF_FORMAT_ENCASE6`]).
    /// Returns 1 on success, -1 on error.
    pub fn libewf_handle_set_format(
        handle: *mut libewf_handle_t,
        format: u8,
        error: *mut *mut libewf_error_t,
    ) -> c_int;

    /// Sets bytes per sector (default is 512). Must be called before writes.
    /// Returns 1 on success, -1 on error.
    pub fn libewf_handle_set_bytes_per_sector(
        handle: *mut libewf_handle_t,
        bytes_per_sector: u32,
        error: *mut *mut libewf_error_t,
    ) -> c_int;

    /// Sets a UTF-8 header value (case number, examiner name, description…).
    ///
    /// `identifier` examples: `b"case_number"`, `b"examiner_name"`,
    /// `b"description"`, `b"evidence_number"`, `b"notes"`.
    ///
    /// Returns 1 on success, -1 on error.
    pub fn libewf_handle_set_header_value(
        handle: *mut libewf_handle_t,
        identifier: *const u8,
        identifier_length: usize,
        utf8_string: *const u8,
        utf8_string_length: usize,
        error: *mut *mut libewf_error_t,
    ) -> c_int;

    /// Sets a UTF-8 hash value by name (e.g. `b"MD5"`, `b"SHA1"`).
    /// Stores the hash as a hex string inside the EWF metadata.
    /// Returns 1 on success, -1 on error.
    pub fn libewf_handle_set_hash_value(
        handle: *mut libewf_handle_t,
        identifier: *const u8,
        identifier_length: usize,
        utf8_string: *const u8,
        utf8_string_length: usize,
        error: *mut *mut libewf_error_t,
    ) -> c_int;

    /// Embeds the raw 16-byte MD5 digest into the EWF image.
    /// Returns 1 on success, -1 on error.
    pub fn libewf_handle_set_md5_hash(
        handle: *mut libewf_handle_t,
        md5_hash: *const u8,
        size: usize,
        error: *mut *mut libewf_error_t,
    ) -> c_int;

    /// Embeds the raw 20-byte SHA-1 digest into the EWF image.
    /// Returns 1 on success, -1 on error.
    pub fn libewf_handle_set_sha1_hash(
        handle: *mut libewf_handle_t,
        sha1_hash: *const u8,
        size: usize,
        error: *mut *mut libewf_error_t,
    ) -> c_int;

    // ── Read path ──────────────────────────────────────────────────────────

    /// Reads up to `buffer_size` bytes into `buffer` at the current offset.
    /// Returns the number of bytes read, 0 at EOF, or -1 on error.
    pub fn libewf_handle_read_buffer(
        handle: *mut libewf_handle_t,
        buffer: *mut c_void,
        buffer_size: usize,
        error: *mut *mut libewf_error_t,
    ) -> isize;

    /// Returns the total image size in bytes via `media_size`.
    /// Returns 1 on success, -1 on error.
    pub fn libewf_handle_get_media_size(
        handle: *mut libewf_handle_t,
        media_size: *mut u64,
        error: *mut *mut libewf_error_t,
    ) -> c_int;

    /// Retrieves the stored MD5 hash (16 bytes) into `md5_hash`.
    /// Returns 1 if set, 0 if not set, -1 on error.
    pub fn libewf_handle_get_md5_hash(
        handle: *mut libewf_handle_t,
        md5_hash: *mut u8,
        size: usize,
        error: *mut *mut libewf_error_t,
    ) -> c_int;
}

/// Returns the libewf library version string (e.g. `"20240506"`).
///
/// The returned string points to a static C literal; it is valid for the
/// lifetime of the process and never needs to be freed.
pub fn libewf_version() -> &'static str {
    // SAFETY: libewf_get_version returns a pointer to a static, null-terminated
    // C string literal that is valid for the entire process lifetime.
    let ptr = unsafe { libewf_get_version() };
    if ptr.is_null() {
        return "";
    }
    unsafe { std::ffi::CStr::from_ptr(ptr) }
        .to_str()
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    #[test]
    fn version_is_non_empty() {
        // libewf_get_version() must return a non-null, non-empty string.
        let ver = unsafe { libewf_get_version() };
        assert!(!ver.is_null(), "libewf_get_version returned NULL");
        let s = unsafe { CStr::from_ptr(ver) }
            .to_str()
            .expect("version not UTF-8");
        assert!(!s.is_empty(), "libewf_get_version returned empty string");
    }
}
