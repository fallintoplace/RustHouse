use super::*;
use crate::{Operator, parse_select};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

fn run(query: &str, part: Option<&Path>) -> Result<String, ExecutionError> {
    let select = parse_select(query).unwrap();
    let mut output = Vec::new();
    execute(&select, part, &mut output)?;
    Ok(String::from_utf8(output).unwrap())
}

#[test]
fn runs_scalar_queries_without_a_part() {
    assert_eq!(run("SELECT 1", None).unwrap(), "1\n1\n");
    assert_eq!(run("SELECT -5 LIMIT 0", None).unwrap(), "-5\n");
}

#[test]
fn single_result_aliases_change_headers_and_support_order_by() {
    assert_eq!(run("SELECT 1 AS result", None).unwrap(), "result\n1\n");
    assert_eq!(
        run("SELECT count(*) AS total FROM numbers(4)", None).unwrap(),
        "total\n4\n"
    );
    assert_eq!(
        run("SELECT sum(number) AS total FROM numbers(4)", None).unwrap(),
        "total\n6\n"
    );
    assert_eq!(
        run(
            "SELECT number AS n FROM numbers(4) ORDER BY n DESC LIMIT 2",
            None
        )
        .unwrap(),
        "n\n3\n2\n"
    );
    assert_eq!(
        run(
            "SELECT DISTINCT number AS n FROM numbers(2) ORDER BY n",
            None
        )
        .unwrap(),
        "n\n0\n1\n"
    );
}

#[test]
fn groups_numeric_aggregates_on_generated_sources() {
    assert_eq!(
        run(
            "SELECT number, sum(number) FROM numbers(3) GROUP BY number ORDER BY number DESC",
            None,
        )
        .unwrap(),
        "number\tsum(number)\n2\t2\n1\t1\n0\t0\n",
    );
    assert_eq!(
        run(
            "SELECT number, median(number) FROM numbers(5) WHERE number >= 2 GROUP BY number ORDER BY number LIMIT 1 OFFSET 1",
            None,
        ).unwrap(),
        "number\tmedian(number)\n3\t3\n",
    );
    assert_eq!(
        run(
            "SELECT number, avg(number) FROM numbers(0) GROUP BY number",
            None,
        )
        .unwrap(),
        "number\tavg(number)\n",
    );
    assert!(
        run(
            "SELECT number, sum(other) FROM numbers(2) GROUP BY number",
            None
        )
        .is_err()
    );
}

#[test]
fn wildcard_rename_preserves_rows_and_resolves_order_alias() {
    assert_eq!(
        run(
            "SELECT * RENAME number AS value FROM numbers(5) WHERE number >= 2 ORDER BY value DESC LIMIT 2",
            None
        )
        .unwrap(),
        "value\n4\n3\n"
    );
    assert_eq!(
        run(
            "SELECT DISTINCT * RENAME number AS value FROM numbers(2)",
            None
        )
        .unwrap(),
        "value\n0\n1\n"
    );
    assert_eq!(
        run("SELECT * RENAME number AS number FROM numbers(0)", None).unwrap(),
        "number\n"
    );
    assert!(matches!(
        run(
            "SELECT * RENAME missing AS label FROM numbers(0) LIMIT 0",
            None
        ),
        Err(ExecutionError::Invalid(_))
    ));
}

#[test]
fn clickhouse_count_empty_parentheses_matches_count_star() {
    assert_eq!(
        run("SELECT count() FROM numbers(17) WHERE number >= 12", None).unwrap(),
        run("SELECT count(*) FROM numbers(17) WHERE number >= 12", None).unwrap()
    );
    assert_eq!(
        run(
            "SELECT number, count() FROM numbers(3) GROUP BY number",
            None
        )
        .unwrap(),
        run(
            "SELECT number, count(*) FROM numbers(3) GROUP BY number",
            None
        )
        .unwrap()
    );
}

#[test]
fn runs_generated_numbers_with_projection_filter_count_and_limit() {
    assert_eq!(
        run("SELECT * FROM numbers(4)", None).unwrap(),
        "number\n0\n1\n2\n3\n"
    );
    assert_eq!(
        run(
            "SELECT number, number FROM numbers(5) WHERE number >= 3 LIMIT 1",
            None,
        )
        .unwrap(),
        "number\tnumber\n3\t3\n"
    );
    assert_eq!(
        run("SELECT count(*) FROM numbers(5)", None).unwrap(),
        "count\n5\n"
    );
    assert_eq!(
        run("SELECT count(*) FROM numbers(5) WHERE number < 3", None).unwrap(),
        "count\n3\n"
    );
    assert_eq!(
        run("SELECT number FROM numbers(0)", None).unwrap(),
        "number\n"
    );
    assert_eq!(
        run("SELECT number FROM numbers(4) LIMIT 0", None).unwrap(),
        "number\n"
    );
    assert!(run("SELECT missing FROM numbers(0)", None).is_err());
    assert!(run("SELECT number FROM numbers(0) WHERE missing = 1", None).is_err());
    assert!(run("SELECT number FROM numbers(0) WHERE number = -1", None).is_err());
    assert!(run("SELECT number FROM numbers(2)", Some(Path::new("."))).is_err());
}

