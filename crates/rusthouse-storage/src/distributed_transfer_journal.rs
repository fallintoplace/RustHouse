//! Versioned CRC32C frames for replica transfer receipt persistence.
//! CRC32C detects accidental corruption, not malicious tampering.
use crate::distributed_crc32c::crc32c;
use std::collections::HashSet;
const MAGIC: &[u8; 4] = b"RHJ1";
const FRAME_SIZE: usize = 24;
const MAX_RECORDS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JournalReceipt {
    pub sequence: u64,
    pub chunk_index: u64,
    pub payload_crc32c: u32,
}

pub fn encode_transfer_journal(records: &[JournalReceipt]) -> Result<Vec<u8>, &'static str> {
    if records.len() > MAX_RECORDS { return Err("transfer journal exceeds record bound"); }
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(4 + records.len() * FRAME_SIZE);
    out.extend_from_slice(MAGIC);
    for (index, record) in records.iter().enumerate() {
        if record.sequence != index as u64 + 1 || !seen.insert(record.chunk_index) {
            return Err("noncanonical transfer receipt sequence");
        }
        let begin = out.len();
        out.extend_from_slice(&record.sequence.to_le_bytes());
        out.extend_from_slice(&record.chunk_index.to_le_bytes());
        out.extend_from_slice(&record.payload_crc32c.to_le_bytes());
        let checksum = crc32c(&out[begin..]);
        out.extend_from_slice(&checksum.to_le_bytes());
    }
    Ok(out)
}

pub fn decode_transfer_journal(input: &[u8]) -> Result<Vec<JournalReceipt>, &'static str> {
    if !input.starts_with(MAGIC) || input.len() < MAGIC.len() {
        return Err("invalid transfer journal version");
    }
    let frames = &input[MAGIC.len()..];
    if frames.len() % FRAME_SIZE != 0 || frames.len() / FRAME_SIZE > MAX_RECORDS {
        return Err("truncated or oversized transfer journal");
    }
    let mut records = Vec::with_capacity(frames.len() / FRAME_SIZE);
    let mut seen = HashSet::new();
    for frame in frames.chunks_exact(FRAME_SIZE) {
        let sequence = u64::from_le_bytes(frame[0..8].try_into().expect("eight bytes"));
        let chunk_index = u64::from_le_bytes(frame[8..16].try_into().expect("eight bytes"));
        let payload_crc32c = u32::from_le_bytes(frame[16..20].try_into().expect("four bytes"));
        let record_crc32c = u32::from_le_bytes(frame[20..24].try_into().expect("four bytes"));
        if record_crc32c != crc32c(&frame[..20])
            || sequence != records.len() as u64 + 1
            || !seen.insert(chunk_index)
        {
            return Err("corrupt, duplicated, or reordered transfer receipt");
        }
        records.push(JournalReceipt { sequence, chunk_index, payload_crc32c });
    }
    Ok(records)
}


/// Decode a journal only when its complete receipt count is known independently.
///
/// Per-frame CRCs cannot detect truncation at a whole-frame boundary. The
/// expected count must come from separately trusted transfer metadata.
pub fn decode_transfer_journal_exact(
    input: &[u8],
    expected_records: usize,
) -> Result<Vec<JournalReceipt>, &'static str> {
    if expected_records > MAX_RECORDS {
        return Err("expected transfer journal length exceeds bound");
    }
    if input.len() != MAGIC.len() + expected_records * FRAME_SIZE {
        return Err("transfer journal receipt count mismatch");
    }
    decode_transfer_journal(input)
}

#[cfg(test)]
mod tests {
    #[test]
    fn expected_count_catches_whole_frame_truncation() {
        let records = [entry(1, 8), entry(2, 3), entry(3, 4)];
        let encoded = encode_transfer_journal(&records).unwrap();
        let truncated = &encoded[..encoded.len() - FRAME_SIZE];
        // Frame checksums alone cannot distinguish this from a shorter journal.
        assert!(decode_transfer_journal(truncated).is_ok());
        assert!(decode_transfer_journal_exact(truncated, records.len()).is_err());
        assert_eq!(decode_transfer_journal_exact(&encoded, records.len()).unwrap(), records);
        assert!(decode_transfer_journal_exact(&encoded, MAX_RECORDS + 1).is_err());
    }

    use super::*;
    fn entry(sequence: u64, chunk_index: u64) -> JournalReceipt {
        JournalReceipt { sequence, chunk_index, payload_crc32c: 13 }
    }
    #[test]
    fn roundtrips_out_of_order_chunk_numbers_with_monotonic_journal_sequence() {
        let entries = [entry(1, 8), entry(2, 3), entry(3, 4)];
        let bytes = encode_transfer_journal(&entries).unwrap();
        assert_eq!(decode_transfer_journal(&bytes).unwrap(), entries);
    }
    #[test]
    fn rejects_corrupt_and_truncated_frames() {
        let mut bytes = encode_transfer_journal(&[entry(1, 9)]).unwrap();
        bytes[12] ^= 1;
        assert!(decode_transfer_journal(&bytes).is_err());
        bytes.pop();
        assert!(decode_transfer_journal(&bytes).is_err());
    }
    #[test]
    fn rejects_repeated_chunks_and_nonsequential_records() {
        assert!(encode_transfer_journal(&[entry(1, 5), entry(2, 5)]).is_err());
        assert!(encode_transfer_journal(&[entry(2, 4)]).is_err());
        assert!(decode_transfer_journal(b"bad").is_err());
    }
}
