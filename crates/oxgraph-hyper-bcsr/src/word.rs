//! Borrowed-section word abstraction for bipartite-CSR payloads.

use zerocopy::byteorder::{LE, U32};

/// Integer word usable in borrowed bipartite-CSR sections.
///
/// In-memory test fixtures and examples use `u32`; snapshot-backed views read
/// the same logical values through unaligned little-endian [`U32<LE>`] words
/// without copying.
///
/// `BcsrWord` is a stable public facade for this crate. If a future
/// `oxgraph-csr-core` crate ever generalises the word abstraction, that crate
/// will provide the underlying trait and `BcsrWord` will delegate to it
/// without breaking downstream code.
///
/// # Performance
///
/// Reading a word is expected to be `O(1)`.
pub trait BcsrWord: Copy {
    /// Returns this bipartite-CSR word as a host-endian `u32`.
    ///
    /// # Performance
    ///
    /// This method is `O(1)`.
    fn get(self) -> u32;
}

impl BcsrWord for u32 {
    fn get(self) -> u32 {
        self
    }
}

impl BcsrWord for U32<LE> {
    fn get(self) -> u32 {
        Self::get(self)
    }
}