#[test]
fn filters_ordered_numbers_in_input_order_without_building_a_topk_heap() {
    assert_eq!(
        run(
            "SELECT number FROM numbers(100) WHERE number % 7 = 0 ORDER BY number DESC LIMIT 3",
            None
        )
        .unwrap(),
        "number\n98\n91\n84\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(100) WHERE number % 7 = 0 ORDER BY number DESC LIMIT 2 OFFSET 1",
            None
        )
        .unwrap(),
        "number\n91\n84\n"
    );
    assert_eq!(
        run(
            "SELECT number AS n FROM numbers(40) WHERE number % 5 = 0 ORDER BY n LIMIT 2 OFFSET 2",
            None
        )
        .unwrap(),
        "n\n10\n15\n"
    );
    assert_eq!(
        run(
            "SELECT number, number FROM numbers(12) WHERE number % 4 = 0 ORDER BY number DESC LIMIT 2",
            None
        )
        .unwrap(),
        "number\tnumber\n8\t8\n4\t4\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(3, 20, 4) WHERE number % 3 = 1 ORDER BY number DESC LIMIT 1 OFFSET 1",
            None
        )
        .unwrap(),
        "number\n7\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(30) WHERE number = 3 OR number = 23 ORDER BY number DESC LIMIT 1",
            None
        )
        .unwrap(),
        "number\n23\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(5) WHERE number % 7 = 6 ORDER BY number DESC LIMIT 2",
            None
        )
        .unwrap(),
        "number\n"
    );
}

#[test]
fn queries_numbers_with_start_and_count() {
    assert_eq!(
        run(
            "SELECT number FROM numbers(7, 4) WHERE number >= 8 LIMIT 2",
            None
        )
        .unwrap(),
        "number\n8\n9\n"
    );
    assert_eq!(
        run("SELECT count(*) FROM numbers(7, 4)", None).unwrap(),
        "count\n4\n"
    );
    assert_eq!(
        run("SELECT sum(number) FROM numbers(7, 4)", None).unwrap(),
        "sum(number)\n34\n"
    );
    assert_eq!(
        run("SELECT number FROM numbers(7, 0)", None).unwrap(),
        "number\n"
    );
}

#[test]
fn stepped_numbers_respect_clickhouse_range_semantics() {
    assert_eq!(
        run("SELECT number FROM numbers(3, 10, 4)", None).unwrap(),
        "number\n3\n7\n11\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(3, 10, 4) LIMIT 1 OFFSET 2",
            None
        )
        .unwrap(),
        "number\n11\n"
    );
    assert_eq!(
        run("SELECT count(*) FROM numbers(3, 10, 4)", None).unwrap(),
        "count\n3\n"
    );
    assert_eq!(
        run("SELECT sum(number) FROM numbers(3, 10, 4)", None).unwrap(),
        "sum(number)\n21\n"
    );
    assert_eq!(
        run("SELECT number FROM numbers(5, 0, 2)", None).unwrap(),
        "number\n"
    );
    assert_eq!(
        run("SELECT number FROM numbers(5, 3, 100)", None).unwrap(),
        "number\n5\n"
    );
}

#[test]
fn filters_with_modulo_for_generated_numbers() {
    assert_eq!(
        run(
            "SELECT number FROM numbers(12) WHERE number % 3 = 1 ORDER BY number DESC LIMIT 2",
            None,
        )
        .unwrap(),
        "number\n10\n7\n"
    );
    assert_eq!(
        run(
            "SELECT count(*) FROM numbers(10) WHERE number % 2 = 0",
            None
        )
        .unwrap(),
        "count\n5\n"
    );
}

#[test]
fn excludes_modulo_remainders_for_generated_numbers() {
    assert_eq!(
        run("SELECT number FROM numbers(10) WHERE number % 3 != 1", None).unwrap(),
        "number\n0\n2\n3\n5\n6\n8\n9\n"
    );
    assert_eq!(
        run(
            "SELECT count(*) FROM numbers(10) WHERE number % 3 != 1 AND number >= 4",
            None
        )
        .unwrap(),
        "count\n4\n"
    );
    assert!(!number_matches(
        Number::Float(1.0),
        Number::Float(1.0),
        Operator::ModuloNe(3)
    ));
}

#[test]
fn evaluates_not_in_with_filters_and_aggregates() {
    assert_eq!(
        run(
            "SELECT number FROM numbers(10) WHERE number NOT IN (1, 3, 5) LIMIT 4",
            None,
        )
        .unwrap(),
        "number\n0\n2\n4\n6\n"
    );
    assert_eq!(
        run(
            "SELECT count(*) FROM numbers(10) WHERE number NOT IN (1, 3, 5)",
            None,
        )
        .unwrap(),
        "count\n7\n"
    );
}

#[test]
fn evaluates_or_across_generated_rows() {
    assert_eq!(
        run(
            "SELECT number FROM numbers(10) WHERE number = 1 OR number >= 4 AND number < 6",
            None,
        )
        .unwrap(),
        "number\n1\n4\n5\n"
    );
    assert_eq!(
        run(
            "SELECT count(*) FROM numbers(10) WHERE number = 0 OR number = 9",
            None,
        )
        .unwrap(),
        "count\n2\n"
    );
}

