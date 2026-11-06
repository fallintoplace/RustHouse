use super::*;

#[test]
fn recycles_decoded_allocation_across_read_and_bufread_boundaries() {
    use ch_cityhash102::cityhash128;

    let path = std::env::temp_dir().join(format!(
        "rusthouse-mark-scratch-{}-{}.bin",
        std::process::id(),
        line!()
    ));
    let mut encoded = Vec::new();
    for payload in [b"aaaa", b"bbbb", b"cccc"] {
        let mut block = vec![0x02];
        block.extend_from_slice(&(9u32 + payload.len() as u32).to_le_bytes());
        block.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        block.extend_from_slice(payload);
        encoded.extend_from_slice(&u128::from(cityhash128(&block)).to_le_bytes());
        encoded.extend_from_slice(&block);
    }
    std::fs::write(&path, encoded).unwrap();

    let mut stream = Stream {
        marks: vec![WideMark {
            compressed_offset: 0,
            decompressed_offset: 0,
            rows: Some(1),
        }],
        reader: CompressedBlockReader::open(&path).unwrap(),
        cached: None,
    };
    {
        let mut mark = MarkStream::open(&mut stream, 0).unwrap();
        assert_eq!(mark.fill_buf().unwrap(), b"aaaa");
        let first = mark.block.as_ref().unwrap().data.as_ptr();
        mark.consume(4);
        assert_eq!(mark.fill_buf().unwrap(), b"bbbb");
        assert_eq!(mark.block.as_ref().unwrap().data.as_ptr(), first);
        let mut second = [0u8; 4];
        mark.read_exact(&mut second).unwrap();
        assert_eq!(&second, b"bbbb");
        let mut third = [0u8; 4];
        mark.read_exact(&mut third).unwrap();
        assert_eq!(&third, b"cccc");
        assert_eq!(mark.block.as_ref().unwrap().data.as_ptr(), first);
    }
    drop(stream);
    std::fs::remove_file(path).unwrap();
}
