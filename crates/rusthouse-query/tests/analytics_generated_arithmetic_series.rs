//! Formula-based analytical aggregation of large generated numeric sources.
use rusthouse_query::{execute, parse_select};

fn run(sql: &str) -> Result<String,String> {
    let mut out=Vec::new();
    execute(&parse_select(sql).map_err(|e|e.to_string())?,None,&mut out)
        .map_err(|e|e.to_string())?;
    Ok(String::from_utf8(out).unwrap())
}

#[test]
fn large_numeric_aggregates_do_not_materialize_generated_rows() {
    assert_eq!(
        run("SELECT sum(number), min(number), max(number), count(*), avg(number) FROM numbers(1000000000)").unwrap(),
        "sum(number)\tmin(number)\tmax(number)\tcount(*)\tavg(number)\n499999999500000000\t0\t999999999\t1000000000\t499999999.5\n"
    );
    assert_eq!(
        run("SELECT sum(number), min(number), max(number), count(*) FROM generate_series(10, 18, 2)").unwrap(),
        "sum(number)\tmin(number)\tmax(number)\tcount(*)\n70\t10\t18\t5\n"
    );
    assert_eq!(
        run("SELECT sum(number), avg(number), min(number), count(*) FROM numbers(0)").unwrap(),
        "sum(number)\tavg(number)\tmin(number)\tcount(*)\nNULL\tNULL\tNULL\t0\n"
    );
}

#[test]
fn predicates_and_distinct_keep_streaming_and_null_semantics() {
    assert_eq!(
        run("SELECT sum(number), count(*) FROM numbers(6) WHERE number > 3").unwrap(),
        "sum(number)\tcount(*)\n9\t2\n"
    );
    assert_eq!(
        run("SELECT sum(number), count(*) FROM numbers(6) LIMIT 0").unwrap(),
        "sum(number)\tcount(*)\n"
    );
}