#[test]
fn counts_exact_distinct_numeric_values_after_filters() {
    assert_eq!(
        run("SELECT count(DISTINCT number) FROM numbers(10)", None).unwrap(),
        "countDistinct(number)\n10\n"
    );
    assert_eq!(
        run(
            "SELECT count(DISTINCT number) FROM numbers(10) WHERE number < 3 OR number >= 8",
            None
        )
        .unwrap(),
        "countDistinct(number)\n5\n"
    );
    assert_eq!(
        run("SELECT count(DISTINCT number) FROM numbers(0)", None).unwrap(),
        "countDistinct(number)\n0\n"
    );
    assert!(run("SELECT count(DISTINCT missing) FROM numbers(3)", None).is_err());
}

#[test]
fn generated_count_distinct_skips_unfiltered_scans_and_respects_steps() {
    assert_eq!(
        run("SELECT count(DISTINCT number) FROM numbers(1000000)", None).unwrap(),
        "countDistinct(number)\n1000000\n"
    );
    assert_eq!(
        run("SELECT count(DISTINCT number) FROM numbers(3, 10, 4)", None).unwrap(),
        "countDistinct(number)\n3\n"
    );
    assert_eq!(
        run(
            "SELECT count(DISTINCT number) FROM numbers(3, 10, 4) WHERE number >= 7",
            None
        )
        .unwrap(),
        "countDistinct(number)\n2\n"
    );
    assert_eq!(
        run(
            "SELECT count(DISTINCT number) FROM numbers(3, 10, 4) WHERE number = 11",
            None
        )
        .unwrap(),
        "countDistinct(number)\n1\n"
    );
    assert_eq!(
        run("SELECT count(DISTINCT number) FROM numbers(0)", None).unwrap(),
        "countDistinct(number)\n0\n"
    );
    assert_eq!(
        run(
            "SELECT count(DISTINCT number) FROM numbers(4) LIMIT 0",
            None
        )
        .unwrap(),
        "countDistinct(number)\n"
    );
}

#[test]
fn aggregates_generated_numbers_and_empty_inputs() {
    assert_eq!(
        run("SELECT sum(number) FROM numbers(10)", None).unwrap(),
        "sum(number)\n45\n"
    );
    assert_eq!(
        run("SELECT AVG(number) FROM numbers(10)", None).unwrap(),
        "avg(number)\n4.5\n"
    );
    assert_eq!(
        run(
            "SELECT sum(number) FROM numbers(10) WHERE number >= 7",
            None,
        )
        .unwrap(),
        "sum(number)\n24\n"
    );
    assert_eq!(
        run("SELECT min(number) FROM numbers(10)", None).unwrap(),
        "min(number)\n0\n"
    );
    assert_eq!(
        run("SELECT max(number) FROM numbers(10)", None).unwrap(),
        "max(number)\n9\n"
    );
    assert_eq!(
        run("SELECT avg(number) FROM numbers(0)", None).unwrap(),
        "avg(number)\nNULL\n"
    );
    assert_eq!(
        run("SELECT sum(number) FROM numbers(5) WHERE number > 10", None).unwrap(),
        "sum(number)\nNULL\n"
    );
    assert_eq!(
        run("SELECT sum(number) FROM numbers(5) LIMIT 0", None).unwrap(),
        "sum(number)\n"
    );
    assert!(run("SELECT sum(missing) FROM numbers(0)", None).is_err());
}

#[test]
fn median_handles_odd_even_filtered_and_empty_inputs() {
    assert_eq!(
        run("SELECT median(number) FROM numbers(5)", None).unwrap(),
        "median(number)\n2\n"
    );
    assert_eq!(
        run("SELECT median(number) FROM numbers(6)", None).unwrap(),
        "median(number)\n2.5\n"
    );
    assert_eq!(
        run(
            "SELECT median(number) FROM numbers(20) WHERE number % 3 = 1",
            None
        )
        .unwrap(),
        "median(number)\n10\n"
    );
    assert_eq!(
        run("SELECT median(number) FROM numbers(1, 6, 2)", None).unwrap(),
        "median(number)\n3\n"
    );
    assert_eq!(
        run("SELECT median(number) FROM numbers(0)", None).unwrap(),
        "median(number)\nNULL\n"
    );
    assert_eq!(
        run(
            "SELECT median(number) FROM numbers(3) WHERE number > 9",
            None
        )
        .unwrap(),
        "median(number)\nNULL\n"
    );
}

#[test]
fn median_is_memory_bounded_and_preserves_odd_large_integers() {
    let mut state = AggregateState::new(AggregateFunction::Median);
    state.update(Number::Unsigned(u128::MAX)).unwrap();
    assert_eq!(state.render(), u128::MAX.to_string());
    let error = run("SELECT median(number) FROM numbers(100001)", None).unwrap_err();
    assert!(error.to_string().contains("at most 100000 matching rows"));
    assert_eq!(
        run("SELECT median(number) FROM numbers(100000)", None).unwrap(),
        "median(number)\n49999.5\n"
    );
}

