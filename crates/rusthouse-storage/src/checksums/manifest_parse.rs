use super::*;

// Check borrowed names after parsing, so the hash table never allocates
// an additional String for every immutable object in a large manifest.
pub(super) fn has_duplicate_file_names(files: &[FileChecksum]) -> bool {
    if files.len() <= 8 {
        return files
            .iter()
            .enumerate()
            .any(|(index, file)| files[..index].iter().any(|prior| prior.name == file.name));
    }
    let mut names = HashSet::with_capacity(files.len().min(128));
    files.iter().any(|file| !names.insert(file.name.as_str()))
}

pub(super) fn parse_checksums(content: &[u8], path: &Path) -> Result<PartChecksums, PartError> {
    let (version, payload) = parse_checksum_header(content, path)?;
    let decoded = if version == 4 {
        Some(decode_compressed_checksums(payload, path)?)
    } else {
        None
    };
    let bytes = decoded.as_deref().unwrap_or(payload);
    let mut reader = BinaryReader::new(bytes, path);
    let count = usize::try_from(reader.varuint()?)
        .map_err(|_| invalid(path, "file count exceeds address space"))?;
    if count > MAX_MANIFEST_FILES {
        return Err(invalid(path, "manifest file count exceeds limit"));
    }
    if count > reader.remaining() / 20 {
        return Err(invalid(path, "file count exceeds remaining data"));
    }

    let mut files = Vec::with_capacity(count.min(128));
    for _ in 0..count {
        files.push(reader.file_checksum()?);
    }
    if has_duplicate_file_names(&files) {
        return Err(invalid(path, "duplicate file name"));
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

    fn file_checksum(&mut self) -> Result<FileChecksum, PartError> {
        let length = usize::try_from(self.varuint()?)
            .map_err(|_| invalid(self.path, "file name length exceeds address space"))?;
        let name = std::str::from_utf8(self.take(length)?)
            .map_err(|_| invalid(self.path, "file name is not UTF-8"))?;
        if !valid_name(name) {
            return Err(invalid(self.path, "unsafe or empty file name"));
        }
        let size = self.varuint()?;
        let hash = self.hash()?;
        let uncompressed = match self.take(1)?[0] {
            0 => None,
            1 => Some(UncompressedChecksum {
                size: self.varuint()?,
                hash: self.hash()?,
            }),
            _ => return Err(invalid(self.path, "invalid compressed flag")),
        };
        Ok(FileChecksum {
            name: name.to_owned(),
            size,
            hash,
            uncompressed,
        })
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
