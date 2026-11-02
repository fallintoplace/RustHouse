use crate::fixed_width::MAX_BATCH_BYTES;
use crate::fixed_width::read_fixed_width;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr};

/// Decode ClickHouse IPv4's little-endian UInt32 physical representation.
pub fn read_ipv4_values<R: Read>(reader: &mut R, rows: usize) -> io::Result<Vec<Ipv4Addr>> {
    read_fixed_width(reader, rows, "IPv4 batch exceeds 64 MiB", |bytes| {
        Ipv4Addr::from(u32::from_le_bytes(bytes))
    })
}

/// Decode ClickHouse IPv6's 16-byte network-order physical representation.
pub fn read_ipv6_values<R: Read>(reader: &mut R, rows: usize) -> io::Result<Vec<Ipv6Addr>> {
    read_fixed_width(reader, rows, "IPv6 batch exceeds 64 MiB", Ipv6Addr::from)
}

fn validate_ip_write_batch(rows: usize, width: usize) -> io::Result<()> {
    let bytes = rows.checked_mul(width).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "IP batch size overflow")
    })?;
    if bytes > MAX_BATCH_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "IP batch exceeds 64 MiB",
        ));
    }
    Ok(())
}

/// Write ClickHouse IPv4 UInt32 values in little-endian disk order.
/// Use bounded fixed-size blocks regardless of the total number of rows.
pub fn write_ipv4_values<W: Write>(writer: &mut W, addresses: &[Ipv4Addr]) -> io::Result<()> {
    validate_ip_write_batch(addresses.len(), 4)?;
    let mut buffer = [0u8; 16 * 1024];
    for chunk in addresses.chunks(buffer.len() / 4) {
        for (index, &address) in chunk.iter().enumerate() {
            buffer[index * 4..index * 4 + 4]
                .copy_from_slice(&u32::from(address).to_le_bytes());
        }
        writer.write_all(&buffer[..chunk.len() * 4])?;
    }
    Ok(())
}

/// Write IPv6 addresses in ClickHouse's network-byte-order disk encoding.
pub fn write_ipv6_values<W: Write>(writer: &mut W, addresses: &[Ipv6Addr]) -> io::Result<()> {
    validate_ip_write_batch(addresses.len(), 16)?;
    let mut buffer = [0u8; 16 * 1024];
    for chunk in addresses.chunks(buffer.len() / 16) {
        for (index, &address) in chunk.iter().enumerate() {
            buffer[index * 16..index * 16 + 16].copy_from_slice(&address.octets());
        }
        writer.write_all(&buffer[..chunk.len() * 16])?;
    }
    Ok(())
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

    #[test]
    fn batches_address_reads_and_handles_short_readers() {
        struct CountingReader<R> {
            inner: R,
            calls: usize,
        }

        impl<R: Read> Read for CountingReader<R> {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                self.calls += 1;
                self.inner.read(output)
            }
        }

        let ipv4 = [1u8, 2, 0, 192].repeat(8192);
        let mut reader = CountingReader {
            inner: ipv4.as_slice(),
            calls: 0,
        };
        let result = read_ipv4_values(&mut reader, 8192).unwrap();
        assert_eq!(reader.calls, 1);
        assert_eq!(result[8191], Ipv4Addr::new(192, 0, 2, 1));

        let ipv6 = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        let ipv6_values = ipv6.repeat(2048);
        let mut reader = CountingReader {
            inner: ipv6_values.as_slice(),
            calls: 0,
        };
        let result = read_ipv6_values(&mut reader, 2048).unwrap();
        assert_eq!(reader.calls, 1);
        assert_eq!(result[2047], Ipv6Addr::from(ipv6));

        struct Bytewise<R>(R);

        impl<R: Read> Read for Bytewise<R> {
            fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
                let count = output.len().min(1);
                self.0.read(&mut output[..count])
            }
        }

        assert_eq!(
            read_ipv4_values(&mut Bytewise(&[1u8, 2, 0, 192][..]), 1).unwrap(),
            vec![Ipv4Addr::new(192, 0, 2, 1)]
        );
    }
}

#[cfg(test)]
mod physical_ip_write_roundtrip_tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    struct ShortWriter(Vec<u8>);
    impl Write for ShortWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let count = bytes.len().min(7);
            self.0.extend_from_slice(&bytes[..count]);
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }

    #[test]
    fn physical_ipv4_bytes_and_short_writes_round_trip() {
        let values = [Ipv4Addr::new(192, 0, 2, 1), Ipv4Addr::new(8, 8, 8, 8)];
        let mut writer = ShortWriter(Vec::new());
        write_ipv4_values(&mut writer, &values).unwrap();
        assert_eq!(writer.0, [1, 2, 0, 192, 8, 8, 8, 8]);
        assert_eq!(read_ipv4_values(&mut writer.0.as_slice(), 2).unwrap(), values);
    }

    #[test]
    fn physical_ipv6_buffer_rollover_matches_reading() {
        let addresses = (0..2050u32)
            .map(|n| Ipv6Addr::from(n as u128 * 100_003))
            .collect::<Vec<_>>();
        let mut output = Vec::new();
        write_ipv6_values(&mut output, &addresses).unwrap();
        assert_eq!(&output[..16], &addresses[0].octets());
        assert_eq!(output.len(), addresses.len() * 16);
        assert_eq!(
            read_ipv6_values(&mut output.as_slice(), addresses.len()).unwrap(),
            addresses
        );
    }

    #[test]
    fn invalid_batch_preflight_never_writes_to_a_destination() {
        assert!(validate_ip_write_batch(usize::MAX, 16).is_err());
        assert!(validate_ip_write_batch(MAX_BATCH_BYTES / 4 + 1, 4).is_err());
        assert!(validate_ip_write_batch(MAX_BATCH_BYTES / 16, 16).is_ok());
        let mut output = Vec::new();
        write_ipv4_values(&mut output, &[]).unwrap();
        write_ipv6_values(&mut output, &[]).unwrap();
        assert!(output.is_empty());
    }
}