#[test]
fn calculates_population_variance_and_standard_deviation() {
    assert_eq!(
        run("SELECT varPop(number) FROM numbers(4)", None).unwrap(),
        "varPop(number)\n1.25\n"
    );
    assert_eq!(
        run("SELECT stddevPop(number) FROM numbers(1)", None).unwrap(),
        "stddevPop(number)\n0\n"
    );
    assert_eq!(
        run("SELECT varPop(number) FROM numbers(0)", None).unwrap(),
        "varPop(number)\nNULL\n"
    );
    assert_eq!(
        run(
            "SELECT stddevPop(number) FROM numbers(10) WHERE number > 99",
            None
        )
        .unwrap(),
        "stddevPop(number)\nNULL\n"
    );
    assert_eq!(
        run("SELECT varPop(number) FROM numbers(4) LIMIT 0", None).unwrap(),
        "varPop(number)\n"
    );

    let output = run(
        "SELECT stddevPop(number) FROM numbers(10) WHERE number >= 7",
        None,
    )
    .unwrap();
    let actual = output.lines().nth(1).unwrap().parse::<f64>().unwrap();
    let expected = (2.0_f64 / 3.0).sqrt();
    assert!((actual - expected).abs() < 1e-12);
}

#[test]
fn population_statistics_handle_signed_values_and_stable_large_offsets() {
    let mut variance = AggregateState::new(AggregateFunction::VarPop);
    for n in [-2, 0, 2] {
        variance.update(Number::Signed(n)).unwrap();
    }
    assert!((variance.render().parse::<f64>().unwrap() - 8.0 / 3.0).abs() < 1e-12);

    let mut deviation = AggregateState::new(AggregateFunction::StddevPop);
    for n in [
        1_000_000_000_000.0,
        1_000_000_000_001.0,
        1_000_000_000_002.0,
    ] {
        deviation.update(Number::Float(n)).unwrap();
    }
    assert!((deviation.render().parse::<f64>().unwrap() - (2.0_f64 / 3.0).sqrt()).abs() < 1e-12);
}

#[test]
fn exact_unfiltered_generated_aggregates_match_scanned_values() {
    for (start, length, step) in [
        (0, 0, 1),
        (0, 17, 1),
        (3, 10, 4),
        (1000, 1, 10),
        (u64::MAX - 20, 20, 6),
    ] {
        let rows = length / step + u64::from(length % step != 0);
        for function in [
            AggregateFunction::Sum,
            AggregateFunction::Min,
            AggregateFunction::Max,
        ] {
            let mut state = AggregateState::new(function);
            for index in 0..rows {
                state
                    .update(Number::Unsigned(u128::from(start + index * step)))
                    .unwrap();
            }
            let sql = format!(
                "SELECT {}(number) FROM numbers({start}, {length}, {step})",
                function.name()
            );
            assert_eq!(
                run(&sql, None).unwrap(),
                format!("{}(number)\n{}\n", function.name(), state.render()),
                "{sql}"
            );
        }
    }
    assert_eq!(
        run("SELECT sum(number) FROM numbers(1000000)", None).unwrap(),
        "sum(number)\n499999500000\n"
    );
    assert_eq!(
        run(
            "SELECT sum(number) FROM numbers(10) WHERE number >= 8",
            None
        )
        .unwrap(),
        "sum(number)\n17\n"
    );
}

#[test]
fn aggregate_state_preserves_precision_and_rejects_overflow() {
    let mut sum = AggregateState::new(AggregateFunction::Sum);
    sum.update(Number::Unsigned(u128::MAX)).unwrap();
    assert!(sum.update(Number::Unsigned(1)).is_err());

    let mut min = AggregateState::new(AggregateFunction::Min);
    min.update(Number::Signed(i128::MAX)).unwrap();
    min.update(Number::Signed(i128::MIN)).unwrap();
    assert_eq!(min.render(), i128::MIN.to_string());
}

#[test]
fn sorts_numbers_with_top_k_and_filtered_rows() {
    assert_eq!(
        run(
            "SELECT number FROM numbers(10) ORDER BY number DESC LIMIT 3",
            None,
        )
        .unwrap(),
        "number\n9\n8\n7\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(8) WHERE number >= 4 ORDER BY number ASC LIMIT 2",
            None,
        )
        .unwrap(),
        "number\n4\n5\n"
    );
    assert_eq!(
        run("SELECT number FROM numbers(3) ORDER BY number DESC", None).unwrap(),
        "number\n2\n1\n0\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(3) ORDER BY number LIMIT 0",
            None,
        )
        .unwrap(),
        "number\n"
    );
    assert!(run("SELECT number FROM numbers(0) ORDER BY missing", None).is_err());
}

#[test]
fn orders_unfiltered_generated_ranges_without_scanning() {
    assert_eq!(
        run(
            "SELECT number FROM numbers(1000000) ORDER BY number DESC LIMIT 2 OFFSET 1000",
            None
        )
        .unwrap(),
        "number\n998999\n998998\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(1000000) ORDER BY number LIMIT 1 OFFSET 99999",
            None
        )
        .unwrap(),
        "number\n99999\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(20, 13, 4) ORDER BY number DESC LIMIT 2 OFFSET 1",
            None
        )
        .unwrap(),
        "number\n28\n24\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(3) ORDER BY number DESC LIMIT 2 OFFSET 5",
            None
        )
        .unwrap(),
        "number\n"
    );
    assert!(run("SELECT number FROM numbers(200001) ORDER BY number", None).is_err());
    assert!(
        run(
            "SELECT number FROM numbers(200001) ORDER BY number LIMIT 200001",
            None
        )
        .is_err()
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(100001) ORDER BY number LIMIT 0",
            None
        )
        .unwrap(),
        "number\n"
    );
}

