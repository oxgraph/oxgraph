//! OXGDB delta-log (write-ahead log) and superblock IO.
//!
//! This module owns the append-only commit log and the superblock manifest of
//! the immutable-base + delta-log architecture. A commit is encoded by
//! [`encode_commit`] into one self-describing record (a [`LogRecordHeader`]
//! followed by its [`MutationOp`]s and an optional UTF-8 blob), appended to the
//! per-generation `delta-{g}.log` by [`append_commit`], and recovered by
//! [`replay`], which walks records strictly by `len`, validating magic, base
//! generation, framing, CRC, and ascending LSN.
//!
//! The superblock (`super.oxgdb`) is the store's single linearization point and
//! is published crash-atomically with the same temp + fsync + rename +
//! directory-fsync sequence as the whole-store writer; it is read and verified
//! by [`read_superblock`] and written by [`write_superblock`].
//!
//! Recovery distinguishes two failure modes. A torn TAIL — the failing record
//! is the last thing in the buffer with no fully valid record after it — is
//! truncated silently; the valid prefix is the durable frontier. Any failure
//! with a well-formed record after it is a loud [`StorageError::LogCorrupt`] or
//! [`StorageError::BaseGenerationMismatch`].
//!
//! # Performance
//!
//! `perf: unspecified`; this module defines log/superblock IO and the `O(log
//! bytes)` replay walk.

use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

use zerocopy::{FromBytes, IntoBytes};

use crate::{
    StorageError,
    crc::{checksum, checksum_append},
    wire::{
        LogRecordHeader, MutationOp, OXGDB_FORMAT_VERSION, OXGLOGR, SUPERBLOCK_CRC_PREFIX_LEN,
        SUPERBLOCK_MAGIC, SuperblockRecord,
    },
};

/// Superblock filename in a database root.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
pub(crate) const SUPERBLOCK_FILE: &str = "super.oxgdb";

/// Temporary superblock filename used for crash-atomic replacement.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
const SUPERBLOCK_TEMP_FILE: &str = "super.oxgdb.tmp";

/// One decoded, owned delta-log commit record.
///
/// # Performance
///
/// Cloning is `O(op count + blob length)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommitFrame {
    /// Log sequence number of this commit.
    pub(crate) lsn: u64,
    /// Writer transaction id that produced this commit.
    pub(crate) txn_id: u64,
    /// Base generation this commit applies over.
    pub(crate) base_generation: u64,
    /// Decoded mutation ops in record order.
    pub(crate) ops: Vec<MutationOp>,
    /// Owned UTF-8 blob holding the record's variable-length bytes.
    pub(crate) blob: Vec<u8>,
}

/// Result of replaying a delta-log buffer: the maximal valid prefix of frames,
/// the byte length that prefix occupies (the durable frontier), and the LSN and
/// txn id of the last recovered frame.
///
/// This is the raw recovered prefix only. Deriving the live `commit_seq` (and the
/// rest of the linearization story the [`SuperblockRecord`] doc promises is
/// "derived from the valid prefix of the delta-log") from these frames is the
/// caller's responsibility in the commit/recovery path; `ReplayOutcome`
/// deliberately exposes the recovered frames and frontier rather than computing
/// it here.
///
/// # Performance
///
/// Cloning is `O(total frame size)`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReplayOutcome {
    /// Recovered frames in ascending LSN order.
    pub(crate) frames: Vec<CommitFrame>,
    /// Byte length of the valid prefix; bytes after this are torn-tail garbage.
    pub(crate) valid_len: usize,
    /// LSN of the last recovered frame, or `0` when none were recovered.
    pub(crate) last_lsn: u64,
    /// Txn id of the last recovered frame, or `0` when none were recovered.
    pub(crate) last_txn: u64,
}

/// Byte length of a [`LogRecordHeader`].
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
const HEADER_LEN: usize = size_of::<LogRecordHeader>();

/// Byte length of one [`MutationOp`].
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
const OP_LEN: usize = size_of::<MutationOp>();

