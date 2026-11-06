use super::*;

#[test]
fn accepts_explicit_lz4_preferences_in_normal_header_lists() {
    for value in [
        "x-rusthouse-lz4",
        "X-RUSTHOUSE-LZ4",
        "gzip, X-RUSTHOUSE-LZ4",
        "gzip;q=1, x-rusthouse-lz4; q=0.5",
        "x-rusthouse-lz4;q=1.000",
        "x-rusthouse-lz4;q=0.001",
        "x-rusthouse-lz4;q=1.",
        "x-rusthouse-lz4;Q=0.75, gzip",
    ] {
        assert!(accepts_rusthouse_lz4(value), "{value}");
    }
}

#[test]
fn disabled_invalid_or_implicit_lz4_is_not_negotiated() {
    for value in [
        "",
        "gzip",
        "*",
        "x-rusthouse-lz4;q=0",
        "x-rusthouse-lz4;q=0.000",
        "x-rusthouse-lz4;q=1.001",
        "x-rusthouse-lz4;q=0.0001",
        "x-rusthouse-lz4;q=2",
        "x-rusthouse-lz4;quality=1",
        "x-rusthouse-lz4;q=1;version=1",
        "x-rusthouse-lz4;q=1, x-rusthouse-lz4;q=0",
    ] {
        assert!(!accepts_rusthouse_lz4(value), "{value}");
    }
}