#[test]
fn not_between_filters_generated_numbers_and_combines_with_or() {
    assert_eq!(
        run(
            "SELECT number FROM numbers(10) WHERE number NOT BETWEEN 3 AND 6",
            None
        )
        .unwrap(),
        "number\n0\n1\n2\n7\n8\n9\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(10) WHERE number NOT BETWEEN 3 AND 6 AND number < 9 OR number = 9",
            None
        )
        .unwrap(),
        "number\n0\n1\n2\n7\n8\n9\n"
    );
    assert_eq!(
        run(
            "SELECT count(*) FROM numbers(10) WHERE number NOT BETWEEN 0 AND 9",
            None
        )
        .unwrap(),
        "count\n0\n"
    );
}

#[test]
fn between_supports_inclusive_ranges_and_chained_filters() {
    assert_eq!(
        run(
            "SELECT number FROM numbers(10) WHERE number BETWEEN 3 AND 7",
            None
        )
        .unwrap(),
        "number\n3\n4\n5\n6\n7\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(10) WHERE number BETWEEN 3 AND 8 AND number != 5 ORDER BY number DESC LIMIT 3 OFFSET 1",
            None,
        )
        .unwrap(),
        "number\n7\n6\n4\n"
    );
    assert_eq!(
        run(
            "SELECT count(*) FROM numbers(10) WHERE number BETWEEN 3 AND 7",
            None
        )
        .unwrap(),
        "count\n5\n"
    );
    assert_eq!(
        run(
            "SELECT sum(number) FROM numbers(10) WHERE number BETWEEN 3 AND 7",
            None
        )
        .unwrap(),
        "sum(number)\n25\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(10) WHERE number BETWEEN 7 AND 3",
            None
        )
        .unwrap(),
        "number\n"
    );
    assert!(
        run(
            "SELECT number FROM numbers(0) WHERE number BETWEEN -1 AND 3",
            None
        )
        .is_err()
    );
}

#[test]
fn multiple_where_conditions_filter_numbers_counts_and_aggregates() {
    assert_eq!(
        run(
            "SELECT number FROM numbers(10) WHERE number >= 3 AND number < 7",
            None,
        )
        .unwrap(),
        "number\n3\n4\n5\n6\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(10) WHERE number > 3 AND number <= 8 ORDER BY number DESC LIMIT 2",
            None,
        )
        .unwrap(),
        "number\n8\n7\n"
    );
    assert_eq!(
        run(
            "SELECT count(*) FROM numbers(10) WHERE number >= 2 AND number < 5",
            None,
        )
        .unwrap(),
        "count\n3\n"
    );
    assert_eq!(
        run(
            "SELECT sum(number) FROM numbers(10) WHERE number >= 2 AND number < 5",
            None,
        )
        .unwrap(),
        "sum(number)\n9\n"
    );
    assert!(
        run(
            "SELECT number FROM numbers(0) WHERE number > 2 AND missing < 5",
            None
        )
        .is_err()
    );
}

#[test]
fn groups_numeric_rows_after_filtering_before_pagination() {
    assert_eq!(
        run(
            "SELECT number, COUNT(*) FROM numbers(5) GROUP BY number",
            None,
        )
        .unwrap(),
        "number\tcount\n0\t1\n1\t1\n2\t1\n3\t1\n4\t1\n"
    );
    assert_eq!(
        run(
            "SELECT number, COUNT(*) FROM numbers(10) WHERE number BETWEEN 2 AND 6 GROUP BY number ORDER BY number DESC LIMIT 2 OFFSET 1",
            None,
        )
        .unwrap(),
        "number\tcount\n5\t1\n4\t1\n"
    );
    assert_eq!(
        run(
            "SELECT number, COUNT(*) FROM numbers(0) GROUP BY number",
            None
        )
        .unwrap(),
        "number\tcount\n"
    );
    assert_eq!(
        run(
            "SELECT number, COUNT(*) FROM numbers(3) GROUP BY number LIMIT 0",
            None
        )
        .unwrap(),
        "number\tcount\n"
    );
    assert!(run("SELECT bad, COUNT(*) FROM numbers(0) GROUP BY bad", None).is_err());
}

#[test]
fn grouped_count_aliases_work_with_ordering_and_having() {
    assert_eq!(
        run(
            "SELECT number, count(*) AS c FROM numbers(4) GROUP BY number HAVING c >= 1 ORDER BY c DESC, number LIMIT 2",
            None,
        )
        .unwrap(),
        "number\tc\n0\t1\n1\t1\n"
    );
}

