//! Grouped analytics over virtual ClickHouse numbers sources.
use rusthouse_query::{execute,parse_select};
fn run(sql:&str)->Result<String,String>{
    let mut out=Vec::new();
    execute(&parse_select(sql).map_err(|e|e.to_string())?,None,&mut out)
        .map_err(|e|e.to_string())?;
    Ok(String::from_utf8(out).unwrap())
}
#[test]
fn groups_generated_series_with_multiple_numeric_metrics() {
    assert_eq!(
        run("SELECT number, sum(number), count(number), max(number) FROM numbers(5) GROUP BY number ORDER BY number DESC LIMIT 2").unwrap(),
        "number\tsum(number)\tcount(number)\tmax(number)\n4\t4\t1\t4\n3\t3\t1\t3\n"
    );
    assert_eq!(
        run("SELECT number, sum(number) AS revenue, count(number) AS events FROM numbers(5) GROUP BY number ORDER BY revenue DESC LIMIT 1 OFFSET 1").unwrap(),
        "number\trevenue\tevents\n3\t3\t1\n"
    );
    assert_eq!(
        run("SELECT number, sum(number), count(number) FROM numbers(6) WHERE number > 3 GROUP BY number ORDER BY number").unwrap(),
        "number\tsum(number)\tcount(number)\n4\t4\t1\n5\t5\t1\n"
    );
    assert_eq!(
        run("SELECT number, sum(number), count(number) FROM numbers(0) GROUP BY number").unwrap(),
        "number\tsum(number)\tcount(number)\n"
    );
    assert!(run("SELECT number, sum(number), count(number) FROM numbers(200001) GROUP BY number").is_err());
}
