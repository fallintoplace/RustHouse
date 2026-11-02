use super::*;

impl ResumeMap {
    pub fn complete(&self) -> bool {
        let full_bytes = self.chunks / 8;
        if !all_full_bytes_set(&self.completed[..full_bytes]) {
            return false;
        }
        let remainder = self.chunks % 8;
        if remainder == 0 {
            return true;
        }
        let mask = low_bit_mask(remainder);
        self.completed[full_bytes] & mask == mask
    }

    /// Stable ASCII encoding for persisting transfer progress.
    pub fn encode(&self) -> String {
        // Dense maps are expanded eight flags at a time without first filling
        // a separate zeroed output. Sparse maps keep the set-bit walk below.
        if self.chunks >= 256 && self.completed_count() >= self.chunks / 2 {
            let mut encoded = Vec::with_capacity(self.chunks);
            let full_bytes = self.chunks / 8;
            for &byte in &self.completed[..full_bytes] {
                encoded.extend_from_slice(&ASCII_BITMAP_BYTES[usize::from(byte)]);
            }
            let tail = self.chunks % 8;
            if tail != 0 {
                encoded.extend_from_slice(
                    &ASCII_BITMAP_BYTES[usize::from(self.completed[full_bytes])][..tail],
                );
            }
            return String::from_utf8(encoded).expect("binary map contains only ASCII digits");
        }
        // Fill the output once. Only set completion positions, instead of
        // appending and masking each of the up to 65,536 input bits.
        let mut encoded = vec![b'0'; self.chunks];
        for (byte_index, &completed) in self.completed.iter().enumerate() {
            let mut bits = completed;
            while bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                let index = byte_index * 8 + bit;
                if index < self.chunks {
                    encoded[index] = b'1';
                }
                bits &= bits - 1;
            }
        }
        String::from_utf8(encoded).expect("binary map contains only ASCII digits")
    }

    pub fn decode(text: &str) -> Result<Self, &'static str> {
        let mut result = Self::new(text.len())?;
        for (byte_index, group) in text.as_bytes().chunks(8).enumerate() {
            let mut flags = 0u8;
            for (bit, &value) in group.iter().enumerate() {
                match value {
                    b'0' => {}
                    b'1' => flags |= 1u8 << bit,
                    _ => return Err("invalid resume map byte"),
                }
            }
            result.completed[byte_index] = flags;
        }
        Ok(result)
    }

    /// Merge verified progress for the same immutable object and chunk layout.
    /// The caller must authenticate that both maps refer to the same object.
    pub fn merge_verified(&mut self, other: &Self) -> Result<(), &'static str> {
        if self.chunks != other.chunks {
            return Err("cannot merge mismatched transfer layouts");
        }
        for (destination, &source) in self.completed.iter_mut().zip(&other.completed) {
            *destination |= source;
        }
        Ok(())
    }

    /// Retain only chunks that both authenticated checkpoints independently
    /// confirm as complete. No new completion bit can be introduced.
    ///
    /// Both maps must describe the same immutable object and chunk layout.
    /// The caller is responsible for authenticating object identity.
    pub fn retain_common_verified(&mut self, other: &Self) -> Result<(), &'static str> {
        if self.chunks != other.chunks {
            return Err("cannot intersect mismatched transfer layouts");
        }
        for (destination, &source) in self.completed.iter_mut().zip(&other.completed) {
            *destination &= source;
        }
        Ok(())
    }

    /// Write an authenticated-by-checksum checkpoint envelope.
    ///
    /// A flipped completion bit must never silently skip an unverified chunk.
    /// CRC32C detects accidental corruption, not malicious tampering. The
    /// original compact binary format remains available for compatibility.
    pub fn encode_binary_checked(&self) -> Vec<u8> {
        let mut encoded = Vec::with_capacity(12 + self.completed.len());
        encoded.extend_from_slice(b"RSM1");
        encoded.extend_from_slice(&(self.chunks as u32).to_le_bytes());
        encoded.extend_from_slice(&self.completed);
        let checksum = crate::distributed_crc32c::crc32c(&encoded);
        encoded.extend_from_slice(&checksum.to_le_bytes());
        encoded
    }

    /// Verify framing and checksum before trusting any completion flag.
    pub fn decode_binary_checked(encoded: &[u8]) -> Result<Self, &'static str> {
        if encoded.get(..4) != Some(b"RSM1".as_slice()) {
            return Err("invalid resume checkpoint magic");
        }
        let count = encoded.get(4..8).ok_or("truncated resume checkpoint")?;
        let chunks = u32::from_le_bytes(
            count
                .try_into()
                .map_err(|_| "invalid resume checkpoint header")?,
        ) as usize;
        // The checksum envelope only needs the encoded bitmap length.
        // Do not allocate a throwaway ResumeMap before decode_binary builds
        // the validated result.
        if chunks > 65_536 {
            return Err("resume map exceeds chunk limit");
        }
        let body_end = 8 + chunks.div_ceil(8);
        if encoded.len() != body_end + 4 {
            return Err("invalid resume checkpoint length");
        }
        let received = u32::from_le_bytes(
            encoded[body_end..body_end + 4]
                .try_into()
                .map_err(|_| "invalid resume checkpoint checksum")?,
        );
        if crate::distributed_crc32c::crc32c(&encoded[..body_end]) != received {
            return Err("resume checkpoint checksum mismatch");
        }
        // Reuse the strict packed-bitmap decoder, including unused tail bits.
        Self::decode_binary(&encoded[4..body_end])
    }

    /// Serialize the exact chunk count and completion bitmap.
    /// Four little-endian count bytes are followed by the packed flags.
    pub fn encode_binary(&self) -> Vec<u8> {
        let mut encoded = Vec::with_capacity(4 + self.completed.len());
        encoded.extend_from_slice(&(self.chunks as u32).to_le_bytes());
        encoded.extend_from_slice(&self.completed);
        encoded
    }

    /// Reject truncated, oversized and noncanonical checkpoint bitmaps.
    pub fn decode_binary(bytes: &[u8]) -> Result<Self, &'static str> {
        let count = bytes.get(..4).ok_or("truncated resume checkpoint")?;
        let chunks = u32::from_le_bytes(
            count
                .try_into()
                .map_err(|_| "invalid resume checkpoint header")?,
        ) as usize;
        let mut map = Self::new(chunks)?;
        if bytes.len() != 4 + map.completed.len() {
            return Err("invalid resume checkpoint length");
        }
        map.completed.copy_from_slice(&bytes[4..]);
        let tail = chunks % 8;
        if tail != 0
            && map
                .completed
                .last()
                .is_some_and(|flags| *flags >> tail != 0)
        {
            return Err("resume checkpoint has invalid tail bits");
        }
        Ok(map)
    }
}
