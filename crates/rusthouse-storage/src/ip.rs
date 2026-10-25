use std::io::{self, Read};
use std::net::{Ipv4Addr, Ipv6Addr};

const MAX_BATCH_BYTES: usize = 64 * 1024 * 1024;

/// Decode ClickHouse IPv4's little-endian UInt32 physical representation.
pub fn read_ipv4_values<R: Read>(reader: &mut R, rows: usize) -> io::Result<Vec<Ipv4Addr>> {
    if rows > MAX_BATCH_BYTES / 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "IPv4 batch exceeds 64 MiB",
        ));
    }
    let mut result = Vec::with_capacity(rows);
    let mut bytes = [0u8; 4];
    for _ in 0..rows {
        reader.read_exact(&mut bytes)?;
        result.push(Ipv4Addr::from(u32::from_le_bytes(bytes)));
    }
    Ok(result)
}

/// Decode ClickHouse IPv6's 16-byte network-order physical representation.
pub fn read_ipv6_values<R: Read>(reader: &mut R, rows: usize) -> io::Result<Vec<Ipv6Addr>> {
    if rows > MAX_BATCH_BYTES / 16 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "IPv6 batch exceeds 64 MiB",
        ));
    }
    let mut result = Vec::with_capacity(rows);
    let mut bytes = [0u8; 16];
    for _ in 0..rows {
        reader.read_exact(&mut bytes)?;
        result.push(Ipv6Addr::from(bytes));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_ipv4_addresses_in_network_not_file_order() {
        let bytes = [1, 2, 0, 192, 8, 8, 8, 8];
        let result = read_ipv4_values(&mut bytes.as_slice(), 2).unwrap();
        assert_eq!(result[0].to_string(), "192.0.2.1");
        assert_eq!(result[1].to_string(), "8.8.8.8");
        assert_eq!(read_ipv4_values(&mut [].as_slice(), 0).unwrap().len(), 0);
    }

    #[test]
    fn reads_ipv6_network_order_bytes() {
        let bytes = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        let addresses = read_ipv6_values(&mut bytes.as_slice(), 1).unwrap();
        assert_eq!(addresses[0].to_string(), "2001:db8::1");
    }

    #[test]
    fn bounds_allocations_and_fails_on_truncated_addresses() {
        assert_eq!(
            read_ipv4_values(&mut [].as_slice(), MAX_BATCH_BYTES / 4 + 1)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            read_ipv6_values(&mut [].as_slice(), MAX_BATCH_BYTES / 16 + 1)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            read_ipv4_values(&mut [1, 2, 3].as_slice(), 1)
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert_eq!(
            read_ipv6_values(&mut [0; 15].as_slice(), 1)
                .unwrap_err()
                .kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}
