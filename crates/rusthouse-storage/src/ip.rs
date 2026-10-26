use crate::fixed_width::read_fixed_width;
#[cfg(test)]
use crate::fixed_width::MAX_BATCH_BYTES;
use std::io::{self, Read};
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
        assert_eq!(reader.calls, 2);
        assert_eq!(result[8191], Ipv4Addr::new(192, 0, 2, 1));

        let ipv6 = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        let ipv6_values = ipv6.repeat(2048);
        let mut reader = CountingReader {
            inner: ipv6_values.as_slice(),
            calls: 0,
        };
        let result = read_ipv6_values(&mut reader, 2048).unwrap();
        assert_eq!(reader.calls, 2);
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
