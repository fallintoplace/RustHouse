use rusthouse_query::{Ddl, parse_ddl};

#[test]
fn float_precision_maps_to_32_or_64_bit_storage() {
    let Some(Ddl::CreateTable { columns, .. }) =
        parse_ddl("CREATE TABLE metrics (a FLOAT(1), b FLOAT(24), c FLOAT(25), d FLOAT(53), e FLOAT)").unwrap()
    else {
        panic!("expected CREATE TABLE");
    };
    let types: Vec<_> = columns.iter().map(|col| col.data_type.as_str()).collect();
    assert_eq!(types, ["Float32", "Float32", "Float64", "Float64", "Float32"]);

    for declaration in ["FLOAT(0)", "FLOAT(54)", "FLOAT(-1)", "FLOAT(1.5)", "FLOAT()", "FLOAT(24"] {
        let sql = format!("CREATE TABLE metrics (score {declaration})");
        assert!(parse_ddl(&sql).is_err(), "{declaration}");
    }
}
