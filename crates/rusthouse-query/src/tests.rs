use super::*;

#[test]
fn parses_median_as_a_numeric_aggregate() {
    assert_eq!(
        parse_select("SELECT median(number) FROM numbers(5)")
            .unwrap()
            .projection,
        Projection::Aggregate {
            function: AggregateFunction::Median,
            column: "number".into(),
        }
    );
    assert!(parse_select("SELECT median() FROM numbers(1)").is_err());
    assert!(parse_select("SELECT median(number, number) FROM numbers(1)").is_err());
}

#[test]
fn parses_scalar_and_table_queries() {
    assert_eq!(
        parse_select(" SELECT -42; ").unwrap(),
        Select {
            projection: Projection::Literal(-42),
            alias: None,
            rename: None,
            distinct: false,
            source: None,
            predicates: Vec::new(),
            or_predicates: Vec::new(),
            order_by: None,
            order_by_extra: Vec::new(),
            having: None,
            limit: None,
            offset: None,
        }
    );
    assert_eq!(
        parse_select("select id, price from part limit 2;").unwrap(),
        Select {
            projection: Projection::Columns(vec!["id".into(), "price".into()]),
            alias: None,
            rename: None,
            distinct: false,
            source: Some(Source::Table("part".into())),
            predicates: Vec::new(),
            or_predicates: Vec::new(),
            order_by: None,
            order_by_extra: Vec::new(),
            having: None,
            limit: Some(2),
            offset: None,
        }
    );
    assert_eq!(
        parse_select("SELECT count, id FROM part")
            .unwrap()
            .projection,
        Projection::Columns(vec!["count".into(), "id".into()])
    );
    assert_eq!(
        parse_select("SELECT * FROM part").unwrap().projection,
        Projection::All
    );
    assert_eq!(
        parse_select("SeLeCt count(*) FROM part")
            .unwrap()
            .projection,
        Projection::Count
    );
}