#[test]
fn grouped_having_filters_after_counting_before_limit() {
    let query = crate::parse_select(
        "SELECT number, count(*) FROM numbers(10) GROUP BY number HAVING count(*) >= 2 ORDER BY number LIMIT 1",
    )
    .unwrap();
    let mut groups = GroupedCounts::default();
    for value in [3, 3, 3, 1, 1, 5] {
        groups.push(Number::Unsigned(value)).unwrap();
    }
    let mut result = Vec::new();
    groups.write(&query, "number", &mut result).unwrap();
    assert_eq!(String::from_utf8(result).unwrap(), "number\tcount\n1\t2\n");
    assert_eq!(
        run(
            "SELECT number, count(*) FROM numbers(3) GROUP BY number HAVING count(*) > 1",
            None
        )
        .unwrap(),
        "number\tcount\n"
    );
}

#[test]
fn grouped_counts_support_ranges_and_or_filters() {
    assert_eq!(
        run(
            "SELECT number, COUNT(*) FROM numbers(8, 4) WHERE number = 8 OR number = 11 GROUP BY number ORDER BY number DESC",
            None
        )
        .unwrap(),
        "number\tcount\n11\t1\n8\t1\n"
    );
}

#[test]
fn grouped_count_order_sorts_by_frequency_then_key() {
    let query = crate::parse_select(
        "SELECT number, count(*) FROM numbers(5) GROUP BY number ORDER BY count() DESC LIMIT 2",
    )
    .unwrap();
    let mut groups = GroupedCounts::default();
    for value in [3, 3, 1, 1, 1, 2] {
        groups.push(Number::Unsigned(value)).unwrap();
    }
    let mut result = Vec::new();
    groups.write(&query, "number", &mut result).unwrap();
    assert_eq!(
        String::from_utf8(result).unwrap(),
        "number\tcount\n1\t3\n3\t2\n"
    );
}

#[test]
fn grouped_counts_merge_equal_values_without_precision_loss() {
    let mut groups = GroupedCounts::default();
    groups.push(Number::Unsigned(u128::MAX)).unwrap();
    groups.push(Number::Unsigned(u128::MAX)).unwrap();
    groups.push(Number::Unsigned(u128::MAX - 1)).unwrap();
    groups.push(Number::Float(0.0)).unwrap();
    groups.push(Number::Float(-0.0)).unwrap();
    assert_eq!(groups.rows.len(), 3);
    assert_eq!(groups.rows[0].1, 2);
    assert_eq!(groups.rows[1].1, 1);
    assert_eq!(groups.rows[2].1, 2);
}

#[test]
fn distinct_keys_preserve_numeric_identity() {
    let mut seen = HashSet::new();
    let mut retained_bytes = 0usize;
    assert!(remember_distinct(&mut seen, &mut retained_bytes, &[Number::Unsigned(u128::MAX)]).unwrap());
    assert!(!remember_distinct(&mut seen, &mut retained_bytes, &[Number::Unsigned(u128::MAX)]).unwrap());
    assert!(remember_distinct(&mut seen, &mut retained_bytes, &[Number::Unsigned(u128::MAX - 1)]).unwrap());
    assert!(remember_distinct(&mut seen, &mut retained_bytes, &[Number::Float(0.0)]).unwrap());
    assert!(!remember_distinct(&mut seen, &mut retained_bytes, &[Number::Float(-0.0)]).unwrap());
    assert!(remember_distinct(&mut seen, &mut retained_bytes, &[Number::Float(f64::NAN)]).unwrap());
    assert!(!remember_distinct(&mut seen, &mut retained_bytes, &[Number::Float(-f64::NAN)]).unwrap());
    assert!(remember_distinct(&mut seen, &mut retained_bytes, &[Number::Signed(-1)]).unwrap());
}

#[test]
fn distinct_limit_rejects_new_rows_but_accepts_existing_rows() {
    let mut seen = HashSet::new();
    let mut retained_bytes = 0usize;
    for index in 0..MAX_DISTINCT_ROWS {
        assert!(remember_distinct(&mut seen, &mut retained_bytes, &[Number::Unsigned(index as u128)]).unwrap());
    }
    assert!(!remember_distinct(&mut seen, &mut retained_bytes, &[Number::Unsigned(0)]).unwrap());
    assert!(matches!(
        remember_distinct(&mut seen, &mut retained_bytes, &[Number::Unsigned(MAX_DISTINCT_ROWS as u128)]),
        Err(ExecutionError::Invalid(_))
    ));
}

#[test]
fn distinct_on_generated_numbers_preserves_unique_rows() {
    assert_eq!(
        run("SELECT DISTINCT number FROM numbers(3)", None).unwrap(),
        "number\n0\n1\n2\n"
    );
    assert_eq!(
        run(
            "SELECT DISTINCT number, number FROM numbers(3) LIMIT 2 OFFSET 1",
            None
        )
        .unwrap(),
        "number\tnumber\n1\t1\n2\t2\n"
    );
    assert_eq!(
        run("SELECT DISTINCT * FROM numbers(0)", None).unwrap(),
        "number\n"
    );
}