/// Byte offset of a header's `crc32c` field; the CRC covers every record byte
/// except the four bytes starting here.
///
/// # Performance
///
/// `perf: unspecified`; this is a compile-time constant.
const HEADER_CRC_OFFSET: usize = HEADER_LEN - size_of::<u32>();

/// Computes the CRC-32C a record stores in its `crc32c` field: over the whole
/// record (header + ops + blob) except the four CRC-field bytes. `record` must
/// be the fully assembled bytes with the CRC field present (its value is
/// ignored). The covered region is the header prefix before the CRC field
/// followed by everything after it; the two non-contiguous slices are chained
/// through [`checksum_append`] so no joined buffer is allocated on the replay
/// data path.
///
/// # Performance
///
/// This function is `O(record.len())` and allocation-free.
fn record_crc(record: &[u8]) -> u32 {
    let prefix = &record[..HEADER_CRC_OFFSET];
    let suffix = &record[HEADER_CRC_OFFSET + size_of::<u32>()..];
    checksum_append(checksum(prefix), suffix)
}

/// Encodes one commit into a self-describing delta-log record: a
/// [`LogRecordHeader`] (with `len` and `crc32c` set), the `ops`, then the
/// `blob`. The CRC covers the whole record except its own four bytes.
///
/// # Errors
///
/// Returns [`StorageError::InvalidStore`] when the assembled record length or the op
/// count exceeds `u32::MAX`, the `u32` widths the header reserves for them
/// (mirroring the base format's `u32` section widths). A record this large is
/// not representable in the on-disk framing.
///
/// # Performance
///
/// This function is `O(ops.len() + blob.len())`.
pub(crate) fn encode_commit(
    lsn: u64,
    txn_id: u64,
    base_generation: u64,
    ops: &[MutationOp],
    blob: &[u8],
) -> Result<Vec<u8>, StorageError> {
    let len = HEADER_LEN + ops.len() * OP_LEN + blob.len();
    // `len` and `op_count` are stored in the header's `u32` fields; a record or
    // op count exceeding `u32::MAX` is not representable in the on-disk framing,
    // so reject it instead of truncating (mirroring the base format's `u32`
    // section widths).
    let len_u32 = u32::try_from(len)
        .map_err(|_overflow| StorageError::invalid_store("delta-log record too large"))?;
    let op_count_u32 = u32::try_from(ops.len())
        .map_err(|_overflow| StorageError::invalid_store("delta-log op count too large"))?;
    let header = LogRecordHeader {
        base_generation: base_generation.into(),
        lsn: lsn.into(),
        txn_id: txn_id.into(),
        magic: OXGLOGR.into(),
        len: len_u32.into(),
        op_count: op_count_u32.into(),
        crc32c: 0u32.into(),
    };
    let mut record = Vec::with_capacity(len);
    record.extend_from_slice(header.as_bytes());
    for op in ops {
        record.extend_from_slice(op.as_bytes());
    }
    record.extend_from_slice(blob);
    let crc = record_crc(&record);
    record[HEADER_CRC_OFFSET..HEADER_CRC_OFFSET + size_of::<u32>()]
        .copy_from_slice(&crc.to_le_bytes());
    Ok(record)
}

/// Outcome of parsing one delta-log record at the head of a buffer.
///
/// # Performance
///
/// Cloning is `O(frame size)` for the [`Self::Frame`] variant; the others are
/// `O(1)`.
enum RecordParse {
    /// The record is a torn tail: replay stops silently and keeps the prefix.
    Stop,
    /// The record is loudly corrupt; the wrapped error is returned to the caller.
    Loud(StorageError),
    /// A fully valid record decoded into `frame` occupying `len` bytes.
    Frame(CommitFrame, usize),
}

/// State the LSN-ascending check needs from the already-recovered prefix: the
/// last accepted LSN, and whether any record has been accepted yet.
///
/// # Performance
///
/// Copying is `O(1)`.
#[derive(Clone, Copy)]
struct LsnCursor {
    /// LSN of the last accepted record; meaningful only when `seen` is true.
    last_lsn: u64,
    /// Whether any record has been accepted yet.
    seen: bool,
}