#[test]
fn parses_wildcard_rename_and_rejects_invalid_forms() {
    let query = parse_select(
        "SELECT DISTINCT * ReNaMe number aS value FROM numbers(5) ORDER BY value DESC LIMIT 2",
    )
    .unwrap();
    assert_eq!(query.projection, Projection::All);
    assert_eq!(query.rename, Some(("number".into(), "value".into())));
    assert_eq!(query.order_by.unwrap().column, "value");
    for sql in [
        "SELECT * RENAME number TO value FROM numbers(1)",
        "SELECT * RENAME number AS FROM numbers(1)",
        "SELECT * RENAME AS value FROM numbers(1)",
        "SELECT number RENAME number AS value FROM numbers(1)",
        "SELECT * RENAME number AS value, other AS name FROM numbers(1)",
        "SELECT * RENAME number AS value RENAME number AS again FROM numbers(1)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn parses_single_column_result_aliases() {
    assert_eq!(
        parse_select("SELECT number AS n FROM numbers(10)")
            .unwrap()
            .alias
            .as_deref(),
        Some("n")
    );
    assert_eq!(
        parse_select("SELECT count(*) AS total FROM numbers(10)")
            .unwrap()
            .alias
            .as_deref(),
        Some("total")
    );
    assert_eq!(
        parse_select("SELECT -3 AS result")
            .unwrap()
            .alias
            .as_deref(),
        Some("result")
    );
    for sql in [
        "SELECT number AS FROM numbers(5)",
        "SELECT number, number AS alias FROM numbers(5)",
        "SELECT * AS everything FROM numbers(5)",
        "SELECT number AS 3 FROM numbers(5)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn projects_bounded_mixed_constant_rows() {
    let select = parse_select("SELECT 'Ada' AS name, -7 AS id, NULL AS missing").unwrap();
    assert_eq!(
        select.projection,
        Projection::Constants(vec![
            Constant {
                value: ConstantValue::Text("Ada".into()),
                alias: Some("name".into()),
            },
            Constant {
                value: ConstantValue::Integer(-7),
                alias: Some("id".into()),
            },
            Constant {
                value: ConstantValue::Null,
                alias: Some("missing".into()),
            },
        ])
    );
    let mut output = Vec::new();
    execute(&select, None, &mut output).unwrap();
    assert_eq!(output, b"name\tid\tmissing\nAda\t-7\t\\N\n");

    let mut output = Vec::new();
    execute(
        &parse_select("SELECT 'hi', NULL LIMIT 0").unwrap(),
        None,
        &mut output,
    )
    .unwrap();
    assert_eq!(output, b"'hi'\tNULL\n");

    let mut output = Vec::new();
    execute(
        &parse_select("SELECT 'hi', 2 LIMIT 1 OFFSET 1").unwrap(),
        None,
        &mut output,
    )
    .unwrap();
    assert_eq!(output, b"'hi'\t2\n");

    for sql in [
        "SELECT 1,",
        "SELECT 'hi' FROM numbers(2)",
        "SELECT NULL AS",
        "SELECT 1, unknown",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn accepts_composite_order_by_keys_and_bounds_the_count() {
    let parsed =
        parse_select("SELECT id, price FROM part ORDER BY 2 DESC, id ASC, 1 LIMIT 3").unwrap();
    assert_eq!(parsed.order_by.unwrap().column, "price");
    assert_eq!(
        parsed.order_by_extra,
        vec![
            OrderBy {
                column: "id".into(),
                descending: false,
                nulls_first: None,
            },
            OrderBy {
                column: "id".into(),
                descending: false,
                nulls_first: None,
            },
        ]
    );
    let grouped =
        parse_select("SELECT id, count(*) FROM part GROUP BY id ORDER BY count DESC, id").unwrap();
    assert_eq!(grouped.order_by_extra[0].column, "id");
    for sql in [
        "SELECT id FROM part ORDER BY id,",
        "SELECT id FROM part ORDER BY id, 2",
        "SELECT count(*) FROM part ORDER BY id, id",
        "SELECT id, count(*) FROM part GROUP BY id ORDER BY id, missing",
        "SELECT id FROM part ORDER BY id, id, id, id, id, id, id, id, id",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn parses_generated_numbers_and_rejects_invalid_counts() {
    let query = parse_select("SELECT number FROM numbers(12) WHERE number >= 5 LIMIT 2").unwrap();
    assert_eq!(query.source, Some(Source::Numbers(12)));
    assert_eq!(query.projection, Projection::Columns(vec!["number".into()]));
    assert_eq!(
        parse_select("SELECT count(*) FROM NUMBERS(0)")
            .unwrap()
            .source,
        Some(Source::Numbers(0))
    );
    for sql in [
        "SELECT number FROM numbers()",
        "SELECT number FROM numbers(-1)",
        "SELECT number FROM numbers(1.5)",
        "SELECT number FROM numbers(1000000001)",
        "SELECT number FROM numbers(18446744073709551616)",
        "SELECT number FROM numbers(3",
        "SELECT number FROM numbers",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn parses_numbers_start_count_with_checked_bounds() {
    assert_eq!(
        parse_select("SELECT number FROM numbers(10, 3)")
            .unwrap()
            .source,
        Some(Source::NumbersRange {
            start: 10,
            count: 3
        })
    );
    assert_eq!(
        parse_select("SELECT count(*) FROM numbers(18446744073709551615, 0)")
            .unwrap()
            .source,
        Some(Source::NumbersRange {
            start: u64::MAX,
            count: 0
        })
    );
    for sql in [
        "SELECT number FROM numbers(1,)",
        "SELECT number FROM numbers(1,-1)",
        "SELECT number FROM numbers(1,1000000001)",
        "SELECT number FROM numbers(18446744073709551615,2)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn parses_stepped_numbers_with_bounded_ranges() {
    let query = parse_select("SELECT number FROM numbers(4, 9, 3)").unwrap();
    assert_eq!(
        query.source,
        Some(Source::NumbersStepped {
            start: 4,
            length: 9,
            step: 3,
        })
    );
    for sql in [
        "SELECT number FROM numbers(0, 10, 0)",
        "SELECT number FROM numbers(0, 10, -1)",
        "SELECT number FROM numbers(0, 10, 2, 3)",
        "SELECT number FROM numbers(18446744073709551615, 2, 1)",
        "SELECT number FROM numbers(0, 1000000001, 2)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn parses_like_predicates_and_rejects_nontext_patterns() {
    let query =
        parse_select("SELECT service FROM part WHERE service LIKE 'a%' AND service NOT LIKE '%z'")
            .unwrap();
    assert_eq!(
        query.predicates,
        vec![
            Predicate::Like {
                column: "service".into(),
                pattern: "a%".into(),
                case_insensitive: false,
                negated: false,
            },
            Predicate::Like {
                column: "service".into(),
                pattern: "%z".into(),
                case_insensitive: false,
                negated: true,
            },
        ]
    );
    let negated = parse_select("SELECT service FROM part WHERE NOT service LIKE 'd%'").unwrap();
    assert_eq!(
        negated.predicates,
        vec![Predicate::Like {
            column: "service".into(),
            pattern: "d%".into(),
            case_insensitive: false,
            negated: true,
        }]
    );
    for invalid in [
        "SELECT service FROM part WHERE service LIKE 42",
        "SELECT service FROM part WHERE service LIKE",
        "SELECT service FROM part WHERE service NOT LIKE NULL",
    ] {
        assert!(parse_select(invalid).is_err(), "{invalid}");
    }
}
#[test]
fn parses_ilike_and_rejects_malformed_patterns() {
    let select = parse_select(
        "SELECT service FROM part WHERE service ILIKE 'API%' OR service NOT ILIKE 'DB'",
    )
    .unwrap();
    assert_eq!(
        select.predicates,
        vec![Predicate::Like {
            column: "service".into(),
            pattern: "API%".into(),
            case_insensitive: true,
            negated: false,
        }]
    );
    assert_eq!(
        select.or_predicates,
        vec![vec![Predicate::Like {
            column: "service".into(),
            pattern: "DB".into(),
            case_insensitive: true,
            negated: true,
        }]]
    );
    for sql in [
        "SELECT service FROM part WHERE service ILIKE 1",
        "SELECT service FROM part WHERE service ILIKE NULL",
        "SELECT service FROM part WHERE service LIKE 'abc\\'",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn parses_numeric_aggregates_and_rejects_unsupported_functions() {
    for (name, expected) in [
        ("SUM", AggregateFunction::Sum),
        ("AVG", AggregateFunction::Avg),
        ("MIN", AggregateFunction::Min),
        ("MAX", AggregateFunction::Max),
        ("VARPOP", AggregateFunction::VarPop),
        ("STDDEVPOP", AggregateFunction::StddevPop),
        ("MEDIAN", AggregateFunction::Median),
    ] {
        let sql = format!("SELECT {name}(price) FROM part WHERE id > 2");
        assert_eq!(
            parse_select(&sql).unwrap().projection,
            Projection::Aggregate {
                function: expected,
                column: "price".into(),
            }
        );
    }
    for sql in [
        "SELECT sum(*) FROM part",
        "SELECT sum() FROM part",
        "SELECT sum(a, b) FROM part",
        "SELECT avg(number) FROM numbers(3) ORDER BY number",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn parses_count_column_and_rejects_malformed_arguments() {
    let parsed = parse_select("SELECT COUNT(number) AS total FROM numbers(10)").unwrap();
    assert_eq!(
        parsed.projection,
        Projection::CountColumn {
            column: "number".into(),
        }
    );
    assert_eq!(parsed.alias.as_deref(), Some("total"));
    for sql in [
        "SELECT count(number, other) FROM numbers(10)",
        "SELECT count(1) FROM numbers(10)",
        "SELECT count(*number) FROM numbers(10)",
        "SELECT count(number FROM numbers(10)",
        "SELECT count(number) FROM numbers(5) ORDER BY number",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn parses_count_distinct_projection() {
    assert_eq!(
        parse_select("SELECT count(DISTINCT number) FROM numbers(10)")
            .unwrap()
            .projection,
        Projection::CountDistinct {
            column: "number".into(),
        }
    );
    for sql in [
        "SELECT count(DISTINCT) FROM numbers(10)",
        "SELECT count(DISTINCT number, other) FROM numbers(10)",
        "SELECT DISTINCT count(DISTINCT number) FROM numbers(10)",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn parses_order_by_and_rejects_invalid_ordering() {
    let select = parse_select(
        "SELECT number FROM numbers(10) WHERE number > 2 ORDER BY number DESC LIMIT 3",
    )
    .unwrap();
    assert_eq!(
        select.order_by,
        Some(OrderBy {
            column: "number".into(),
            descending: true,
            nulls_first: None,
        })
    );
    assert_eq!(
        parse_select("SELECT id FROM part ORDER BY price ASC")
            .unwrap()
            .order_by,
        Some(OrderBy {
            column: "price".into(),
            descending: false,
            nulls_first: None,
        })
    );
    for sql in [
        "SELECT number FROM numbers(3) ORDER number",
        "SELECT number FROM numbers(3) ORDER BY",
        "SELECT number FROM numbers(3) ORDER BY number DESC ASC",
        "SELECT number FROM numbers(3) LIMIT 1 ORDER BY number",
        "SELECT count(*) FROM numbers(3) ORDER BY number",
        "SELECT 1 ORDER BY number",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn parses_explicit_nulls_precedence_per_order_term() {
    let query = parse_select(
        "SELECT attempt, message FROM part ORDER BY attempt DESC NULLS FIRST, message ASC NULLS LAST",
    )
    .unwrap();
    assert_eq!(query.order_by.as_ref().unwrap().nulls_first, Some(true));
    assert_eq!(query.order_by_extra[0].nulls_first, Some(false));

    for sql in [
        "SELECT attempt FROM part ORDER BY attempt NULLS",
        "SELECT attempt FROM part ORDER BY attempt NULLS MIDDLE",
        "SELECT attempt FROM part ORDER BY attempt NULLS FIRST NULLS LAST",
        "SELECT attempt FROM part ORDER BY attempt NULLS LAST, message NULLS MIDDLE",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn grouped_queries_can_order_by_count() {
    for order in ["count", "count()", "count(*)"] {
        let sql = format!(
            "SELECT number, count(*) FROM numbers(10) GROUP BY number ORDER BY {order} DESC"
        );
        let query = parse_select(&sql).unwrap();
        assert_eq!(
            query.order_by,
            Some(OrderBy {
                column: "count".into(),
                descending: true,
                nulls_first: None,
            })
        );
    }
    assert!(
        parse_select(
            "SELECT number, count(*) FROM numbers(10) GROUP BY number ORDER BY count(foo)"
        )
        .is_err()
    );
}

#[test]
fn parses_numeric_where_operators() {
    for (input, expected) in [
        ("=", Operator::Eq),
        ("!=", Operator::Ne),
        ("<>", Operator::Ne),
        ("<", Operator::Lt),
        ("<=", Operator::Le),
        (">", Operator::Gt),
        (">=", Operator::Ge),
    ] {
        let sql = format!("SELECT id FROM part WHERE id {input} -12.5 LIMIT 0");
        let query = parse_select(&sql).unwrap();
        assert_eq!(
            query.predicates.into_iter().next().unwrap(),
            Predicate::Comparison {
                column: "id".into(),
                operator: expected,
                value: "-12.5".into(),
            }
        );
        assert_eq!(query.limit, Some(0));
    }
}

#[test]
fn allows_quoted_predicate_values_for_type_validation_at_execution() {
    assert!(parse_select("SELECT id FROM part WHERE id > 'abc'").is_ok());
}

#[test]
fn parses_scalar_arithmetic_with_checked_overflow_and_precedence() {
    for (sql, value) in [
        ("SELECT 1 + 2 * 3", 7),
        ("SELECT (1 + 2) * 3", 9),
        ("SELECT 10 % 4", 2),
        ("SELECT 7 / 2", 3),
        ("SELECT 12 - 5 * 2", 2),
        ("SELECT 12-5*2", 2),
        ("SELECT 10 + -4", 6),
        ("SELECT - 2 * 3", -6),
        ("SELECT -9223372036854775808", i64::MIN),
        ("SELECT 1 - -2", 3),
    ] {
        assert_eq!(
            parse_select(sql).unwrap().projection,
            Projection::Literal(value),
            "{sql}"
        );
    }
    assert_eq!(
        parse_select("SELECT 1--2").unwrap().projection,
        Projection::Literal(1)
    );
    let aliased = parse_select("SELECT 5 * (2 + 1) AS answer LIMIT 1").unwrap();
    assert_eq!(aliased.projection, Projection::Literal(15));
    assert_eq!(aliased.alias.as_deref(), Some("answer"));
    for sql in [
        "SELECT 1 +",
        "SELECT 1 -",
        "SELECT 1 % 0",
        "SELECT 1 + 9223372036854775807",
        "SELECT 9223372036854775807 * 2",
        "SELECT -9223372036854775808 - 1",
        "SELECT 1 + (2 * 3",
        "SELECT 1 + 2 FROM part",
        "SELECT 1.5 + 2",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
    let deeply_nested = format!("SELECT {}1{}", "(".repeat(20), ")".repeat(20));
    assert!(parse_select(&deeply_nested).is_err());
}

#[test]
fn rejects_malformed_and_unsupported_statements() {
    for sql in [
        "",
        "DELETE FROM part",
        "SELECT *",
        "SELECT 1 FROM part",
        "SELECT 1; SELECT 2",
        "SELECT 1;;",
        "SELECT id, FROM part",
        "SELECT id FROM part LIMIT -1",
        "SELECT id FROM part LIMIT 1.5",
        "SELECT id FROM part WHERE id =",
        "SELECT id FROM part LIMIT 184467440737095516161",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn parses_and_conditions_and_rejects_incomplete_filters() {
    let query =
        parse_select("SELECT number FROM numbers(12) WHERE number >= 3 AND number < 8 LIMIT 4")
            .unwrap();
    assert_eq!(
        query.predicates,
        vec![
            Predicate::Comparison {
                column: "number".into(),
                operator: Operator::Ge,
                value: "3".into(),
            },
            Predicate::Comparison {
                column: "number".into(),
                operator: Operator::Lt,
                value: "8".into(),
            },
        ]
    );
    for sql in [
        "SELECT number FROM numbers(5) WHERE number > 1 AND",
        "SELECT number FROM numbers(5) WHERE number > 1 AND AND number < 4",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
    let many = format!(
        "SELECT number FROM numbers(5) WHERE {}",
        vec!["number > 0"; 17].join(" AND ")
    );
    assert!(parse_select(&many).is_err());
}

#[test]
fn parses_between_as_inclusive_range_predicates() {
    let query = parse_select(
        "SELECT number FROM numbers(12) WHERE number BETWEEN 3 AND 8 AND number != 4 LIMIT 3",
    )
    .unwrap();
    assert_eq!(
        query.predicates,
        vec![
            Predicate::Comparison {
                column: "number".into(),
                operator: Operator::Ge,
                value: "3".into(),
            },
            Predicate::Comparison {
                column: "number".into(),
                operator: Operator::Le,
                value: "8".into(),
            },
            Predicate::Comparison {
                column: "number".into(),
                operator: Operator::Ne,
                value: "4".into(),
            },
        ]
    );
    for sql in [
        "SELECT number FROM numbers(5) WHERE number BETWEEN",
        "SELECT number FROM numbers(5) WHERE number BETWEEN 1",
        "SELECT number FROM numbers(5) WHERE number BETWEEN 1 OR 2",
        "SELECT number FROM numbers(5) WHERE number BETWEEN 1 AND",
        "SELECT number FROM numbers(5) WHERE number BETWEEN 1 AND 3 AND",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
    let too_many = format!(
        "SELECT number FROM numbers(5) WHERE {} AND number BETWEEN 1 AND 2",
        vec!["number > 0"; 15].join(" AND ")
    );
    assert!(parse_select(&too_many).is_err());
}

#[test]
fn parses_not_between_with_other_predicates() {
    let parsed = parse_select(
        "SELECT number FROM numbers(10) WHERE number NOT BETWEEN 3 AND 6 AND number < 9",
    )
    .unwrap();
    assert_eq!(
        parsed.predicates[0],
        Predicate::NotBetween {
            column: "number".into(),
            lower: "3".into(),
            upper: "6".into(),
        }
    );
    assert!(parse_select("SELECT number FROM numbers(5) WHERE number NOT BETWEEN 2").is_err());
}

#[test]
fn parses_grouped_counts_and_rejects_unsupported_grouping() {
    let query = parse_select(
        "SELECT id, COUNT(*) FROM part WHERE id IN (2, 4) GROUP BY id ORDER BY id DESC LIMIT 2 OFFSET 1",
    )
    .unwrap();
    assert_eq!(
        query.projection,
        Projection::GroupedCount {
            column: "id".into(),
        }
    );
    assert_eq!(
        query.order_by,
        Some(OrderBy {
            column: "id".into(),
            descending: true,
            nulls_first: None,
        })
    );
    for sql in [
        "SELECT id, count(*) FROM part",
        "SELECT id, count(*) FROM part GROUP BY price",
        "SELECT id, count(*) FROM part GROUP id",
        "SELECT id, count(*) FROM part GROUP BY id, price",
        "SELECT id, count(*) FROM part GROUP BY id ORDER BY price",
        "SELECT id FROM part GROUP BY id",
        "SELECT count(*) FROM part GROUP BY id",
        "SELECT DISTINCT id, count(*) FROM part GROUP BY id",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn parses_grouped_numeric_aggregates_and_bounds_ordering() {
    for name in ["sum", "avg", "min", "max", "varPop", "stddevPop", "median"] {
        let sql = format!(
            "SELECT service, {name}(status) FROM part WHERE status > 0 GROUP BY service ORDER BY service DESC LIMIT 3"
        );
        let select = parse_select(&sql).unwrap();
        let Projection::GroupedAggregate {
            column,
            function,
            value_column,
        } = select.projection
        else {
            panic!("not a grouped aggregate: {sql}");
        };
        assert_eq!(column, "service");
        assert_eq!(
            function.name().to_ascii_lowercase(),
            name.to_ascii_lowercase()
        );
        assert_eq!(value_column, "status");
    }
    for sql in [
        "SELECT service, sum(status) FROM part",
        "SELECT service, sum(status) FROM part GROUP BY status",
        "SELECT service, sum(status) FROM part GROUP BY service ORDER BY status",
        "SELECT DISTINCT service, sum(status) FROM part GROUP BY service",
        "SELECT service, sum(*) FROM part GROUP BY service",
        "SELECT service, sum(status, id) FROM part GROUP BY service",
        "SELECT service, avg(status) FROM part GROUP BY service HAVING count(*) > 1",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn parses_distinct_and_rejects_invalid_projections() {
    let query = parse_select(
        "SELECT DISTINCT id, price FROM part WHERE id > 1 ORDER BY price DESC LIMIT 3",
    )
    .unwrap();
    assert!(query.distinct);
    assert_eq!(
        query.projection,
        Projection::Columns(vec!["id".into(), "price".into()])
    );
    assert!(
        parse_select("SELECT DISTINCT * FROM part")
            .unwrap()
            .distinct
    );
    for sql in [
        "SELECT DISTINCT 1",
        "SELECT DISTINCT count(*) FROM part",
        "SELECT DISTINCT sum(id) FROM part",
        "SELECT DISTINCT DISTINCT id FROM part",
        "SELECT DISTINCT FROM part",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn parses_having_counts_after_grouping() {
    let query = parse_select(
        "SELECT number, count(*) FROM numbers(10) GROUP BY number HAVING count(*) >= 2 ORDER BY number",
    )
    .unwrap();
    assert_eq!(
        query.having,
        Some(Having {
            operator: Operator::Ge,
            value: 2,
            aggregate_index: None,
        })
    );
    for sql in [
        "SELECT number FROM numbers(10) HAVING count(*) > 1",
        "SELECT number, count(*) FROM numbers(10) GROUP BY number HAVING sum(number) > 1",
        "SELECT number, count(*) FROM numbers(10) GROUP BY number HAVING count(*) >= -1",
        "SELECT number, count(*) FROM numbers(10) GROUP BY number HAVING count(*) >",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn parses_or_with_and_precedence() {
    let query = parse_select(
        "SELECT number FROM numbers(10) WHERE number = 1 OR number >= 4 AND number < 6",
    )
    .unwrap();
    assert_eq!(query.predicates.len(), 1);
    assert_eq!(query.or_predicates.len(), 1);
    assert_eq!(query.or_predicates[0].len(), 2);
    for sql in [
        "SELECT number FROM numbers(10) WHERE number = 1 OR",
        "SELECT number FROM numbers(10) WHERE number = 1 OR OR number = 2",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn parses_in_and_checks_list_bounds() {
    let query =
        parse_select("SELECT number FROM numbers(9) WHERE number IN (1, 4, 7) AND number > 2")
            .unwrap();
    assert_eq!(
        query.predicates,
        vec![
            Predicate::In {
                column: "number".into(),
                values: vec!["1".into(), "4".into(), "7".into()],
            },
            Predicate::Comparison {
                column: "number".into(),
                operator: Operator::Gt,
                value: "2".into(),
            },
        ]
    );
    for sql in [
        "SELECT number FROM numbers(5) WHERE number IN ()",
        "SELECT number FROM numbers(5) WHERE number IN (1,)",
        "SELECT number FROM numbers(5) WHERE number IN (1 2)",
        "SELECT number FROM numbers(5) WHERE number IN 1",
        "SELECT number FROM numbers(5) WHERE number IN (1, 2",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
    let too_many = format!(
        "SELECT number FROM numbers(9) WHERE number IN ({})",
        vec!["1"; 33].join(",")
    );
    assert!(parse_select(&too_many).is_err());
}

#[test]
fn parses_modulo_filters_and_rejects_invalid_divisors() {
    let query = parse_select("SELECT number FROM numbers(10) WHERE number % 3 = 1").unwrap();
    assert!(matches!(
        query.predicates[0],
        Predicate::Comparison {
            operator: Operator::Modulo(3),
            ..
        }
    ));
    assert!(matches!(
        parse_select("SELECT number FROM numbers(10) WHERE number % 3 != 1")
            .unwrap()
            .predicates[0],
        Predicate::Comparison {
            operator: Operator::ModuloNe(3),
            ..
        }
    ));
    for sql in [
        "SELECT number FROM numbers(10) WHERE number % 0 = 1",
        "SELECT number FROM numbers(10) WHERE number % -1 = 0",
        "SELECT number FROM numbers(10) WHERE number % 2 > 0",
        "SELECT number FROM numbers(10) WHERE number % 2.5 = 1",
        "SELECT number FROM numbers(10) WHERE number % 2 =",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn parses_not_in_numeric_lists() {
    let query =
        parse_select("SELECT number FROM numbers(10) WHERE number NOT IN (1, 4, 7)").unwrap();
    assert_eq!(
        query.predicates,
        vec![Predicate::NotIn {
            column: "number".into(),
            values: vec!["1".into(), "4".into(), "7".into()],
        }]
    );
    for sql in [
        "SELECT number FROM numbers(10) WHERE number NOT IN ()",
        "SELECT number FROM numbers(10) WHERE number NOT IN (1,)",
        "SELECT number FROM numbers(10) WHERE number NOT = 1",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn parses_limit_offset_and_rejects_invalid_pagination() {
    let query =
        parse_select("SELECT number FROM numbers(10) ORDER BY number DESC LIMIT 3 OFFSET 2;")
            .unwrap();
    assert_eq!(query.limit, Some(3));
    assert_eq!(query.offset, Some(2));
    assert_eq!(
        parse_select("SELECT id FROM part LIMIT 0 OFFSET 0")
            .unwrap()
            .offset,
        Some(0)
    );
    for sql in [
        "SELECT number FROM numbers(10) LIMIT 2 OFFSET -1",
        "SELECT number FROM numbers(10) LIMIT 2 OFFSET 1.5",
        "SELECT number FROM numbers(10) LIMIT 2 OFFSET",
        "SELECT number FROM numbers(10) LIMIT 2 OFFSET 184467440737095516160",
        "SELECT number FROM numbers(10) LIMIT 2 OFFSET 1 OFFSET 2",
        "SELECT number FROM numbers(10) LIMIT 2 OFFSET 1 ORDER BY number",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn supports_clickhouse_limit_offset_count_syntax() {
    let query = parse_select("SELECT number FROM numbers(20) LIMIT 3, 2").unwrap();
    assert_eq!(query.limit, Some(2));
    assert_eq!(query.offset, Some(3));
    for sql in [
        "SELECT number FROM numbers(20) LIMIT 1,",
        "SELECT number FROM numbers(20) LIMIT 1, -2",
        "SELECT number FROM numbers(20) LIMIT 1, 2 OFFSET 3",
        "SELECT number FROM numbers(20) LIMIT 1, 184467440737095516160",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn supports_clickhouse_count_without_arguments() {
    assert_eq!(
        parse_select("SELECT count() FROM numbers(10)")
            .unwrap()
            .projection,
        Projection::Count
    );
    assert_eq!(
        parse_select("SELECT number, count() FROM numbers(10) GROUP BY number")
            .unwrap()
            .projection,
        Projection::GroupedCount {
            column: "number".into(),
        }
    );
    for sql in [
        "SELECT count( FROM numbers(2)",
        "SELECT count(,) FROM numbers(2)",
        "SELECT number, count( FROM numbers(2) GROUP BY number",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn bounds_sql_input_and_rejects_invalid_characters() {
    assert!(parse_select(&" ".repeat(4097)).is_err());
    assert!(parse_select("SELECT café FROM part").is_err());
}
#[test]
fn parses_unary_not_on_numeric_predicates() {
    let query = parse_select("SELECT number FROM numbers(6) WHERE NOT number < 3").unwrap();
    assert_eq!(
        query.predicates,
        vec![Predicate::Comparison {
            column: "number".into(),
            operator: Operator::Ge,
            value: "3".into(),
        }]
    );
    assert!(matches!(
        parse_select("SELECT number FROM numbers(6) WHERE NOT number IN (1, 3)")
            .unwrap()
            .predicates[0],
        Predicate::NotIn { .. }
    ));
    assert!(matches!(
        parse_select("SELECT number FROM numbers(6) WHERE NOT number BETWEEN 2 AND 4")
            .unwrap()
            .predicates[0],
        Predicate::NotBetween { .. }
    ));
    let twice =
        parse_select("SELECT number FROM numbers(6) WHERE NOT number NOT BETWEEN 2 AND 4").unwrap();
    assert_eq!(twice.predicates.len(), 2);
    for sql in [
        "SELECT number FROM numbers(6) WHERE number NOT = 3",
        "SELECT number FROM numbers(6) WHERE NOT number % 2 = 1",
        "SELECT number FROM numbers(6) WHERE NOT",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn resolves_one_based_order_by_positions() {
    let query =
        parse_select("SELECT number AS n FROM numbers(10) ORDER BY 1 DESC LIMIT 3").unwrap();
    assert_eq!(
        query.order_by,
        Some(OrderBy {
            column: "number".into(),
            descending: true,
            nulls_first: None,
        })
    );
    let grouped =
        parse_select("SELECT id, count(*) FROM part GROUP BY id ORDER BY 2 DESC").unwrap();
    assert_eq!(
        grouped.order_by,
        Some(OrderBy {
            column: "count".into(),
            descending: true,
            nulls_first: None,
        })
    );
    let grouped_key = parse_select("SELECT id, count(*) FROM part GROUP BY id ORDER BY 1").unwrap();
    assert_eq!(grouped_key.order_by.unwrap().column, "id");
}

#[test]
fn rejects_invalid_order_by_positions() {
    for sql in [
        "SELECT number FROM numbers(3) ORDER BY 0",
        "SELECT number FROM numbers(3) ORDER BY 2",
        "SELECT number FROM numbers(3) ORDER BY -1",
        "SELECT * FROM numbers(3) ORDER BY 1",
        "SELECT number FROM numbers(3) ORDER BY 184467440737095516160",
        "SELECT id, count(*) FROM part GROUP BY id ORDER BY 3",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn parses_standard_fetch_pagination() {
    let query = parse_select(
        "SELECT number FROM numbers(10) ORDER BY number DESC OFFSET 2 ROWS FETCH NEXT 3 ROWS ONLY",
    )
    .unwrap();
    assert_eq!(query.limit, Some(3));
    assert_eq!(query.offset, Some(2));
    let fetch = parse_select("SELECT number FROM numbers(10) FETCH FIRST 0 ROW ONLY").unwrap();
    assert_eq!(fetch.limit, Some(0));
    assert_eq!(fetch.offset, None);
    let legacy = parse_select("SELECT number FROM numbers(10) LIMIT 3 OFFSET 2").unwrap();
    assert_eq!(legacy.limit, query.limit);
    assert_eq!(legacy.offset, query.offset);
}

#[test]
fn rejects_malformed_fetch_clauses() {
    for sql in [
        "SELECT number FROM numbers(10) FETCH 3 ROWS ONLY",
        "SELECT number FROM numbers(10) FETCH FIRST -1 ROWS ONLY",
        "SELECT number FROM numbers(10) FETCH FIRST 3",
        "SELECT number FROM numbers(10) FETCH FIRST 3 ROWS",
        "SELECT number FROM numbers(10) LIMIT 2 FETCH FIRST 3 ROWS ONLY",
        "SELECT number FROM numbers(10) OFFSET -1 ROWS FETCH NEXT 2 ROWS ONLY",
        "SELECT number FROM numbers(10) FETCH NEXT 184467440737095516160 ROWS ONLY",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn accepts_offset_without_limit_or_fetch() {
    let query = parse_select("SELECT number FROM numbers(10) OFFSET 2 ROWS").unwrap();
    assert_eq!(query.limit, None);
    assert_eq!(query.offset, Some(2));
    let sorted =
        parse_select("SELECT number FROM numbers(10) ORDER BY number DESC OFFSET 4").unwrap();
    assert_eq!(sorted.limit, None);
    assert_eq!(sorted.offset, Some(4));
}

#[test]
fn allows_grouped_numeric_ordering_by_aggregate_position() {
    for aggregate in ["sum", "avg", "min", "max"] {
        let query = parse_select(&format!(
            "SELECT category, {aggregate}(amount) FROM part GROUP BY category ORDER BY 2 DESC LIMIT 3"
        )).unwrap();
        assert_eq!(query.order_by.unwrap().column, "#aggregate");
    }
    let query = parse_select(
        "SELECT category, sum(amount) FROM part GROUP BY category ORDER BY 2 DESC, category ASC",
    )
    .unwrap();
    assert_eq!(query.order_by_extra.len(), 1);
    for invalid in [
        "SELECT category, median(amount) FROM part GROUP BY category ORDER BY 2",
        "SELECT category, sum(amount) FROM part GROUP BY category ORDER BY 3",
    ] {
        assert!(parse_select(invalid).is_err(), "{invalid}");
    }
}
