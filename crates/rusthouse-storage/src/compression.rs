use crate::PartError;
use ch_cityhash102::cityhash128;
use lz4_flex::block::decompress_into;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const CHECKSUM_SIZE: u64 = 16;
const HEADER_SIZE: u32 = 9;
const PREFIX_SIZE: usize = 25;
const MAX_BLOCK_SIZE: u32 = 0x4000_0000;
const MAX_DECODE_BLOCK_SIZE: u32 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressedBlock {
    pub offset: u64,
    pub method: u8,
    pub compressed_size: u32,
    pub decompressed_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedBlock {
    pub block: CompressedBlock,
    pub data: Vec<u8>,
}

struct BlockHeader {
    block: CompressedBlock,
    prefix: [u8; PREFIX_SIZE],
}

pub struct CompressedBlockReader<R: Read + Seek> {
    reader: R,
    path: PathBuf,
    file_len: u64,
    offset: u64,
    done: bool,
}

impl CompressedBlockReader<File> {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PartError> {
        let path = path.as_ref();
        let file = File::open(path).map_err(|source| PartError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let metadata = file.metadata().map_err(|source| PartError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if !metadata.is_file() {
            return Err(PartError::Io {
                path: path.to_path_buf(),
                source: io::Error::new(io::ErrorKind::InvalidInput, "expected a regular file"),
            });
        }
        Self::new(file, path)
    }
}

impl<R: Read + Seek> CompressedBlockReader<R> {
    pub fn new(mut reader: R, path: impl AsRef<Path>) -> Result<Self, PartError> {
        let path = path.as_ref().to_path_buf();
        let file_len = reader
            .seek(SeekFrom::End(0))
            .map_err(|source| PartError::Io {
                path: path.clone(),
                source,
            })?;
        reader
            .seek(SeekFrom::Start(0))
            .map_err(|source| PartError::Io {
                path: path.clone(),
                source,
            })?;
        Ok(Self {
            reader,
            path,
            file_len,
            offset: 0,
            done: false,
        })
    }

    pub fn seek_to_block(&mut self, offset: u64) -> Result<(), PartError> {
        if offset >= self.file_len {
            return Err(PartError::InvalidCompressedBlock {
                path: self.path.clone(),
                offset,
                reason: "block offset is outside file",
            });
        }

        self.reader
            .seek(SeekFrom::Start(offset))
            .map_err(|source| PartError::Io {
                path: self.path.clone(),
                source,
            })?;
        self.offset = offset;
        self.done = false;
        Ok(())
    }

    fn next_header(&mut self) -> Option<Result<BlockHeader, PartError>> {
        if self.done || self.offset == self.file_len {
            return None;
        }
        if self.file_len - self.offset < PREFIX_SIZE as u64 {
            return self.invalid("truncated checksum or block header");
        }

        let mut prefix = [0u8; PREFIX_SIZE];
        if let Err(source) = self.reader.read_exact(&mut prefix) {
            return self.io_error(source);
        }
        let compressed_size = u32::from_le_bytes(prefix[17..21].try_into().unwrap());
        let decompressed_size = u32::from_le_bytes(prefix[21..25].try_into().unwrap());
        if compressed_size < HEADER_SIZE {
            return self.invalid("compressed size is smaller than the block header");
        }
        if compressed_size > MAX_BLOCK_SIZE || decompressed_size > MAX_BLOCK_SIZE {
            return self.invalid("compressed or decompressed size exceeds the 1 GiB limit");
        }
        if decompressed_size == 0 {
            return self.invalid("decompressed size is zero");
        }

        let Some(end) = self
            .offset
            .checked_add(CHECKSUM_SIZE + u64::from(compressed_size))
        else {
            return self.invalid("compressed block offset overflows");
        };
        if end > self.file_len {
            return self.invalid("compressed block extends past end of file");
        }

        let block = CompressedBlock {
            offset: self.offset,
            method: prefix[16],
            compressed_size,
            decompressed_size,
        };
        self.offset = end;
        Some(Ok(BlockHeader { block, prefix }))
    }

    pub fn next_decoded(&mut self) -> Option<Result<DecodedBlock, PartError>> {
        let header = match self.next_header()? {
            Ok(header) => header,
            Err(error) => return Some(Err(error)),
        };
        let block = header.block;
        if block.compressed_size > MAX_DECODE_BLOCK_SIZE
            || block.decompressed_size > MAX_DECODE_BLOCK_SIZE
        {
            return self.decoded_error(block.offset, "block exceeds the 64 MiB decode limit");
        }

        let mut encoded = vec![0u8; block.compressed_size as usize];
        encoded[..HEADER_SIZE as usize].copy_from_slice(&header.prefix[CHECKSUM_SIZE as usize..]);
        if let Err(source) = self.reader.read_exact(&mut encoded[HEADER_SIZE as usize..]) {
            return self.io_error(source);
        }
        let checksum = u128::from(cityhash128(&encoded)).to_le_bytes();
        if header.prefix[..CHECKSUM_SIZE as usize] != checksum {
            return self.decoded_error(block.offset, "checksum mismatch");
        }

        let data = match block.method {
            0x02 => {
                if encoded.len() - HEADER_SIZE as usize != block.decompressed_size as usize {
                    return self.decoded_error(block.offset, "uncompressed size mismatch");
                }
                encoded.copy_within(HEADER_SIZE as usize.., 0);
                encoded.truncate(block.decompressed_size as usize);
                encoded
            }
            0x82 => {
                let mut decoded = vec![0u8; block.decompressed_size as usize];
                match decompress_into(&encoded[HEADER_SIZE as usize..], &mut decoded) {
                    Ok(size) if size == decoded.len() => decoded,
                    _ => {
                        return self.decoded_error(
                            block.offset,
                            "invalid LZ4 payload or decompressed size",
                        );
                    }
                }
            }
            _ => return self.decoded_error(block.offset, "unsupported compression method"),
        };
        Some(Ok(DecodedBlock { block, data }))
    }

    fn decoded_error(
        &mut self,
        offset: u64,
        reason: &'static str,
    ) -> Option<Result<DecodedBlock, PartError>> {
        self.done = true;
        Some(Err(PartError::InvalidCompressedBlock {
            path: self.path.clone(),
            offset,
            reason,
        }))
    }

    fn invalid<T>(&mut self, reason: &'static str) -> Option<Result<T, PartError>> {
        self.done = true;
        Some(Err(PartError::InvalidCompressedBlock {
            path: self.path.clone(),
            offset: self.offset,
            reason,
        }))
    }

    fn io_error<T>(&mut self, source: io::Error) -> Option<Result<T, PartError>> {
        self.done = true;
        Some(Err(PartError::Io {
            path: self.path.clone(),
            source,
        }))
    }
}

impl<R: Read + Seek> Iterator for CompressedBlockReader<R> {
    type Item = Result<CompressedBlock, PartError>;

    fn next(&mut self) -> Option<Self::Item> {
        let header = match self.next_header()? {
            Ok(header) => header,
            Err(error) => return Some(Err(error)),
        };
        if let Err(source) = self.reader.seek(SeekFrom::Start(self.offset)) {
            return self.io_error(source);
        }
        Some(Ok(header.block))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn block(method: u8, payload: &[u8], decompressed_size: u32) -> Vec<u8> {
        let mut bytes = vec![0; CHECKSUM_SIZE as usize];
        bytes.push(method);
        bytes.extend_from_slice(&(HEADER_SIZE + payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&decompressed_size.to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn checked_block(method: u8, payload: &[u8], decompressed_size: u32) -> Vec<u8> {
        let mut bytes = block(method, payload, decompressed_size);
        let checksum = u128::from(cityhash128(&bytes[CHECKSUM_SIZE as usize..])).to_le_bytes();
        bytes[..CHECKSUM_SIZE as usize].copy_from_slice(&checksum);
        bytes
    }

    fn read(bytes: Vec<u8>) -> Vec<Result<CompressedBlock, PartError>> {
        CompressedBlockReader::new(Cursor::new(bytes), "data.bin")
            .unwrap()
            .collect()
    }

    #[test]
    fn reads_adjacent_blocks_and_keeps_codec_bytes() {
        let mut bytes = block(0x82, &[1, 2, 3, 4], 100);
        bytes.extend(block(0x90, &[5, 6], 200));
        bytes.extend(block(0xfe, &[7], 300));
        let blocks = read(bytes)
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            blocks,
            vec![
                CompressedBlock {
                    offset: 0,
                    method: 0x82,
                    compressed_size: 13,
                    decompressed_size: 100
                },
                CompressedBlock {
                    offset: 29,
                    method: 0x90,
                    compressed_size: 11,
                    decompressed_size: 200
                },
                CompressedBlock {
                    offset: 56,
                    method: 0xfe,
                    compressed_size: 10,
                    decompressed_size: 300
                },
            ]
        );
    }

    #[test]
    fn empty_file_has_no_blocks() {
        assert!(read(Vec::new()).is_empty());
    }

    #[test]
    fn rejects_truncated_prefix_and_stops_after_failure() {
        for size in [1, 16, 24] {
            let mut reader =
                CompressedBlockReader::new(Cursor::new(vec![0u8; size]), "bad.bin").unwrap();
            assert!(matches!(
                reader.next(),
                Some(Err(PartError::InvalidCompressedBlock { offset: 0, .. }))
            ));
            assert!(reader.next().is_none());
        }
    }

    #[test]
    fn rejects_invalid_sizes_without_allocating_payloads() {
        for (compressed, decompressed) in [
            (8u32, 10u32),
            (9, 0),
            (0x4000_0001, 10),
            (9, 0x4000_0001),
            (100, 10),
        ] {
            let mut bytes = vec![0; PREFIX_SIZE];
            bytes[17..21].copy_from_slice(&compressed.to_le_bytes());
            bytes[21..25].copy_from_slice(&decompressed.to_le_bytes());
            assert!(matches!(
                read(bytes).into_iter().next(),
                Some(Err(PartError::InvalidCompressedBlock { offset: 0, .. }))
            ));
        }
    }

    #[test]
    fn rejects_trailing_bytes_after_complete_block() {
        let mut bytes = block(0x02, &[1, 2], 2);
        bytes.push(0);
        let result = read(bytes);
        assert!(result[0].is_ok());
        assert!(matches!(
            result[1],
            Err(PartError::InvalidCompressedBlock { offset: 27, .. })
        ));
    }

    #[test]
    fn starts_at_beginning_of_supplied_reader() {
        let mut cursor = Cursor::new(block(0x82, &[1, 2], 2));
        cursor.set_position(5);
        let blocks = CompressedBlockReader::new(cursor, "memory.bin")
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].offset, 0);
        assert_eq!(blocks[0].compressed_size, 11);
    }

    #[test]
    fn reports_invalid_second_block_at_its_own_offset() {
        let mut bytes = block(0x82, &[1, 2], 2);
        let second_offset = bytes.len() as u64;
        bytes.extend(vec![0u8; PREFIX_SIZE]);
        let mut reader = CompressedBlockReader::new(Cursor::new(bytes), "invalid.bin").unwrap();
        assert_eq!(reader.next().unwrap().unwrap().offset, 0);
        assert!(matches!(
            reader.next(),
            Some(Err(PartError::InvalidCompressedBlock { offset, .. })) if offset == second_offset
        ));
        assert!(reader.next().is_none());
    }

    #[test]
    fn reports_truncated_reads_if_file_changes_after_opening() {
        let mut reader =
            CompressedBlockReader::new(Cursor::new(vec![0u8; PREFIX_SIZE]), "modified.bin")
                .unwrap();
        reader.reader.get_mut().truncate(1);
        assert!(matches!(reader.next(), Some(Err(PartError::Io { .. }))));
        assert!(reader.next().is_none());
    }

    #[test]
    fn decodes_checked_lz4_and_none_blocks() {
        let expected = b"rusthouse compressed columns ".repeat(40);
        let compressed = [
            0xff, 0x0e, 0x72, 0x75, 0x73, 0x74, 0x68, 0x6f, 0x75, 0x73, 0x65, 0x20, 0x63, 0x6f,
            0x6d, 0x70, 0x72, 0x65, 0x73, 0x73, 0x65, 0x64, 0x20, 0x63, 0x6f, 0x6c, 0x75, 0x6d,
            0x6e, 0x73, 0x20, 0x1d, 0x00, 0xff, 0xff, 0xff, 0xff, 0x57, 0x50, 0x75, 0x6d, 0x6e,
            0x73, 0x20,
        ];
        let mut bytes = checked_block(0x82, &compressed, expected.len() as u32);
        let second_offset = bytes.len() as u64;
        bytes.extend(checked_block(0x02, b"plain", 5));

        let mut reader = CompressedBlockReader::new(Cursor::new(bytes), "data.bin").unwrap();
        let first = reader.next_decoded().unwrap().unwrap();
        assert_eq!(first.block.offset, 0);
        assert_eq!(first.block.method, 0x82);
        assert_eq!(first.data, expected);
        let second = reader.next_decoded().unwrap().unwrap();
        assert_eq!(second.block.offset, second_offset);
        assert_eq!(second.data, b"plain");
        assert!(reader.next_decoded().is_none());
    }

    #[test]
    fn can_inspect_then_decode_next_block() {
        let mut bytes = checked_block(0x02, b"first", 5);
        let second_offset = bytes.len() as u64;
        bytes.extend(checked_block(0x02, b"second", 6));
        let mut reader = CompressedBlockReader::new(Cursor::new(bytes), "data.bin").unwrap();
        assert_eq!(reader.next().unwrap().unwrap().offset, 0);
        let second = reader.next_decoded().unwrap().unwrap();
        assert_eq!(second.block.offset, second_offset);
        assert_eq!(second.data, b"second");
        assert!(reader.next().is_none());
    }

    #[test]
    fn rejects_checksum_changes_to_header_payload_or_checksum() {
        for index in [0, 16, PREFIX_SIZE] {
            let mut bytes = checked_block(0x02, b"hello", 5);
            bytes[index] ^= 1;
            let mut reader = CompressedBlockReader::new(Cursor::new(bytes), "data.bin").unwrap();
            assert!(matches!(
                reader.next_decoded(),
                Some(Err(PartError::InvalidCompressedBlock {
                    offset: 0,
                    reason: "checksum mismatch",
                    ..
                }))
            ));
            assert!(reader.next_decoded().is_none());
        }
    }

    #[test]
    fn rejects_invalid_lz4_payload_and_wrong_decompressed_size() {
        let valid = &include_bytes!("../tests/fixtures/blocks/lz4-none.bin")[PREFIX_SIZE..36];
        for bytes in [
            checked_block(0x82, &[0xff], 100),
            checked_block(0x82, &valid, 100),
            checked_block(0x82, &valid, 2),
        ] {
            let mut reader = CompressedBlockReader::new(Cursor::new(bytes), "data.bin").unwrap();
            assert!(matches!(
                reader.next_decoded(),
                Some(Err(PartError::InvalidCompressedBlock {
                    reason: "invalid LZ4 payload or decompressed size",
                    ..
                }))
            ));
        }
    }

    #[test]
    fn rejects_uncompressed_size_mismatch_and_unsupported_codec() {
        for (bytes, expected_reason) in [
            (
                checked_block(0x02, b"plain", 6),
                "uncompressed size mismatch",
            ),
            (
                checked_block(0x90, b"zstd", 4),
                "unsupported compression method",
            ),
        ] {
            let mut reader = CompressedBlockReader::new(Cursor::new(bytes), "data.bin").unwrap();
            assert!(matches!(
                reader.next_decoded(),
                Some(Err(PartError::InvalidCompressedBlock { reason, .. })) if reason == expected_reason
            ));
            assert!(reader.next_decoded().is_none());
        }
    }

    #[test]
    fn rejects_large_decompression_before_allocating() {
        let bytes = checked_block(0x82, &[1], MAX_DECODE_BLOCK_SIZE + 1);
        let mut reader = CompressedBlockReader::new(Cursor::new(bytes), "data.bin").unwrap();
        assert!(matches!(
            reader.next_decoded(),
            Some(Err(PartError::InvalidCompressedBlock {
                reason: "block exceeds the 64 MiB decode limit",
                ..
            }))
        ));
    }

    #[test]
    fn reports_truncated_payload_after_source_changes() {
        let bytes = checked_block(0x02, b"hello", 5);
        let mut reader = CompressedBlockReader::new(Cursor::new(bytes), "data.bin").unwrap();
        reader.reader.get_mut().truncate(PREFIX_SIZE);
        assert!(matches!(
            reader.next_decoded(),
            Some(Err(PartError::Io { .. }))
        ));
        assert!(reader.next_decoded().is_none());
    }

    #[test]
    fn decodes_independent_compressed_block_fixture() {
        let bytes = include_bytes!("../tests/fixtures/blocks/lz4-none.bin");
        let mut reader = CompressedBlockReader::new(Cursor::new(bytes), "fixture.bin").unwrap();

        let first = reader.next_decoded().unwrap().unwrap();
        assert_eq!(first.block.offset, 0);
        assert_eq!(first.block.method, 0x82);
        assert_eq!(first.data, vec![b'a'; 80]);

        let second = reader.next_decoded().unwrap().unwrap();
        assert_eq!(second.block.offset, 36);
        assert_eq!(second.block.method, 0x02);
        assert_eq!(second.data, b"hello");
        assert!(reader.next_decoded().is_none());
    }

    #[test]
    fn seeks_to_compressed_offsets_and_restarts_after_eof() {
        let bytes = include_bytes!("../tests/fixtures/blocks/lz4-none.bin");
        let mut reader = CompressedBlockReader::new(Cursor::new(bytes), "fixture.bin").unwrap();

        reader.seek_to_block(36).unwrap();
        assert_eq!(reader.next_decoded().unwrap().unwrap().data, b"hello");
        assert!(reader.next_decoded().is_none());

        reader.seek_to_block(0).unwrap();
        assert_eq!(reader.next_decoded().unwrap().unwrap().data, vec![b'a'; 80]);
        assert!(matches!(
            reader.seek_to_block(bytes.len() as u64),
            Err(PartError::InvalidCompressedBlock {
                reason: "block offset is outside file",
                ..
            })
        ));
        assert_eq!(reader.next_decoded().unwrap().unwrap().data, b"hello");
    }

    #[test]
    fn rejects_corrupt_reference_frame_at_its_offset() {
        let mut bytes = include_bytes!("../tests/fixtures/blocks/lz4-none.bin").to_vec();
        bytes[36 + PREFIX_SIZE] ^= 1;
        let mut reader = CompressedBlockReader::new(Cursor::new(bytes), "fixture.bin").unwrap();

        assert_eq!(reader.next_decoded().unwrap().unwrap().data, vec![b'a'; 80]);
        assert!(matches!(
            reader.next_decoded(),
            Some(Err(PartError::InvalidCompressedBlock {
                offset: 36,
                reason: "checksum mismatch",
                ..
            }))
        ));
        assert!(reader.next_decoded().is_none());
    }
}
