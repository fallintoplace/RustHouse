//! Safe, relative identifiers for replicated immutable part files.
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectKey {
    pub cluster: String,
    pub shard: String,
    pub generation: u64,
    pub part: String,
    pub file: String,
}

fn component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

impl ObjectKey {
    /// Build a key without admitting path separators, traversal or control bytes.
    pub fn new(
        cluster: &str,
        shard: &str,
        generation: u64,
        part: &str,
        file: &str,
    ) -> Result<Self, &'static str> {
        if !component(cluster) || !component(shard) || !component(part) || !component(file) {
            return Err("invalid distributed object key component");
        }
        if generation == 0 {
            return Err("distributed generations must be positive");
        }
        Ok(Self {
            cluster: cluster.to_owned(),
            shard: shard.to_owned(),
            generation,
            part: part.to_owned(),
            file: file.to_owned(),
        })
    }

    /// All segments are validated before they reach the filesystem.
    pub fn relative_path(&self) -> PathBuf {
        PathBuf::from(&self.cluster)
            .join(&self.shard)
            .join(format!("g{}", self.generation))
            .join(&self.part)
            .join(&self.file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructs_stable_generation_scoped_keys() {
        let key = ObjectKey::new("prod", "shard_03", 12, "all_4_4_0", "checksums.txt").unwrap();
        assert_eq!(
            key.relative_path(),
            PathBuf::from("prod/shard_03/g12/all_4_4_0/checksums.txt")
        );
    }

    #[test]
    fn rejects_traversal_and_unsafe_names() {
        for invalid in ["", ".", "..", "../secrets", "/tmp", "foo/bar", ".hidden", "a..b", "a\\b", "x\n"] {
            assert!(ObjectKey::new(invalid, "s1", 1, "all_0_0_0", "count.txt").is_err());
            assert!(ObjectKey::new("prod", "s1", 1, "all_0_0_0", invalid).is_err());
        }
        assert!(ObjectKey::new("prod", "s1", 0, "all_0_0_0", "count.txt").is_err());
    }
}