/// Decodes the `op_count` [`MutationOp`]s and the trailing blob of a validated
/// record into owned vectors. `record` is the exact record bytes, `framed_min`
/// the header + ops byte length (blob start), and `op_count` the declared op
/// count. A chunk that fails zerocopy parsing stops op decoding early; it cannot
/// occur for `OP_LEN`-sized chunks but is handled without panicking.
///
/// # Performance
///
/// This function is `O(record.len())`.
fn decode_ops(record: &[u8], framed_min: usize, op_count: usize) -> (Vec<MutationOp>, Vec<u8>) {
    let ops_bytes = &record[HEADER_LEN..framed_min];
    let mut ops = Vec::with_capacity(op_count);
    for chunk in ops_bytes.chunks_exact(OP_LEN) {
        let Ok(op) = MutationOp::read_from_bytes(chunk) else {
            break;
        };
        ops.push(op);
    }
    let blob = record[framed_min..].to_vec();
    (ops, blob)
}

/// Resolves a failed validation check: a torn-tail failure
/// ([`RecordParse::Stop`]) when nothing fully valid can follow this record
/// (`is_last`), otherwise loud corruption carrying `error`
/// ([`RecordParse::Loud`]).
///
/// # Performance
///
/// This function is `O(1)`.
fn tail_or_loud(is_last: bool, error: StorageError) -> RecordParse {
    if is_last {
        RecordParse::Stop
    } else {
        RecordParse::Loud(error)
    }
}

/// Parses the single record at the head of `remaining`, classifying it as a torn
/// tail ([`RecordParse::Stop`]), loud corruption ([`RecordParse::Loud`]), or a
/// valid [`RecordParse::Frame`]. `remaining` is the buffer slice starting at the
/// record; because it runs to the end of the whole log, "nothing fully valid can
/// follow this record" is exactly `len >= remaining.len()`, which (with framing)
/// decides tail-vs-loud for every check. `cursor` carries the LSN-ascending
/// state from the recovered prefix.
///
/// # Performance
///
/// This function is `O(remaining.len())`.
fn parse_one_record(base_generation: u64, remaining: &[u8], cursor: LsnCursor) -> RecordParse {
    // (a) A full header must remain; otherwise this is a torn tail.
    if remaining.len() < HEADER_LEN {
        return RecordParse::Stop;
    }
    let Ok(header) = LogRecordHeader::read_from_bytes(&remaining[..HEADER_LEN]) else {
        // A header that fails zerocopy parsing cannot happen for a slice of
        // exactly `HEADER_LEN` bytes, but treat it as a torn tail rather than
        // panicking.
        return RecordParse::Stop;
    };

    let len = header.len.get() as usize;
    let op_count = header.op_count.get() as usize;
    let lsn = header.lsn.get();

    // Compute the expected framing to know whether a well-formed record can
    // follow this one (which decides tail-vs-loud for every check below).
    let body_len = op_count
        .checked_mul(OP_LEN)
        .and_then(|ops_bytes| ops_bytes.checked_add(HEADER_LEN));
    let frame_well_framed = matches!(body_len, Some(body_bytes) if body_bytes <= len)
        && len <= remaining.len()
        && len >= HEADER_LEN;
    // `is_last` is true when nothing fully valid can follow this record: either it
    // is misframed, or it consumes the buffer to its end (`len >= remaining.len()`,
    // equivalently `offset + len >= log.len()` since `remaining = log[offset..]`).
    let is_last = !frame_well_framed || len >= remaining.len();

    // (b) Magic.
    if header.magic.get() != OXGLOGR {
        return tail_or_loud(
            is_last,
            StorageError::LogCorrupt {
                lsn,
                reason: "delta-log record magic mismatch",
            },
        );
    }

    // (c) Base generation.
    let record_generation = header.base_generation.get();
    if record_generation != base_generation {
        return tail_or_loud(
            is_last,
            StorageError::BaseGenerationMismatch {
                expected: base_generation,
                found: record_generation,
            },
        );
    }

    // (d) Framing: len must equal header + ops + blob and fit the buffer.
    let Some(framed_min) = body_len else {
        // op_count overflowed; nothing valid can follow -> torn tail.
        return RecordParse::Stop;
    };
    if len < framed_min || len < HEADER_LEN {
        return tail_or_loud(
            is_last,
            StorageError::LogCorrupt {
                lsn,
                reason: "delta-log record length undershoots its framing",
            },
        );
    }
    if len > remaining.len() {
        // The declared record runs past the buffer: torn tail.
        return RecordParse::Stop;
    }

    let record = &remaining[..len];

    // (e) CRC over the whole record except the CRC field.
    if record_crc(record) != header.crc32c.get() {
        return tail_or_loud(
            is_last,
            StorageError::LogCorrupt {
                lsn,
                reason: "delta-log record checksum mismatch",
            },
        );
    }

    // (f) Strictly ascending LSN. A CRC-valid record after another CRC-valid
    // record is never a torn tail; an out-of-order LSN here is loud corruption.
    if cursor.seen && lsn <= cursor.last_lsn {
        return RecordParse::Loud(StorageError::LogCorrupt {
            lsn,
            reason: "delta-log record LSN not strictly ascending",
        });
    }

    let (ops, blob) = decode_ops(record, framed_min, op_count);
    RecordParse::Frame(
        CommitFrame {
            lsn,
            txn_id: header.txn_id.get(),
            base_generation: record_generation,
            ops,
            blob,
        },
        len,
    )
}

