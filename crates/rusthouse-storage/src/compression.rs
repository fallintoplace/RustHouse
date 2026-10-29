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
    encoded_scratch: Vec<u8>,
}

impl CompressedBlockReader<File> {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PartError> {
        let path = path.as_ref();
        let entry = std::fs::symlink_metadata(path).map_err(|source| PartError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if !entry.file_type().is_file() {
            return Err(PartError::Io {
                path: path.to_path_buf(),
                source: io::Error::new(io::ErrorKind::InvalidInput, "expected a regular file"),
            });
        }
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if entry.dev() != metadata.dev() || entry.ino() != metadata.ino() {
                return Err(PartError::Io {
                    path: path.to_path_buf(),
                    source: io::Error::new(
                        io::ErrorKind::InvalidData,
                        "compressed stream replaced before open",
                    ),
                });
            }
        }
        // File::open starts at offset zero. Reuse the validated metadata size
        // instead of stat-ing and seeking the file a second time.
        Ok(Self {
            reader: file,
            path: path.to_path_buf(),
            file_len: metadata.len(),
            offset: 0,
            done: false,
            encoded_scratch: Vec::new(),
        })
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
            encoded_scratch: Vec::new(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn seek_to_block(&mut self, offset: u64) -> Result<(), PartError> {
        if offset >= self.file_len {
            return Err(PartError::InvalidCompressedBlock {
                path: self.path.clone(),
                offset,
                reason: "block offset is outside file",
            });
        }

        if offset == self.offset && !self.done {
            return Ok(());
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
            return self.invalid_at(block.offset, "block exceeds the 64 MiB decode limit");
        }
        if !matches!(block.method, 0x02 | 0x82 | 0x90) {
            return self.invalid_at(block.offset, "unsupported compression method");
        }

        // LZ4/ZSTD decode into separate output buffers. Reuse the encoded input
        // allocation for later blocks, including across intervening NONE blocks.
        // For NONE, return the allocation itself to avoid a copy of the payload.
        let mut encoded = if block.method == 0x02 {
            vec![0u8; block.compressed_size as usize]
        } else {
            let mut scratch = std::mem::take(&mut self.encoded_scratch);
            scratch.resize(block.compressed_size as usize, 0);
            scratch
        };
        encoded[..HEADER_SIZE as usize].copy_from_slice(&header.prefix[CHECKSUM_SIZE as usize..]);
        if let Err(source) = self.reader.read_exact(&mut encoded[HEADER_SIZE as usize..]) {
            return self.io_error(source);
        }
        let checksum = u128::from(cityhash128(&encoded)).to_le_bytes();
        if header.prefix[..CHECKSUM_SIZE as usize] != checksum {
            return self.invalid_at(block.offset, "checksum mismatch");
        }

        let data = match block.method {
            0x02 => {
                if encoded.len() - HEADER_SIZE as usize != block.decompressed_size as usize {
                    return self.invalid_at(block.offset, "uncompressed size mismatch");
                }
                encoded.copy_within(HEADER_SIZE as usize.., 0);
                encoded.truncate(block.decompressed_size as usize);
                encoded
            }
            0x82 | 0x90 => {
                let decoded = decode_codec_payload(
                    block.method,
                    &encoded[HEADER_SIZE as usize..],
                    block.decompressed_size as usize,
                );
                self.encoded_scratch = encoded;
                match decoded {
                    Ok(decoded) => decoded,
                    Err(reason) => return self.invalid_at(block.offset, reason),
                }
            }
            _ => return self.invalid_at(block.offset, "unsupported compression method"),
        };
        Some(Ok(DecodedBlock { block, data }))
    }

    fn invalid_at<T>(&mut self, offset: u64, reason: &'static str) -> Option<Result<T, PartError>> {
        self.done = true;
        Some(Err(PartError::InvalidCompressedBlock {
            path: self.path.clone(),
            offset,
            reason,
        }))
    }

    fn invalid<T>(&mut self, reason: &'static str) -> Option<Result<T, PartError>> {
        self.invalid_at(self.offset, reason)
    }

    fn io_error<T>(&mut self, source: io::Error) -> Option<Result<T, PartError>> {
        self.done = true;
        Some(Err(PartError::Io {
            path: self.path.clone(),
            source,
        }))
    }
}

fn decode_codec_payload(
    method: u8,
    payload: &[u8],
    decompressed_size: usize,
) -> Result<Vec<u8>, &'static str> {
    let mut decoded = vec![0u8; decompressed_size];
    let size = match method {
        0x82 => decompress_into(payload, &mut decoded).ok(),
        0x90 => zstd::bulk::decompress_to_buffer(payload, &mut decoded).ok(),
        _ => return Err("unsupported compression method"),
    };
    if size == Some(decoded.len()) {
        Ok(decoded)
    } else if method == 0x82 {
        Err("invalid LZ4 payload or decompressed size")
    } else {
        Err("invalid ZSTD payload or decompressed size")
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
    fn adjacent_blocks_skip_seeking_without_skipping_validation() {
        use std::cell::Cell;
        use std::rc::Rc;

        struct CountingSeek {
            data: Cursor<Vec<u8>>,
            seeks: Rc<Cell<usize>>,
        }
        impl Read for CountingSeek {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                self.data.read(out)
            }
        }
        impl Seek for CountingSeek {
            fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
                self.seeks.set(self.seeks.get() + 1);
                self.data.seek(position)
            }
        }

