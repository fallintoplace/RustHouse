use std::process::Command;

#[test]
fn decode_xml_component_decodes_expected_values() {
    for (query, expected) in [
        ("SELECT decodeXMLComponent('&lt; &#x3A3; &gt;') AS value", "value\n< Σ >\n"),
        ("SELECT decodeXMLComponent('&amp;&quot;&apos;') AS value", "value\n&\"'\n"),
        ("SELECT decodeXMLComponent('&amp;lt;') AS value", "value\n&lt;\n"),
        ("SELECT decodeXMLComponent('&bogus;') AS value", "value\n&bogus;\n"),
        ("SELECT decodeXMLComponent('text') AS value", "value\ntext\n"),
        ("SELECT decodeXMLComponent(NULL) AS value", "value\n\\N\n"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(output.status.success(), "{query}: {}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(output.stdout, expected.as_bytes(), "{query}");
    }
}

#[test]
fn decode_xml_component_rejects_bad_arity() {
    for query in [
        "SELECT decodeXMLComponent()",
        "SELECT decodeXMLComponent('a','b')",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_rusthouse"))
            .args(["sql", query])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{query}");
    }
}
