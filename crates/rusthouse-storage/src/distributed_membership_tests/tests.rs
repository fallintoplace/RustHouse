use super::*;

#[test]
fn parsed_membership_owns_fields_after_input_is_dropped() {
    let members = {
        let manifest = "n1 z1 1\nn2 z2 3\n".to_owned();
        parse_membership(&manifest).unwrap()
    };
    assert_eq!(format_membership(&members).unwrap(), "n1 z1 1\nn2 z2 3\n");
    assert!(parse_membership("n1 z1 1\nn1 z2 2\n").is_err());
}

#[test]
fn round_trips_membership() {
    let parsed = parse_membership("node_a eu1 3\nnode_b eu2 1\n").unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(
        format_membership(&parsed).unwrap(),
        "node_a eu1 3\nnode_b eu2 1\n"
    );
}

#[test]
fn rejects_duplicates_and_bad_records() {
    for input in [
        "",
        "a eu1 0\n",
        "a eu1 1\na eu2 2\n",
        "../etc eu 1\n",
        "a eu 1 extra\n",
    ] {
        assert!(parse_membership(input).is_err(), "{input}");
    }
}
#[test]
fn validation_matches_serialized_manifest_limits() {
    let members = (0..64)
        .map(|index| Member {
            node: format!("{}{:04}", "n".repeat(60), index),
            zone: "z".repeat(64),
            weight: 1000,
        })
        .collect::<Vec<_>>();
    assert!(validate_membership(&members).is_err());
    assert!(format_membership(&members).is_err());
    let small = parse_membership("a zone1 42\nb zone2 2\n").unwrap();
    assert!(validate_membership(&small).is_ok());
    assert_eq!(
        format_membership(&small).unwrap(),
        "a zone1 42\nb zone2 2\n"
    );
}

#[test]
fn formatting_rejects_oversized_and_duplicate_records() {
    let huge = Member {
        node: "x".repeat(1_000_000),
        zone: "z".into(),
        weight: 1,
    };
    assert!(format_membership(&[huge]).is_err());
    let member = Member {
        node: "n".into(),
        zone: "z".into(),
        weight: 1,
    };
    assert!(format_membership(&[member.clone(), member]).is_err());
    assert!(format_membership(&[]).is_err());
}
