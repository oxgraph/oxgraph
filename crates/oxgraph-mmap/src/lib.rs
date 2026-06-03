//! Read-only memory-map shim: the single audited `unsafe` island in the
//! workspace.
//!
//! Memory-mapping a file is inherently `unsafe` in Rust — the kernel can change
//! the mapped bytes under a live `&[u8]` if the file is modified or truncated,
//! which is undefined behavior. `memmap2` therefore exposes only `unsafe fn`
//! constructors for file-backed maps. Every other crate keeps
//! `unsafe_code = "forbid"`; this crate exists so the one unavoidable `unsafe`
//! call lives in exactly one small, audited place behind a safe API, rather than
//! weakening the storage engine's `forbid` posture.
//!
//! # Safety contract for callers
//!
//! The mapped file MUST be immutable for the lifetime of the returned [`Mmap`].
//! The engine only maps published, per-generation base files (`base-{g}.oxgdb`),
//! which are written once via atomic rename and never modified in place or
//! truncated while a reader holds them, so the contract holds by construction.
//!
//! # Performance
//!
//! `perf: unspecified`; mapping is an `O(1)` syscall and pages fault in lazily.

use std::{fs::File, io};

pub use memmap2::Mmap;

/// Maps `file` read-only into memory, returning an [`Mmap`] that dereferences to
/// a byte slice.
///
/// # Safety contract
///
/// The caller MUST guarantee `file` is not modified or truncated for the
/// lifetime of the returned [`Mmap`]. Base files are immutable once published
/// (written once, atomic rename, never rewritten in place), which satisfies
/// this; see the module docs.
///
/// # Errors
///
/// Returns the underlying [`io::Error`] when the map syscall fails.
///
/// # Performance
///
/// This function is `O(1)`: one map syscall; pages fault in on access.
pub fn map_read_only(file: &File) -> io::Result<Mmap> {
    // SAFETY: `memmap2`'s map constructor is unsafe solely because an external
    // modification or truncation of the backing file during the map's lifetime
    // would invalidate the borrowed bytes. The documented caller contract — an
    // immutable, atomically published base file that no process rewrites in
    // place — upholds that invariant, so the mapping stays valid for its whole
    // lifetime.
    #[expect(
        unsafe_code,
        reason = "memmap2 exposes only `unsafe fn` for read-only file maps; this is the workspace's sole audited unsafe site, gated by the documented immutability contract"
    )]
    unsafe {
        Mmap::map(file)
    }
}

#[cfg(test)]
mod tests {
    /// File-backed tests that touch the real filesystem; gated off miri, whose
    /// isolation blocks file IO and whose interpreter cannot run the map syscall.
    #[cfg(not(miri))]
    mod file_backed {
        use std::io::Write;

        use crate::map_read_only;

        /// A read-only map of a freshly written file exposes its exact bytes.
        #[test]
        fn maps_file_bytes() {
            let mut path = std::env::temp_dir();
            path.push(format!("oxgraph-mmap-test-{}.bin", std::process::id()));
            let payload = b"oxgraph base bytes";
            {
                let mut file = std::fs::File::create(&path).expect("create temp file");
                file.write_all(payload).expect("write payload");
                file.sync_all().expect("sync temp file");
            }
            let file = std::fs::File::open(&path).expect("open temp file");
            let map = map_read_only(&file).expect("map temp file");
            assert_eq!(&map[..], payload);
            let _ = std::fs::remove_file(&path);
        }
    }
}
