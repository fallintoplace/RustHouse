//! Safe, relative identifiers for replicated immutable part files.
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectKey {
    cluster: String,
    shard: String,
    generation: u64,
    part: String,
    file: String,
}

fn component(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 128 || !bytes[0].is_ascii_alphanumeric() {
        return false;
    }
    bytes.iter().all(|&byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
    }) && bytes.last() != Some(&b'.')
        && !bytes
            .windows(2)
            .any(|pair| pair[0] == b'.' && pair[1] == b'.')
}

impl ObjectKey {
    pub fn cluster(&self) -> &str {
        &self.cluster
    }

    pub fn shard(&self) -> &str {
        &self.shard
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn part(&self) -> &str {
        &self.part
    }

    pub fn file(&self) -> &str {
        &self.file
    }

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

    /// Parse a generation-scoped key from five safe relative path components.
    /// Reject noncanonical generation spellings and path traversal.
    pub fn parse_relative(path: &std::path::Path) -> Result<Self, &'static str> {
        use std::path::Component;
        // Path components borrow the original Path. Keep the five fields on
        // the stack instead of allocating a Vec for each replicated object.
        let mut segments = path.components();
        let mut fields = [""; 5];
        for field in &mut fields {
            let Some(Component::Normal(value)) = segments.next() else {
                return Err("distributed object key must have five relative components");
            };
            *field = value.to_str().ok_or("non-UTF8 distributed object key")?;
        }
        if segments.next().is_some() {
            return Err("distributed object key must have five components");
        }
        let generation_text = fields[2]
            .strip_prefix('g')
            .ok_or("missing distributed generation prefix")?;
        let generation = generation_text
            .parse::<u64>()
            .map_err(|_| "invalid distributed generation")?;
        if generation_text != generation.to_string() {
            return Err("noncanonical distributed generation");
        }
        let key = Self::new(fields[0], fields[1], generation, fields[3], fields[4])?;
        let original = path.to_str().ok_or("invalid distributed object key path")?;
        let canonical = key.relative_path();
        let canonical = canonical.to_str().ok_or("invalid distributed object key path")?;
        // Unix paths use '/' natively. Comparing borrowed strings avoids
        // allocating two full replacement paths for every object key.
        #[cfg(not(windows))]
        if original != canonical {
            return Err("noncanonical distributed object key path");
        }
        // Windows accepts both slash spellings. Preserve normalized
        // comparison without discarding duplicate or dot components.
        #[cfg(windows)]
        if original.replace(std::path::MAIN_SEPARATOR, "/")
            != canonical.replace(std::path::MAIN_SEPARATOR, "/")
        {
            return Err("noncanonical distributed object key path");
        }
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructs_stable_generation_scoped_keys() {
        let key = ObjectKey::new("prod", "shard_03", 12, "all_4_4_0", "checksums.txt").unwrap();
        assert_eq!(key.cluster(), "prod");
        assert_eq!(key.shard(), "shard_03");
        assert_eq!(key.generation(), 12);
        assert_eq!(key.part(), "all_4_4_0");
        assert_eq!(key.file(), "checksums.txt");
        assert_eq!(
            key.relative_path(),
            PathBuf::from("prod/shard_03/g12/all_4_4_0/checksums.txt")
        );
    }

    #[test]
    fn rejects_traversal_and_unsafe_names() {
        for invalid in [
            "",
            ".",
            "..",
            "../secrets",
            "/tmp",
            "foo/bar",
            ".hidden",
            "a..b",
            "a\\b",
            "x\n",
        ] {
            assert!(ObjectKey::new(invalid, "s1", 1, "all_0_0_0", "count.txt").is_err());
            assert!(ObjectKey::new("prod", "s1", 1, "all_0_0_0", invalid).is_err());
        }
        assert!(ObjectKey::new("prod", "s1", 0, "all_0_0_0", "count.txt").is_err());
    }
}

#[cfg(test)]
mod parse_relative_tests {
    use super::*;

    #[test]
    fn canonical_paths_accept_many_simple_keys_without_normalization() {
        for generation in 1..128u64 {
            let key = ObjectKey::new("cluster", "shard_1", generation, "part_10", "data.bin").unwrap();
            assert_eq!(ObjectKey::parse_relative(&key.relative_path()).unwrap(), key);
        }
        assert!(ObjectKey::parse_relative(
            std::path::Path::new("cluster/shard_1/g1/part_10/data.bin")
        ).is_ok());
    }

    #[test]
    fn component_iterator_requires_exactly_five_safe_components() {
        let valid = std::path::Path::new("prod/shard_4/g432/part_1/data.bin");
        let key = ObjectKey::parse_relative(valid).unwrap();
        assert_eq!(key.generation(), 432);
        for invalid in [
            "prod/shard_4/g432/part_1",
            "prod/shard_4/g432/part_1/data.bin/extra",
            "prod/shard_4/g432/part_1/../data.bin",
            "/prod/shard_4/g432/part_1/data.bin",
            "prod/shard_4/g0432/part_1/data.bin",
        ] {
            assert!(ObjectKey::parse_relative(std::path::Path::new(invalid)).is_err(), "{invalid}");
        }
    }

    #[test]
    fn round_trips_generation_scoped_paths() {
        let key = ObjectKey::new("cluster", "s1", 42, "all_0_0_0", "data.bin").unwrap();
        assert_eq!(ObjectKey::parse_relative(&key.relative_path()).unwrap(), key);
    }

    #[test]
    fn rejects_unsafe_and_ambiguous_paths() {
        for path in [
            "/cluster/s1/g1/part/file",
            "cluster/s1/g0/part/file",
            "cluster/s1/g01/part/file",
            "cluster/s1/gX/part/file",
            "cluster/s1/g1/../file",
            "cluster/s1/g1/./part/file",
            "cluster//s1/g1/part/file",
            "cluster/s1/g1/part/file/",
            "cluster/s1/g1/.hidden/file",
            "cluster/s1/g1/part",
            "cluster/s1/g1/part/file/extra",
        ] {
            assert!(ObjectKey::parse_relative(PathBuf::from(path).as_path()).is_err(), "{path}");
        }
    }
}

#[cfg(test)]
mod trailing_dot_key_tests {
    use super::*;

    #[test]
    fn refuses_platform_ambiguous_trailing_dots() {
        for component in ["part.", "part..", "data.bin."] {
            assert!(ObjectKey::new("cluster", "s1", 1, component, "data.bin").is_err());
            assert!(ObjectKey::new("cluster", "s1", 1, "part", component).is_err());
        }
        assert!(ObjectKey::new("cluster", "s1", 1, "part", "data.bin").is_ok());
    }
}