/// Replays a delta-log buffer against `base_generation`, returning the maximal
/// valid prefix of frames.
///
/// Records are walked strictly by `len`. For each record, recovery requires (a)
/// at least a header remains, (b) `magic == OXGLOGR`, (c) the record's base
/// generation equals `base_generation`, (d) `len` equals `header + op_count *
/// op_size + blob` and fits the remaining bytes, (e) the CRC recomputes, and
/// (f) LSN strictly ascends. A failing record that is the last content in the
/// buffer (no fully valid record after it) is a torn tail: replay stops
/// silently and the valid prefix is returned. A failing record with a
/// well-formed record after it is loud.
///
/// # Errors
///
/// Returns [`StorageError::LogCorrupt`] for a non-tail framing, magic, CRC, or LSN
/// violation, and [`StorageError::BaseGenerationMismatch`] for a non-tail base
/// generation mismatch.
///
/// # Performance
///
/// This function is `O(log.len())`.
pub(crate) fn replay(base_generation: u64, log: &[u8]) -> Result<ReplayOutcome, StorageError> {
    let mut frames: Vec<CommitFrame> = Vec::new();
    let mut offset = 0usize;
    let mut valid_len = 0usize;
    let mut last_lsn = 0u64;
    let mut last_txn = 0u64;
    let mut seen_lsn = false;

    while offset < log.len() {
        let cursor = LsnCursor {
            last_lsn,
            seen: seen_lsn,
        };
        match parse_one_record(base_generation, &log[offset..], cursor) {
            RecordParse::Stop => break,
            RecordParse::Loud(error) => return Err(error),
            RecordParse::Frame(frame, len) => {
                last_lsn = frame.lsn;
                last_txn = frame.txn_id;
                seen_lsn = true;
                frames.push(frame);
                offset += len;
                valid_len = offset;
            }
        }
    }

    Ok(ReplayOutcome {
        frames,
        valid_len,
        last_lsn,
        last_txn,
    })
}

