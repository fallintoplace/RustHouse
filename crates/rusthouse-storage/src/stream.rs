use crate::{CompressedBlockReader, PartError};
use std::fs::File;
use std::io::{Read, Seek};
use std::path::Path;

pub struct UInt64ColumnReader<R: Read + Seek> {
    blocks: CompressedBlockReader<R>,
    current: Vec<u8>,
    position: usize,
    finished: bool,
}

impl UInt64ColumnReader<File> {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PartError> {
        Ok(Self::new(CompressedBlockReader::open(path)?))
    }
}

impl<R: Read + Seek> UInt64ColumnReader<R> {
    pub fn new(blocks: CompressedBlockReader<R>) -> Self {
        Self {
            blocks,
            current: Vec::new(),
            position: 0,
            finished: false,
        }
    }
    fn next_cross_block_value(&mut self) -> Option<Result<u64, PartError>> {
        let mut value = [0u8; 8];
        let mut filled = 0;
        while filled < value.len() {
            if self.position == self.current.len() {
                // The previous block is fully consumed. Recycle its capacity
                // instead of allocating a fresh decoded Vec for each block.
                let scratch = std::mem::take(&mut self.current);
                match self.blocks.next_decoded_reusing(scratch) {
                    Some(Ok(decoded)) => {
                        self.current = decoded.data;
                        self.position = 0;
                    }
                    Some(Err(error)) => {
                        self.finished = true;
                        return Some(Err(error));
                    }
                    None => {
                        self.finished = true;
                        return if filled == 0 {
                            None
                        } else {
                            Some(Err(PartError::InvalidColumnLength {
                                path: self.blocks.path().to_path_buf(),
                                width: value.len(),
                                trailing_bytes: filled,
                            }))
                        };
                    }
                }
            }

            let count = (value.len() - filled).min(self.current.len() - self.position);
            value[filled..filled + count]
                .copy_from_slice(&self.current[self.position..self.position + count]);
            filled += count;
            self.position += count;
        }

        Some(Ok(u64::from_le_bytes(value)))
    }
}

impl<R: Read + Seek> Iterator for UInt64ColumnReader<R> {
    type Item = Result<u64, PartError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        // Most values fit entirely in the current decoded block. Skip the
        // incremental copy loop, while keeping its cross-block fallback.
        if self.current.len().saturating_sub(self.position) >= 8 {
            let start = self.position;
            self.position += 8;
            let bytes = self.current[start..self.position]
                .try_into()
                .expect("an eight-byte slice");
            return Some(Ok(u64::from_le_bytes(bytes)));
        }

        self.next_cross_block_value()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn scan(bytes: Vec<u8>) -> UInt64ColumnReader<Cursor<Vec<u8>>> {
        let blocks = CompressedBlockReader::new(Cursor::new(bytes), "values.bin").unwrap();
        UInt64ColumnReader::new(blocks)
    }

    fn none_block(data: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0u8; 16];
        bytes.push(0x02);
        bytes.extend_from_slice(&(data.len() as u32 + 9).to_le_bytes());
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(data);
        let checksum = u128::from(ch_cityhash102::cityhash128(&bytes[16..])).to_le_bytes();
        bytes[..16].copy_from_slice(&checksum);
        bytes
    }

    #[test]
    fn streams_lz4_values_from_reference_fixture() {
        let fixture = include_bytes!("../tests/fixtures/blocks/lz4-none.bin");
        let values = scan(fixture[..36].to_vec())
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(values, vec![u64::from_le_bytes(*b"aaaaaaaa"); 10]);
    }

    #[test]
    fn reads_values_split_across_compressed_blocks() {
        let numbers = [7u64, u64::MAX, 42];
        let mut raw = Vec::new();
        for value in numbers {
            raw.extend_from_slice(&value.to_le_bytes());
        }
        let mut frames = none_block(&raw[..5]);
        frames.extend(none_block(&raw[5..14]));
        frames.extend(none_block(&raw[14..]));

        let values = scan(frames).collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(values, numbers);
    }

    #[test]
    fn rejects_trailing_incomplete_value_once() {
        let mut frames = none_block(&42u64.to_le_bytes());
        frames.extend(none_block(&[1, 2, 3]));
        let mut reader = scan(frames);

        assert_eq!(reader.next().unwrap().unwrap(), 42);
        assert!(matches!(
            reader.next(),
            Some(Err(PartError::InvalidColumnLength {
                width: 8,
                trailing_bytes: 3,
                ..
            }))
        ));
        assert!(reader.next().is_none());
    }

    #[test]
    fn stops_on_corrupted_compressed_block() {
        let mut frames = none_block(&7u64.to_le_bytes());
        let mut corrupt = none_block(&9u64.to_le_bytes());
        corrupt[25] ^= 1;
        frames.extend(corrupt);
        let mut reader = scan(frames);

        assert_eq!(reader.next().unwrap().unwrap(), 7);
        assert!(matches!(
            reader.next(),
            Some(Err(PartError::InvalidCompressedBlock {
                reason: "checksum mismatch",
                ..
            }))
        ));
        assert!(reader.next().is_none());
    }

    #[test]
    fn empty_column_is_empty() {
        assert!(scan(Vec::new()).next().is_none());
    }
    #[test]
    fn decodes_many_values_with_the_contiguous_block_path() {
        let expected = (0u64..4096)
            .map(|value| value * 7919 + 17)
            .collect::<Vec<_>>();
        let bytes = expected
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let actual = scan(none_block(&bytes))
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn handles_unaligned_values_after_cross_block_fallback() {
        let expected = [u64::MAX, 42, 77, 81];
        let bytes = expected
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let mut blocks = none_block(&bytes[..3]);
        blocks.extend(none_block(&bytes[3..]));
        let actual = scan(blocks).collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(actual, expected);
    }
    #[test]
    fn reconstructs_uint64_values_at_every_interior_split() {
        let expected = [0x0102030405060708u64, u64::MAX];
        let bytes = expected
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        for split in 1..8 {
            let mut blocks = none_block(&bytes[..split]);
            blocks.extend(none_block(&bytes[split..]));
            let values = scan(blocks).collect::<Result<Vec<_>, _>>().unwrap();
            assert_eq!(values, expected, "split {split}");
        }
    }
}

#[cfg(test)]
mod recycled_column_block_tests {
    use super::*;

    #[test]
    fn thousands_of_tiny_blocks_preserve_values_at_every_boundary() {
        let expected = (0..1024u64)
            .map(|index| index.wrapping_mul(0x9e37_79b9_7f4a_7c15))
            .collect::<Vec<_>>();
        let raw = expected.iter()
            .flat_map(|value| value.to_le_bytes())
            .collect::<Vec<_>>();
        let mut frames = Vec::new();
        for byte in raw {
            frames.extend(tests::none_block(&[byte]));
        }
        let actual = tests::scan(frames).collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(actual, expected);
    }
}