#[test]
fn in_lists_filter_numbers_aggregates_and_empty_inputs() {
    assert_eq!(
        run(
            "SELECT number FROM numbers(10) WHERE number IN (2, 4, 7) ORDER BY number DESC LIMIT 2 OFFSET 1",
            None,
        )
        .unwrap(),
        "number\n4\n2\n"
    );
    assert_eq!(
        run(
            "SELECT count(*) FROM numbers(10) WHERE number IN (1, 4, 8) AND number > 2",
            None,
        )
        .unwrap(),
        "count\n2\n"
    );
    assert_eq!(
        run(
            "SELECT sum(number) FROM numbers(10) WHERE number IN (3, 5, 7)",
            None
        )
        .unwrap(),
        "sum(number)\n15\n"
    );
    assert_eq!(
        run("SELECT number FROM numbers(0) WHERE number IN (1)", None).unwrap(),
        "number\n"
    );
    assert!(run("SELECT number FROM numbers(0) WHERE number IN (-1)", None).is_err());
    assert!(run("SELECT number FROM numbers(0) WHERE missing IN (1)", None).is_err());
}

#[test]
fn large_in_and_not_in_lists_use_hash_lookups_without_changing_results() {
    let values = (0..32).map(|number| number.to_string()).collect::<Vec<_>>();
    let in_query = format!(
        "SELECT count(*) FROM numbers(1000000) WHERE number IN ({})",
        values.join(", ")
    );
    assert_eq!(run(&in_query, None).unwrap(), "count\n32\n");
    let not_in_query = format!(
        "SELECT count(*) FROM numbers(1000000) WHERE number NOT IN ({})",
        values.join(", ")
    );
    assert_eq!(run(&not_in_query, None).unwrap(), "count\n999968\n");

    let float_values = (0..16)
        .map(|number| Number::Float(number as f64))
        .collect::<Vec<_>>();
    let positive = compile_in_test(float_values.clone(), false);
    let negative = compile_in_test(float_values, true);
    assert!(matches!(&positive, CompiledTest::InSet { .. }));
    assert!(positive.matches(Number::Float(-0.0)));
    assert!(positive.matches(Number::Float(15.0)));
    assert!(!positive.matches(Number::Float(16.0)));
    assert!(!positive.matches(Number::Float(f64::NAN)));
    assert!(!positive.matches(Number::Unsigned(1)));
    assert!(!negative.matches(Number::Float(-0.0)));
    assert!(negative.matches(Number::Float(16.0)));
    assert!(negative.matches(Number::Float(f64::NAN)));
    assert!(negative.matches(Number::Unsigned(1)));

    let mut large_values = (0..16)
        .map(|number| Number::Unsigned(number as u128))
        .collect::<Vec<_>>();
    large_values.push(Number::Unsigned(u128::MAX));
    let test = compile_in_test(large_values, false);
    assert!(test.matches(Number::Unsigned(u128::MAX)));
    assert!(!test.matches(Number::Unsigned(u128::MAX - 1)));
}

#[test]
fn pagination_applies_after_filtering_and_sorting() {
    assert_eq!(
        run("SELECT number FROM numbers(10) LIMIT 3 OFFSET 4", None).unwrap(),
        "number\n4\n5\n6\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(10) ORDER BY number DESC LIMIT 3 OFFSET 2",
            None,
        )
        .unwrap(),
        "number\n7\n6\n5\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(10) WHERE number >= 5 LIMIT 2 OFFSET 1",
            None,
        )
        .unwrap(),
        "number\n6\n7\n"
    );
    assert_eq!(
        run("SELECT number FROM numbers(2) LIMIT 2 OFFSET 100", None).unwrap(),
        "number\n"
    );
    assert_eq!(
        run("SELECT count(*) FROM numbers(10) LIMIT 1 OFFSET 1", None).unwrap(),
        "count\n"
    );
    assert_eq!(
        run("SELECT sum(number) FROM numbers(10) LIMIT 1 OFFSET 1", None).unwrap(),
        "sum(number)\n"
    );
    assert_eq!(run("SELECT 1 LIMIT 1 OFFSET 1", None).unwrap(), "1\n");
    assert!(SortedRows::with_offset(Some(usize::MAX), 1, false).is_err());
}

#[test]
fn unfiltered_number_offsets_skip_directly_to_the_first_result() {
    assert_eq!(
        run(
            "SELECT number FROM numbers(1000000) LIMIT 1 OFFSET 999999",
            None
        )
        .unwrap(),
        "number\n999999\n"
    );
    assert_eq!(
        run("SELECT number FROM numbers(7, 4) LIMIT 2 OFFSET 2", None).unwrap(),
        "number\n9\n10\n"
    );
    assert_eq!(
        run("SELECT number FROM numbers(7, 4) LIMIT 2 OFFSET 1000", None).unwrap(),
        "number\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(18446744073709551613, 2) LIMIT 1 OFFSET 1",
            None
        )
        .unwrap(),
        "number\n18446744073709551614\n"
    );
    // Filtered, disjunctive, and sorted queries apply OFFSET to matched rows.
    assert_eq!(
        run(
            "SELECT number FROM numbers(7, 4) WHERE number >= 8 LIMIT 2 OFFSET 1",
            None
        )
        .unwrap(),
        "number\n9\n10\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(10) WHERE number < 3 OR number > 6 LIMIT 2 OFFSET 1",
            None
        )
        .unwrap(),
        "number\n1\n2\n"
    );
    // The OR compiler always builds one branch, even for an empty WHERE.
    assert_eq!(
        run("SELECT count(*) FROM numbers(1000000)", None).unwrap(),
        "count\n1000000\n"
    );
}