/// Reads and verifies the superblock from a database root. This is the open-time
/// gate for the superblock: it enforces every "must equal/​must be zero" contract
/// the [`SuperblockRecord`] field docs assert — magic, CRC, format version, and
/// the reserved `flags`/`pad` words — so a superblock written by a future or
/// incompatible format is rejected rather than silently misinterpreted.
///
/// # Errors
///
/// Returns [`StorageError::NotFound`] when `super.oxgdb` is absent,
/// [`StorageError::InvalidStore`] when its size, magic, CRC, format version, or
/// reserved words are wrong, and [`StorageError::io`] for other IO failures.
///
/// # Performance
///
/// This function is `O(1)`; the superblock is fixed-size.
pub(crate) fn read_superblock(root: &Path) -> Result<SuperblockRecord, StorageError> {
    let path = root.join(SUPERBLOCK_FILE);
    let mut file = File::open(&path).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => StorageError::NotFound,
        _other => StorageError::io("open superblock", error),
    })?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| StorageError::io("read superblock", error))?;
    let record = SuperblockRecord::read_from_bytes(bytes.as_slice())
        .map_err(|_error| StorageError::invalid_store("superblock size mismatch"))?;
    if record.magic != SUPERBLOCK_MAGIC {
        return Err(StorageError::invalid_store("superblock magic mismatch"));
    }
    let expected = checksum(&bytes[..SUPERBLOCK_CRC_PREFIX_LEN]);
    if record.crc32c.get() != expected {
        return Err(StorageError::invalid_store("superblock checksum mismatch"));
    }
    // A CRC-valid superblock from a future/incompatible format would otherwise be
    // silently accepted and then have its fields misread; gate on the documented
    // version and reserved-zero contracts.
    if record.format_version.get() != OXGDB_FORMAT_VERSION {
        return Err(StorageError::invalid_store(
            "superblock format version unsupported",
        ));
    }
    if record.flags.get() != 0 || record.pad.get() != 0 {
        return Err(StorageError::invalid_store(
            "superblock reserved word not zero",
        ));
    }
    Ok(record)
}

/// Writes the superblock to a database root, publishing it crash-atomically via
/// a temp file, `sync_all`, atomic rename, and directory fsync. The caller
/// supplies every field except `crc32c`, which this function computes and
/// stamps over the preceding bytes.
///
/// # Errors
///
/// Returns [`StorageError::io`] when creating, writing, syncing, renaming, or syncing
/// the directory entry fails.
///
/// # Performance
///
/// This function is `O(1)`; the superblock is fixed-size.
pub(crate) fn write_superblock(root: &Path, sb: &SuperblockRecord) -> Result<(), StorageError> {
    fs::create_dir_all(root)
        .map_err(|error| StorageError::io("create database directory", error))?;
    let mut record = *sb;
    record.crc32c = 0u32.into();
    let mut bytes = record.as_bytes().to_vec();
    let crc = checksum(&bytes[..SUPERBLOCK_CRC_PREFIX_LEN]);
    record.crc32c = crc.into();
    bytes = record.as_bytes().to_vec();

    let temp_path = root.join(SUPERBLOCK_TEMP_FILE);
    let mut file =
        File::create(&temp_path).map_err(|error| StorageError::io("create superblock", error))?;
    file.write_all(&bytes)
        .map_err(|error| StorageError::io("write superblock", error))?;
    file.flush()
        .map_err(|error| StorageError::io("flush superblock", error))?;
    file.sync_all()
        .map_err(|error| StorageError::io("sync superblock", error))?;
    fs::rename(&temp_path, root.join(SUPERBLOCK_FILE))
        .map_err(|error| StorageError::io("publish superblock", error))?;
    sync_directory(root)
}

/// Appends one encoded commit record to the delta-log, then fsyncs the file. On
/// any write failure the file is truncated back to the captured EOF so no
/// interior torn record survives.
///
/// # Errors
///
/// Returns [`StorageError::io`] when seeking, writing, truncating, or syncing the log
/// fails.
///
/// # Performance
///
/// This function is `O(frame.len())`.
pub(crate) fn append_commit(log: &mut File, frame: &[u8]) -> Result<(), StorageError> {
    let eof = log
        .seek(SeekFrom::End(0))
        .map_err(|error| StorageError::io("seek delta-log end", error))?;
    if let Err(error) = log.write_all(frame) {
        rollback_to(log, eof);
        return Err(StorageError::io("append delta-log record", error));
    }
    if let Err(error) = log.sync_all() {
        rollback_to(log, eof);
        return Err(StorageError::io("sync delta-log", error));
    }
    Ok(())
}

