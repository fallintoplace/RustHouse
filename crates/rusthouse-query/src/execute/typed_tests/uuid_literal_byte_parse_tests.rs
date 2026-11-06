use super::uuid_literal_bytes;

#[test]
fn decodes_uppercase_and_lowercase_without_changing_byte_order() {
    let lower = uuid_literal_bytes("61f0c404-5cb3-11e7-907b-a6006ad3dba0").unwrap();
    let upper = uuid_literal_bytes("61F0C404-5CB3-11E7-907B-A6006AD3DBA0").unwrap();
    assert_eq!(lower, upper);
    assert_eq!(lower[..4], [0x61, 0xf0, 0xc4, 0x04]);
    assert_eq!(lower[15], 0xa0);
}

#[test]
fn rejects_invalid_uuid_literals_before_decoding() {
    for invalid in [
        "",
        "1",
        "61f0c4045cb311e7907ba6006ad3dba0",
        "61f0c404-5cb3-11e7-907b-a6006ad3dbzz",
        "61f0c404-5cb3-11e7-907b-a6006ad3dbax",
    ] {
        assert!(uuid_literal_bytes(invalid).is_err(), "{invalid}");
    }
}