#[test]
fn sorting_keeps_stable_ties_and_large_integer_precision() {
    let mut sorted = SortedRows::new(Some(3), true);
    for (key, value) in [(4, 0), (2, 1), (4, 2), (5, 3)] {
        sorted
            .push(Number::Signed(key), vec![Number::Unsigned(value)])
            .unwrap();
    }
    let mut output = Vec::new();
    sorted.write(&mut output).unwrap();
    assert_eq!(output, b"3\n0\n2\n");

    let mut sorted = SortedRows::new(Some(1), false);
    for key in [u128::MAX, 0, u128::MAX - 1] {
        sorted
            .push(Number::Unsigned(key), vec![Number::Unsigned(key)])
            .unwrap();
    }
    let mut output = Vec::new();
    sorted.write(&mut output).unwrap();
    assert_eq!(output, b"0\n");
}

#[test]
fn count_without_where_uses_part_metadata() {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-count-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("count.txt"), "42\n").unwrap();
    fs::write(
        dir.join("columns.txt"),
        "columns format version: 1\n1 columns:\n\u{0060}city\u{0060} String\n",
    )
    .unwrap();
    assert_eq!(
        run("SELECT count(*) FROM part", Some(&dir)).unwrap(),
        "count\n42\n"
    );
    assert_eq!(
        run("SELECT count(*) FROM part LIMIT 0", Some(&dir)).unwrap(),
        "count\n"
    );
    assert!(run("SELECT city FROM part", Some(&dir)).is_err());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn checks_predicates_on_empty_parts_and_before_limit_zero() {
    let dir = std::env::temp_dir().join(format!(
        "rusthouse-empty-where-{}-{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    fs::write(dir.join("count.txt"), "0\n").unwrap();
    fs::write(
        dir.join("columns.txt"),
        "columns format version: 1\n1 columns:\n`id` UInt64\n",
    )
    .unwrap();

    for statement in [
        "SELECT id FROM part WHERE id = -1",
        "SELECT id FROM part WHERE id = -1 LIMIT 0",
        "SELECT count(*) FROM part WHERE id = -1",
    ] {
        let select = parse_select(statement).unwrap();
        let mut output = Vec::new();
        assert!(
            matches!(
                execute(&select, Some(&dir), &mut output),
                Err(ExecutionError::Invalid(_))
            ),
            "{statement}"
        );
        assert!(output.is_empty());
    }

    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn rejects_unknown_sources_and_missing_part_directory() {
    assert!(matches!(
        run("SELECT id FROM other", Some(Path::new("."))),
        Err(ExecutionError::Invalid(_))
    ));
    assert!(matches!(
        run("SELECT id FROM part", None),
        Err(ExecutionError::Invalid(_))
    ));
}

#[test]
fn numeric_comparisons_do_not_lose_large_integer_precision() {
    assert!(number_matches(
        Number::Unsigned(u128::MAX),
        Number::Unsigned(u128::MAX - 1),
        Operator::Gt,
    ));
    assert!(number_matches(
        Number::Signed(i128::MIN),
        Number::Signed(0),
        Operator::Lt,
    ));
    assert!(!number_matches(
        Number::Float(f64::NAN),
        Number::Float(1.0),
        Operator::Eq,
    ));
    assert!(number_matches(
        number_at(&NumericColumn::UInt128(vec![u128::MAX]), 0),
        parse_numeric("UInt128", &u128::MAX.to_string()).unwrap(),
        Operator::Eq,
    ));
    assert!(number_matches(
        number_at(&NumericColumn::Int128(vec![i128::MIN]), 0),
        parse_numeric("Int128", &i128::MIN.to_string()).unwrap(),
        Operator::Eq,
    ));
}
#[test]
fn group_entry_path_counts_new_and_existing_keys() {
    let mut groups = GroupedCounts::default();
    for value in 0u64..4096 {
        groups.push(Number::Unsigned(u128::from(value))).unwrap();
    }
    for value in (0u64..4096).rev() {
        groups.push(Number::Unsigned(u128::from(value))).unwrap();
    }
    assert_eq!(groups.by_key.len(), 4096);
    assert_eq!(groups.rows.len(), 4096);
    assert!(groups.rows.iter().all(|(_, count)| *count == 2));
}

#[test]
fn group_entry_path_preserves_count_overflow_errors() {
    let mut groups = GroupedCounts::default();
    groups.push(Number::Unsigned(7)).unwrap();
    groups.rows[0].1 = u64::MAX;
    assert!(matches!(
        groups.push(Number::Unsigned(7)),
        Err(ExecutionError::Invalid(_))
    ));
}
#[test]
fn evaluates_unary_not_with_comparisons_lists_and_ranges() {
    assert_eq!(
        run("SELECT number FROM numbers(6) WHERE NOT number < 3", None).unwrap(),
        "number\n3\n4\n5\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(6) WHERE NOT number IN (1, 3, 5)",
            None
        )
        .unwrap(),
        "number\n0\n2\n4\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(6) WHERE NOT number BETWEEN 2 AND 4",
            None
        )
        .unwrap(),
        "number\n0\n1\n5\n"
    );
    assert_eq!(
        run(
            "SELECT number FROM numbers(6) WHERE NOT number NOT BETWEEN 2 AND 4",
            None
        )
        .unwrap(),
        "number\n2\n3\n4\n"
    );
}
