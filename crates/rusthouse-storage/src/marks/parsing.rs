use super::*;

pub(super) fn parse_mark_record(record: &[u8], format: WideMarkFormat) -> WideMark {
    WideMark {
        compressed_offset: read_u64(&record[..8]),
        decompressed_offset: read_u64(&record[8..16]),
        rows: (format == WideMarkFormat::Adaptive).then(|| read_u64(&record[16..24])),
    }
}

pub(super) fn read_u64(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes.try_into().expect("eight-byte mark word"))
}