/// Truncates the log back to `eof` and best-effort fsyncs after an append
/// failure, so the file never retains an interior torn record. Truncation errors
/// are swallowed because the original append error is the one returned to the
/// caller and is the actionable failure.
///
/// # Performance
///
/// This function is `O(1)`.
fn rollback_to(log: &File, eof: u64) {
    let _truncate = log.set_len(eof);
    let _sync = log.sync_all();
}

/// Syncs a directory entry publication on Unix filesystems, mirroring the
/// whole-store writer's directory fsync.
///
/// # Errors
///
/// Returns [`StorageError::io`] when the directory cannot be opened or synced.
///
/// # Performance
///
/// This function is `O(1)`.
#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), StorageError> {
    let directory =
        File::open(path).map_err(|error| StorageError::io("open database directory", error))?;
    directory
        .sync_all()
        .map_err(|error| StorageError::io("sync database directory", error))
}

/// Treats directory sync as unsupported on non-Unix targets.
///
/// # Performance
///
/// This function is `O(1)`.
#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), StorageError> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use proptest::prelude::*;
    use zerocopy::byteorder::{LE, U64};

    use super::*;
    use crate::wire::{MUTATION_OP_PAYLOAD_WORDS, OP_CREATE_ELEMENT, OP_NEXT_ID_WATERMARK};

    /// Per-process path counter for unique tempdirs.
    static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

    /// Builds a unique temporary database root.
    fn temp_root(name: &str) -> std::path::PathBuf {
        let id = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("oxgraph-wal-{name}-{}-{id}", std::process::id()))
    }

    /// Builds a `MutationOp` with the given kind, flags, and leading payload
    /// words (remaining words zero).
    fn op(op_kind: u32, flags: u32, words: &[u64]) -> MutationOp {
        let mut payload = [U64::<LE>::new(0); MUTATION_OP_PAYLOAD_WORDS];
        for (slot, value) in payload.iter_mut().zip(words) {
            *slot = U64::new(*value);
        }
        MutationOp {
            op_kind: op_kind.into(),
            flags: flags.into(),
            payload,
        }
    }

    /// A single encoded commit round-trips through replay with its ops and blob
    /// intact and reports its byte length as the frontier.
    #[test]
    fn single_commit_roundtrips() {
        let ops = vec![op(OP_CREATE_ELEMENT, 0, &[7])];
        let blob = b"hello".to_vec();
        let record = encode_commit(3, 5, 9, &ops, &blob).expect("encode");
        let outcome = replay(9, &record).expect("replay");
        assert_eq!(outcome.valid_len, record.len());
        assert_eq!(outcome.last_lsn, 3);
        assert_eq!(outcome.last_txn, 5);
        assert_eq!(outcome.frames.len(), 1);
        let frame = &outcome.frames[0];
        assert_eq!(frame.ops, ops);
        assert_eq!(frame.blob, blob);
        assert_eq!(frame.base_generation, 9);
    }

    /// Flipping one byte inside a NON-last record makes replay fail loudly with
    /// `LogCorrupt`.
    #[test]
    fn interior_corruption_is_loud() {
        let first = encode_commit(1, 1, 0, &[op(OP_CREATE_ELEMENT, 0, &[1])], b"").expect("encode");
        let second =
            encode_commit(2, 2, 0, &[op(OP_CREATE_ELEMENT, 0, &[2])], b"").expect("encode");
        let mut log = first;
        log.extend_from_slice(&second);
        // Flip a payload byte of the first (non-last) record.
        let flip_at = HEADER_LEN + 8;
        log[flip_at] ^= 0xFF;
        match replay(0, &log) {
            Err(StorageError::LogCorrupt { .. }) => {}
            other => panic!("expected LogCorrupt, got {other:?}"),
        }
    }

    /// A frame carrying a different base generation, appended after a valid
    /// frame and followed by more bytes, is a loud `BaseGenerationMismatch`.
    #[test]
    fn base_generation_mismatch_is_loud() {
        let g1a = encode_commit(1, 1, 1, &[op(OP_CREATE_ELEMENT, 0, &[1])], b"").expect("encode");
        let g2 = encode_commit(2, 2, 2, &[op(OP_CREATE_ELEMENT, 0, &[2])], b"").expect("encode");
        let g1b = encode_commit(3, 3, 1, &[op(OP_CREATE_ELEMENT, 0, &[3])], b"").expect("encode");
        let mut log = g1a;
        log.extend_from_slice(&g2);
        log.extend_from_slice(&g1b);
        match replay(1, &log) {
            Err(StorageError::BaseGenerationMismatch { expected, found }) => {
                assert_eq!(expected, 1);
                assert_eq!(found, 2);
            }
            other => panic!("expected BaseGenerationMismatch, got {other:?}"),
        }
    }

    /// A torn tail (truncation anywhere past the first complete record) recovers
    /// exactly the maximal complete prefix and never errors.
    #[test]
    fn torn_tail_truncates_silently() {
        let first =
            encode_commit(1, 1, 0, &[op(OP_CREATE_ELEMENT, 0, &[1])], b"a").expect("encode");
        let second =
            encode_commit(2, 2, 0, &[op(OP_CREATE_ELEMENT, 0, &[2])], b"bb").expect("encode");
        let mut log = first.clone();
        log.extend_from_slice(&second);
        // Cut inside the second record.
        let cut = first.len() + 4;
        let outcome = replay(0, &log[..cut]).expect("replay");
        assert_eq!(outcome.valid_len, first.len());
        assert_eq!(outcome.frames.len(), 1);
        assert_eq!(outcome.last_lsn, 1);
    }

    /// Superblock round-trips through write then read.
    #[test]
    fn superblock_roundtrips() {
        let root = temp_root("sb-roundtrip");
        let sb = SuperblockRecord {
            magic: SUPERBLOCK_MAGIC,
            base_generation: 4u64.into(),
            checkpoint_lsn: 11u64.into(),
            log_byte_offset: 0u64.into(),
            commit_seq: 17u64.into(),
            transaction_id: 19u64.into(),
            format_version: crate::wire::OXGDB_FORMAT_VERSION.into(),
            flags: 0u32.into(),
            crc32c: 0u32.into(),
            pad: 0u32.into(),
        };
        write_superblock(&root, &sb).expect("write");
        let read = read_superblock(&root).expect("read");
        assert_eq!(read.base_generation.get(), 4);
        assert_eq!(read.checkpoint_lsn.get(), 11);
        assert_eq!(read.commit_seq.get(), 17);
        assert_eq!(read.transaction_id.get(), 19);
        let _ignore = std::fs::remove_dir_all(&root);
    }

    /// Corrupting a byte of `super.oxgdb` makes `read_superblock` reject it.
    #[test]
    fn superblock_corruption_detected() {
        let root = temp_root("sb-corrupt");
        let sb = SuperblockRecord {
            magic: SUPERBLOCK_MAGIC,
            base_generation: 1u64.into(),
            checkpoint_lsn: 0u64.into(),
            log_byte_offset: 0u64.into(),
            commit_seq: 0u64.into(),
            transaction_id: 0u64.into(),
            format_version: crate::wire::OXGDB_FORMAT_VERSION.into(),
            flags: 0u32.into(),
            crc32c: 0u32.into(),
            pad: 0u32.into(),
        };
        write_superblock(&root, &sb).expect("write");
        let path = root.join(SUPERBLOCK_FILE);
        let mut bytes = std::fs::read(&path).expect("read bytes");
        // Flip a byte inside the CRC-covered prefix (the base generation word).
        bytes[8] ^= 0xFF;
        std::fs::write(&path, &bytes).expect("rewrite");
        match read_superblock(&root) {
            Err(StorageError::InvalidStore { .. }) => {}
            other => panic!("expected InvalidStore, got {other:?}"),
        }
        let _ignore = std::fs::remove_dir_all(&root);
    }

    /// A superblock whose `format_version` is not [`OXGDB_FORMAT_VERSION`] is
    /// rejected by `read_superblock` even when magic and CRC are valid, so a
    /// future/incompatible format cannot be silently opened and misread.
    #[test]
    fn superblock_unsupported_version_rejected() {
        let root = temp_root("sb-version");
        let sb = SuperblockRecord {
            magic: SUPERBLOCK_MAGIC,
            base_generation: 1u64.into(),
            checkpoint_lsn: 0u64.into(),
            log_byte_offset: 0u64.into(),
            commit_seq: 0u64.into(),
            transaction_id: 0u64.into(),
            // A version this build does not recognize.
            format_version: (OXGDB_FORMAT_VERSION + 1).into(),
            flags: 0u32.into(),
            crc32c: 0u32.into(),
            pad: 0u32.into(),
        };
        // `write_superblock` stamps a valid CRC over the (future-version) bytes.
        write_superblock(&root, &sb).expect("write");
        match read_superblock(&root) {
            Err(StorageError::InvalidStore { .. }) => {}
            other => panic!("expected InvalidStore, got {other:?}"),
        }
        let _ignore = std::fs::remove_dir_all(&root);
    }

    /// `append_commit` writes a frame and a subsequent replay recovers it.
    #[test]
    fn append_commit_then_replay() {
        let root = temp_root("append");
        std::fs::create_dir_all(&root).expect("mkdir");
        let path = root.join("delta-0.log");
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&path)
            .expect("open log");
        let frame =
            encode_commit(1, 1, 0, &[op(OP_CREATE_ELEMENT, 0, &[1])], b"x").expect("encode");
        append_commit(&mut file, &frame).expect("append");
        let bytes = std::fs::read(&path).expect("read log");
        let outcome = replay(0, &bytes).expect("replay");
        assert_eq!(outcome.frames.len(), 1);
        assert_eq!(outcome.valid_len, frame.len());
        let _ignore = std::fs::remove_dir_all(&root);
    }

    /// One commit-shaped frame: a small op set plus a blob and a watermark op.
    fn commit_strategy() -> impl Strategy<Value = (u64, Vec<MutationOp>, Vec<u8>)> {
        (
            any::<u64>(),
            prop::collection::vec(any::<u64>(), 0..4),
            prop::collection::vec(any::<u8>(), 0..12),
        )
            .prop_map(|(txn, ids, blob)| {
                let mut ops: Vec<MutationOp> = ids
                    .into_iter()
                    .map(|id| op(OP_CREATE_ELEMENT, 0, &[id]))
                    .collect();
                ops.push(op(OP_NEXT_ID_WATERMARK, 0, &[1, 1, 1, 1, 1, 1, 1, 1, 1]));
                (txn, ops, blob)
            })
    }

    proptest! {
        /// Encoding a sequence of commits with ascending LSNs, concatenating
        /// them, and truncating at an arbitrary point recovers EXACTLY the
        /// maximal set of complete records fully contained before the cut, with
        /// `valid_len` equal to their total bytes.
        #[test]
        fn arbitrary_truncation_recovers_valid_prefix(
            commits in prop::collection::vec(commit_strategy(), 1..6),
            cut_fraction in 0u64..=1000,
        ) {
            let base_generation = 0u64;
            let mut buf = Vec::new();
            let mut boundaries = Vec::new();
            for (index, (txn, ops, blob)) in commits.iter().enumerate() {
                let lsn = index as u64 + 1;
                let record = encode_commit(lsn, *txn, base_generation, ops, blob).expect("encode");
                buf.extend_from_slice(&record);
                boundaries.push(buf.len());
            }
            let fraction = usize::try_from(cut_fraction).unwrap_or(usize::MAX);
            let cut = fraction * buf.len() / 1000;
            // Expected valid prefix: the last record boundary <= cut.
            let expected_len = boundaries
                .iter()
                .copied()
                .rfind(|&boundary| boundary <= cut)
                .unwrap_or(0);
            let expected_count = boundaries.iter().filter(|boundary| **boundary <= cut).count();
            let outcome = replay(base_generation, &buf[..cut])?;
            prop_assert_eq!(outcome.valid_len, expected_len);
            prop_assert_eq!(outcome.frames.len(), expected_count);
            for (index, frame) in outcome.frames.iter().enumerate() {
                prop_assert_eq!(frame.lsn, index as u64 + 1);
            }
        }
    }
}
