use rusthouse_query::{execute, parse_select};

fn query(sql: &str) -> Result<String, String> {
    let select = parse_select(sql).map_err(|error| error.to_string())?;
    let mut output = Vec::new();
    execute(&select, None, &mut output).map_err(|error| error.to_string())?;
    Ok(String::from_utf8(output).unwrap())
}

#[test]
fn rms_computes_population_root_mean_square() {
    let result = query("SELECT rms(number) FROM numbers(3)").unwrap();
    let value: f64 = result.lines().nth(1).unwrap().parse().unwrap();
    assert!((value - (5.0_f64 / 3.0).sqrt()).abs() < 1e-12);
    let result = query("SELECT rms(number) FROM numbers(4) WHERE number >= 2").unwrap();
    let value: f64 = result.lines().nth(1).unwrap().parse().unwrap();
    assert!((value - (13.0_f64 / 2.0).sqrt()).abs() < 1e-12);
}

#[test]
fn rms_handles_empty_and_grouped_inputs() {
    assert_eq!(
        query("SELECT rms(number) FROM numbers(0)").unwrap(),
        "rms(number)\nNULL\n"
    );
    assert_eq!(
        query("SELECT number, rms(number) FROM numbers(3) GROUP BY number").unwrap(),
        "number\trms(number)\n0\t0\n1\t1\n2\t2\n"
    );
}
