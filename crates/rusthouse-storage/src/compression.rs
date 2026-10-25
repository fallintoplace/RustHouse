use crate::PartError;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const CHECKSUM_SIZE: u64 = 16;
const HEADER_SIZE: u32 = 9;
const PREFIX_SIZE: usize = 25;
const MAX_BLOCK_SIZE: u32 = 0x4000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressedBlock {
    pub offset: u64,
    pub method: u8,
    pub compressed_size: u32,
    pub decompressed_size: u32,
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
        let file_len = file
            .metadata()
            .map_err(|source| PartError::Io {
                path: path.to_path_buf(),
                source,
            })?
            .len();
        Ok(Self::new(file, path.to_path_buf(), file_len))
    }
}

impl<R: Read + Seek> CompressedBlockReader<R> {
    fn new(reader: R, path: PathBuf, file_len: u64) -> Self {
        Self {
            reader,
            path,
            file_len,
            offset: 0,
            done: false,
        }
    }

    fn invalid(&mut self, reason: &'static str) -> Option<Result<CompressedBlock, PartError>> {
        self.done = true;
        Some(Err(PartError::InvalidCompressedBlock {
            path: self.path.clone(),
            offset: self.offset,
            reason,
        }))
    }

    fn io_error(&mut self, source: io::Error) -> Option<Result<CompressedBlock, PartError>> {
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

        if let Err(source) = self.reader.seek(SeekFrom::Start(end)) {
            return self.io_error(source);
        }
        let block = CompressedBlock {
            offset: self.offset,
            method: prefix[16],
            compressed_size,
            decompressed_size,
        };
        self.offset = end;
        Some(Ok(block))
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

    fn read(bytes: Vec<u8>) -> Vec<Result<CompressedBlock, PartError>> {
        let len = bytes.len() as u64;
        CompressedBlockReader::new(Cursor::new(bytes), PathBuf::from("data.bin"), len).collect()
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
            let mut reader = CompressedBlockReader::new(
                Cursor::new(vec![0; size]),
                PathBuf::from("bad.bin"),
                size as u64,
            );
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
    fn reports_truncated_reads_if_file_changes_after_opening() {
        let mut reader = CompressedBlockReader::new(
            Cursor::new(vec![0; 1]),
            PathBuf::from("modified.bin"),
            PREFIX_SIZE as u64,
        );
        assert!(matches!(reader.next(), Some(Err(PartError::Io { .. }))));
        assert!(reader.next().is_none());
    }
}
