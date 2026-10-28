//! Generated numbers must support analytical expressions without a stored table.
use rusthouse_query::{execute,parse_select};
fn run(sql:&str)->Result<String,String>{
    let mut out=Vec::new();
    execute(&parse_select(sql).map_err(|e|e.to_string())?,None,&mut out)
        .map_err(|e|e.to_string())?;
    Ok(String::from_utf8(out).unwrap())
}
#[test]
fn generated_boolean_counts_weighted_revenue_and_distinct_expressions() {
    assert_eq!(
        run("SELECT countIf(number > 5), sum(number * 2), avg(number + 1) FROM numbers(10)").unwrap(),
        "countIf(number > 5)\tsum(number * 2)\tavg(number + 1)\n4\t90\t5.5\n"
    );
    assert_eq!(
        run("SELECT count(DISTINCT number > 5), count(*) FROM numbers(10)").unwrap(),
        "count(DISTINCT number > 5)\tcount(*)\n2\t10\n"
    );
    assert_eq!(
        run("SELECT sum(number / 2), count(*) FROM numbers(4)").unwrap(),
        "sum((number) / (2))\tcount(*)\n3\t4\n"
    );
    assert_eq!(
        run("SELECT countIf(number > 5), sum(number * 2) FROM numbers(0)").unwrap(),
        "countIf(number > 5)\tsum(number * 2)\n0\tNULL\n"
    );
    assert!(run("SELECT sum(number / (number - 1)), count(*) FROM numbers(3)").is_err());
    assert!(run("SELECT sum(other + 1), count(*) FROM numbers(3)").is_err());
}
