use crate::{CompressedBlockReader, PartError};
use std::collections::HashSet;
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

const HEADER: &[u8] = b"checksums format version: ";
const MAX_CHECKSUMS_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UncompressedChecksum {
    pub size: u64,
    pub hash: u128,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChecksum {
    pub name: String,
    pub size: u64,
    pub hash: u128,
    pub uncompressed: Option<UncompressedChecksum>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartChecksums {
    pub path: PathBuf,
    pub version: u8,
    pub files: Vec<FileChecksum>,
}

pub fn read_checksums(path: impl AsRef<Path>) -> Result<PartChecksums, PartError> {
    let path = path.as_ref();
    let file = File::open(path).map_err(|source| PartError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut content = Vec::new();
    file.take(MAX_CHECKSUMS_BYTES + 1)
        .read_to_end(&mut content)
        .map_err(|source| PartError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if content.len() as u64 > MAX_CHECKSUMS_BYTES {
        return Err(invalid(path, "checksums file exceeds 64 MiB"));
    }
    parse_checksums(&content, path)
}

fn parse_checksums(content: &[u8], path: &Path) -> Result<PartChecksums, PartError> {
    let header = content
        .strip_prefix(HEADER)
        .ok_or_else(|| invalid(path, "missing format header"))?;
    let end = header
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or_else(|| invalid(path, "unterminated format header"))?;
    let version = match &header[..end] {
        b"3" => 3,
        b"4" => 4,
        _ => return Err(invalid(path, "unsupported checksums format version")),
    };
    let payload = &header[end + 1..];
    let mut decoded = Vec::new();

    if version == 4 {
        let mut blocks = CompressedBlockReader::new(Cursor::new(payload), path)?;
        while let Some(block) = blocks.next_decoded() {
            let block = block?;
            if block.data.len() > MAX_CHECKSUMS_BYTES as usize - decoded.len() {
                return Err(invalid(path, "decoded checksums exceed 64 MiB"));
            }
            decoded.extend_from_slice(&block.data);
        }
    }

    let bytes = if version == 4 { &decoded[..] } else { payload };
    let mut reader = BinaryReader::new(bytes, path);
    let count = usize::try_from(reader.varuint()?)
        .map_err(|_| invalid(path, "file count exceeds address space"))?;
    if count > reader.remaining() / 20 {
        return Err(invalid(path, "file count exceeds remaining data"));
    }

    let mut files = Vec::with_capacity(count);
    let mut names = HashSet::with_capacity(count);
    for _ in 0..count {
        let length = usize::try_from(reader.varuint()?)
            .map_err(|_| invalid(path, "file name length exceeds address space"))?;
        let name = std::str::from_utf8(reader.take(length)?)
            .map_err(|_| invalid(path, "file name is not UTF-8"))?;
        if !valid_name(name) {
            return Err(invalid(path, "unsafe or empty file name"));
        }
        if !names.insert(name.to_owned()) {
            return Err(invalid(path, "duplicate file name"));
        }
        let size = reader.varuint()?;
        let hash = reader.hash()?;
        let uncompressed = match reader.take(1)?[0] {
            0 => None,
            1 => Some(UncompressedChecksum {
                size: reader.varuint()?,
                hash: reader.hash()?,
            }),
            _ => return Err(invalid(path, "invalid compressed flag")),
        };
        files.push(FileChecksum {
            name: name.to_owned(),
            size,
            hash,
            uncompressed,
        });
    }
    if reader.remaining() != 0 {
        return Err(invalid(path, "trailing checksums data"));
    }

    Ok(PartChecksums {
        path: path.to_path_buf(),
        version,
        files,
    })
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('\\')
        && !name.contains('\0')
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && !name.starts_with('/')
}

fn invalid(path: &Path, reason: &'static str) -> PartError {
    PartError::InvalidChecksums {
        path: path.to_path_buf(),
        reason,
    }
}

struct BinaryReader<'a> {
    content: &'a [u8],
    offset: usize,
    path: &'a Path,
}

impl<'a> BinaryReader<'a> {
    fn new(content: &'a [u8], path: &'a Path) -> Self {
        Self {
            content,
            offset: 0,
            path,
        }
    }

    fn remaining(&self) -> usize {
        self.content.len() - self.offset
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], PartError> {
        if length > self.remaining() {
            return Err(invalid(self.path, "truncated checksums data"));
        }
        let bytes = &self.content[self.offset..self.offset + length];
        self.offset += length;
        Ok(bytes)
    }

    fn hash(&mut self) -> Result<u128, PartError> {
        Ok(u128::from_le_bytes(self.take(16)?.try_into().unwrap()))
    }

    fn varuint(&mut self) -> Result<u64, PartError> {
        let mut value = 0u64;
        for index in 0..10 {
            let byte = self.take(1)?[0];
            if index == 9 && byte > 1 {
                return Err(invalid(self.path, "varUInt overflows u64"));
            }
            value |= u64::from(byte & 0x7f) << (index * 7);
            if byte & 0x80 == 0 {
                if index > 0 && byte == 0 {
                    return Err(invalid(self.path, "non-canonical varUInt"));
                }
                return Ok(value);
            }
        }
        Err(invalid(self.path, "varUInt overflows u64"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ch_cityhash102::cityhash128;

    fn varuint(mut value: u64, bytes: &mut Vec<u8>) {
        while value >= 128 {
            bytes.push((value as u8 & 0x7f) | 0x80);
            value >>= 7;
        }
        bytes.push(value as u8);
    }

    fn entry(
        bytes: &mut Vec<u8>,
        name: &str,
        size: u64,
        hash: u128,
        uncompressed: Option<(u64, u128)>,
    ) {
        varuint(name.len() as u64, bytes);
        bytes.extend_from_slice(name.as_bytes());
        varuint(size, bytes);
        bytes.extend_from_slice(&hash.to_le_bytes());
        match uncompressed {
            None => bytes.push(0),
            Some((size, hash)) => {
                bytes.push(1);
                varuint(size, bytes);
                bytes.extend_from_slice(&hash.to_le_bytes());
            }
        }
    }

    fn payload() -> Vec<u8> {
        let mut bytes = Vec::new();
        varuint(2, &mut bytes);
        entry(&mut bytes, "id.bin", 120, 42, Some((200, 64)));
        entry(&mut bytes, "proj.proj/columns.txt", 9, 0x1234, None);
        bytes
    }

    fn v3(payload: &[u8]) -> Vec<u8> {
        let mut bytes = b"checksums format version: 3\n".to_vec();
        bytes.extend_from_slice(payload);
        bytes
    }

    fn checked_block(method: u8, data: &[u8]) -> Vec<u8> {
        let encoded = match method {
            0x02 => data.to_vec(),
            0x82 => lz4_flex::block::compress(data),
            0x90 => zstd::bulk::compress(data, 1).unwrap(),
            _ => unreachable!(),
        };
        let mut bytes = vec![method];
        bytes.extend_from_slice(&(9u32 + encoded.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&encoded);
        let mut result = u128::from(cityhash128(&bytes)).to_le_bytes().to_vec();
        result.extend_from_slice(&bytes);
        result
    }

    fn v4(payload: &[u8], method: u8) -> Vec<u8> {
        let mut bytes = b"checksums format version: 4\n".to_vec();
        bytes.extend(checked_block(method, payload));
        bytes
    }

    #[test]
    fn reads_v3_binary_entries_and_uncompressed_metadata() {
        let checksums = parse_checksums(&v3(&payload()), Path::new("checksums.txt")).unwrap();
        assert_eq!(checksums.version, 3);
        assert_eq!(checksums.files.len(), 2);
        assert_eq!(
            checksums.files[0],
            FileChecksum {
                name: "id.bin".to_owned(),
                size: 120,
                hash: 42,
                uncompressed: Some(UncompressedChecksum {
                    size: 200,
                    hash: 64
                }),
            }
        );
        assert_eq!(checksums.files[1].name, "proj.proj/columns.txt");
        assert_eq!(checksums.files[1].uncompressed, None);
    }

    #[test]
    fn reads_v4_lz4_and_zstd_checked_blocks() {
        for method in [0x82, 0x90] {
            let checksums =
                parse_checksums(&v4(&payload(), method), Path::new("checksums.txt")).unwrap();
            assert_eq!(checksums.version, 4);
            assert_eq!(checksums.files.len(), 2);
            assert_eq!(checksums.files[0].hash, 42);
        }
    }

    #[test]
    fn reads_v4_entries_split_across_blocks() {
        let payload = payload();
        let mut bytes = b"checksums format version: 4\n".to_vec();
        bytes.extend(checked_block(0x82, &payload[..15]));
        bytes.extend(checked_block(0x02, &payload[15..]));
        let checksums = parse_checksums(&bytes, Path::new("checksums.txt")).unwrap();
        assert_eq!(checksums.files.len(), 2);
        assert_eq!(checksums.files[1].hash, 0x1234);
    }

    #[test]
    fn rejects_unsupported_header_and_trailing_data() {
        for data in [
            b"bad header".to_vec(),
            b"checksums format version: 2\n".to_vec(),
            b"checksums format version: 4".to_vec(),
        ] {
            assert!(matches!(
                parse_checksums(&data, Path::new("checksums.txt")),
                Err(PartError::InvalidChecksums { .. })
            ));
        }
        let mut data = payload();
        data.push(0);
        assert!(matches!(
            parse_checksums(&v3(&data), Path::new("checksums.txt")),
            Err(PartError::InvalidChecksums {
                reason: "trailing checksums data",
                ..
            })
        ));
    }

    #[test]
    fn rejects_unsafe_duplicate_and_invalid_utf8_names() {
        for name in [
            "../bad",
            "/absolute",
            "dir//file",
            "dir/./file",
            "dir/../file",
            "dir\\file",
            "bad\0name",
        ] {
            let mut bytes = vec![1];
            entry(&mut bytes, name, 0, 0, None);
            assert!(parse_checksums(&v3(&bytes), Path::new("checksums.txt")).is_err());
        }

        let mut bytes = vec![2];
        entry(&mut bytes, "same", 0, 0, None);
        entry(&mut bytes, "same", 1, 1, None);
        assert!(matches!(
            parse_checksums(&v3(&bytes), Path::new("checksums.txt")),
            Err(PartError::InvalidChecksums {
                reason: "duplicate file name",
                ..
            })
        ));

        let mut bytes = vec![1, 1, 0xff, 0];
        bytes.extend_from_slice(&[0; 16]);
        bytes.push(0);
        assert!(parse_checksums(&v3(&bytes), Path::new("checksums.txt")).is_err());
    }

    #[test]
    fn rejects_truncated_hash_bad_flags_and_varuint_overflows() {
        let mut bytes = vec![1];
        entry(&mut bytes, "x", 1, 0, None);
        bytes.pop();
        assert!(parse_checksums(&v3(&bytes), Path::new("checksums.txt")).is_err());

        let mut bytes = vec![1];
        entry(&mut bytes, "x", 1, 0, None);
        *bytes.last_mut().unwrap() = 2;
        assert!(matches!(
            parse_checksums(&v3(&bytes), Path::new("checksums.txt")),
            Err(PartError::InvalidChecksums {
                reason: "invalid compressed flag",
                ..
            })
        ));
        for bytes in [[0x80, 0x00].as_slice(), &[0xff; 10][..]] {
            assert!(parse_checksums(&v3(bytes), Path::new("checksums.txt")).is_err());
        }
        assert!(parse_checksums(&v3(&[2]), Path::new("checksums.txt")).is_err());
    }

    #[test]
    fn rejects_v4_corrupt_block_checksum() {
        let mut data = v4(&payload(), 0x02);
        *data.last_mut().unwrap() ^= 1;
        assert!(matches!(
            parse_checksums(&data, Path::new("checksums.txt")),
            Err(PartError::InvalidCompressedBlock {
                reason: "checksum mismatch",
                ..
            })
        ));
    }

    #[test]
    fn reads_empty_v3_and_v4_lists() {
        for data in [v3(&[0]), v4(&[0], 0x82)] {
            let checksums = parse_checksums(&data, Path::new("checksums.txt")).unwrap();
            assert!(checksums.files.is_empty());
        }
    }
}