        let mut data = checked_block(0x02, b"first", 5);
        let second_offset = data.len() as u64;
        data.extend(checked_block(0x02, b"second", 6));
        let seeks = Rc::new(Cell::new(0));
        let mut reader = CompressedBlockReader::new(
            CountingSeek { data: Cursor::new(data), seeks: Rc::clone(&seeks) },
            "test.bin",
        ).unwrap();
        assert_eq!(reader.next_decoded().unwrap().unwrap().data, b"first");
        let before = seeks.get();
        reader.seek_to_block(second_offset).unwrap();
        assert_eq!(seeks.get(), before);
        assert_eq!(reader.next_decoded().unwrap().unwrap().data, b"second");
        reader.seek_to_block(0).unwrap();
        assert!(seeks.get() > before);
        assert_eq!(reader.next_decoded().unwrap().unwrap().data, b"first");
    }

    #[test]
    fn file_open_decodes_blocks_from_the_start() {
        let path = std::env::temp_dir().join(format!(
            "rusthouse-compressed-open-{}.bin",
            std::process::id()
        ));
        std::fs::write(&path, checked_block(0x02, b"hello", 5)).unwrap();
        let mut reader = CompressedBlockReader::open(&path).unwrap();
        assert_eq!(reader.next_decoded().unwrap().unwrap().data, b"hello");
        assert!(reader.next_decoded().is_none());
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_compressed_streams() {
        use std::os::unix::fs::symlink;

        let dir = std::env::temp_dir().join(format!(
            "rusthouse-compressed-symlink-{}",
            std::process::id()
        ));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("real.bin"), b"data").unwrap();
        symlink("real.bin", dir.join("link.bin")).unwrap();
        assert!(matches!(
            CompressedBlockReader::open(dir.join("link.bin")),
            Err(PartError::Io { source, .. }) if source.kind() == io::ErrorKind::InvalidInput
        ));
        std::fs::remove_dir_all(dir).unwrap();
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
    fn reuses_encoded_buffer_across_compressed_and_raw_blocks() {
        let data = b"compressed column blocks share a scratch allocation ".repeat(64);
        let compressed = lz4_flex::block::compress(&data);
        let mut bytes = checked_block(0x82, &compressed, data.len() as u32);
        bytes.extend(checked_block(0x02, b"raw", 3));
        bytes.extend(checked_block(0x82, &compressed, data.len() as u32));
        let mut reader = CompressedBlockReader::new(Cursor::new(bytes), "mixed.bin").unwrap();

        assert_eq!(reader.next_decoded().unwrap().unwrap().data, data);
        let address = reader.encoded_scratch.as_ptr();
        assert!(reader.encoded_scratch.capacity() >= compressed.len() + HEADER_SIZE as usize);
        assert_eq!(reader.next_decoded().unwrap().unwrap().data, b"raw");
        assert_eq!(reader.encoded_scratch.as_ptr(), address);
        assert_eq!(reader.next_decoded().unwrap().unwrap().data, data);
        assert_eq!(reader.encoded_scratch.as_ptr(), address);
        assert!(reader.next_decoded().is_none());
    }

    #[test]
    fn reuses_encoded_buffer_for_zstd_frames() {
        let data = b"Repeated ZSTD block bytes ".repeat(80);
        let payload = zstd::bulk::compress(&data, 3).unwrap();
        let mut bytes = checked_block(0x90, &payload, data.len() as u32);
        bytes.extend(checked_block(0x90, &payload, data.len() as u32));
        let mut reader = CompressedBlockReader::new(Cursor::new(bytes), "zstd.bin").unwrap();

        assert_eq!(reader.next_decoded().unwrap().unwrap().data, data);
        let address = reader.encoded_scratch.as_ptr();
        assert_eq!(reader.next_decoded().unwrap().unwrap().data, data);
        assert_eq!(reader.encoded_scratch.as_ptr(), address);
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
            checked_block(0x82, valid, 100),
            checked_block(0x82, valid, 2),
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
                checked_block(0xfe, b"unknown", 7),
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

    #[test]
    fn decodes_native_zstd_frame_and_continues_to_next_block() {
        let frame = include_bytes!("../tests/fixtures/blocks/zstd-frame.bin");
        let mut bytes = checked_block(0x90, frame, 301);
        let second_offset = bytes.len() as u64;
        bytes.extend(checked_block(0x02, b"done", 4));

        let mut reader = CompressedBlockReader::new(Cursor::new(bytes), "data.bin").unwrap();
        let first = reader.next_decoded().unwrap().unwrap();
        assert_eq!(first.block.offset, 0);
        assert_eq!(first.block.method, 0x90);
        assert_eq!(
            first.data,
            b"ClickHouse marks are compressed with ZSTD. ".repeat(7)
        );

        let second = reader.next_decoded().unwrap().unwrap();
        assert_eq!(second.block.offset, second_offset);
        assert_eq!(second.data, b"done");
        assert!(reader.next_decoded().is_none());
    }

    #[test]
    fn rejects_invalid_zstd_frames_and_wrong_decompressed_sizes() {
        let frame = include_bytes!("../tests/fixtures/blocks/zstd-frame.bin");
        for (payload, decompressed_size) in [
            (&[0xff][..], 301),
            (&frame[..frame.len() - 1], 301),
            (&frame[..], 300),
            (&frame[..], 302),
        ] {
            let bytes = checked_block(0x90, payload, decompressed_size);
            let mut reader = CompressedBlockReader::new(Cursor::new(bytes), "bad.bin").unwrap();
            assert!(matches!(
                reader.next_decoded(),
                Some(Err(PartError::InvalidCompressedBlock {
                    reason: "invalid ZSTD payload or decompressed size",
                    ..
                }))
            ));
            assert!(reader.next_decoded().is_none());
        }
    }

    #[test]
    fn validates_zstd_checksum_and_decode_size_before_decompression() {
        let frame = include_bytes!("../tests/fixtures/blocks/zstd-frame.bin");
        let mut bytes = checked_block(0x90, frame, 301);
        bytes[PREFIX_SIZE] ^= 1;
        let mut reader = CompressedBlockReader::new(Cursor::new(bytes), "bad.bin").unwrap();
        assert!(matches!(
            reader.next_decoded(),
            Some(Err(PartError::InvalidCompressedBlock {
                reason: "checksum mismatch",
                ..
            }))
        ));

        let bytes = checked_block(0x90, frame, MAX_DECODE_BLOCK_SIZE + 1);
        let mut reader = CompressedBlockReader::new(Cursor::new(bytes), "bad.bin").unwrap();
        assert!(matches!(
            reader.next_decoded(),
            Some(Err(PartError::InvalidCompressedBlock {
                reason: "block exceeds the 64 MiB decode limit",
                ..
            }))
        ));
    }
}

#[cfg(test)]
mod unsupported_codec_early_rejection_tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn rejects_unsupported_header_before_reading_payload() {
        let mut bytes = vec![0u8; 16];
        bytes.push(0xff);
        bytes.extend_from_slice(&19u32.to_le_bytes());
        bytes.extend_from_slice(&10u32.to_le_bytes());
        // A complete but untrusted payload must not be decoded or hashed.
        bytes.extend_from_slice(&[0u8; 10]);
        let cursor = Cursor::new(bytes);
        let mut reader = CompressedBlockReader::new(cursor, "unknown-codec").unwrap();
        assert!(matches!(
            reader.next_decoded(),
            Some(Err(PartError::InvalidCompressedBlock {
                reason: "unsupported compression method",
                ..
            }))
        ));
    }
}

#[cfg(all(test, unix))]
mod compressed_stream_identity_tests {
    use super::*;
    use std::os::unix::fs::MetadataExt;

    #[test]
    fn inode_identity_detects_same_size_replacement_files() {
        let root = std::env::temp_dir().join(format!(
            "rusthouse-block-inode-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let first = root.join("one.bin");
        let second = root.join("two.bin");
        std::fs::write(&first, [0u8; 25]).unwrap();
        std::fs::write(&second, [1u8; 25]).unwrap();
        let old = std::fs::symlink_metadata(&first).unwrap();
        let replacement = File::open(&second).unwrap().metadata().unwrap();
        assert_eq!(old.len(), replacement.len());
        assert_ne!(old.ino(), replacement.ino());
        std::fs::remove_dir_all(root).unwrap();
    }
}
