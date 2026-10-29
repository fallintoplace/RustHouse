use rusthouse_query::{parse_select, Projection};

#[test]
fn accepts_group_by_multiple_ordinals_for_counts_and_aggregates() {
    for sql in [
        "SELECT region, city, count(*) FROM part GROUP BY 1, 2",
        "SELECT region, city, sum(sales) FROM part GROUP BY 1, 2",
        "SELECT region, city, sum(sales) FROM part GROUP BY region, 2",
        "SELECT region, city, count(DISTINCT user_id) FROM part GROUP BY 1, 2",
    ] {
        assert!(parse_select(sql).is_ok(), "{sql}");
    }
}

#[test]
fn out_of_range_and_duplicate_group_ordinals_remain_rejected() {
    for sql in [
        "SELECT region, city, count(*) FROM part GROUP BY 0, 2",
        "SELECT region, city, count(*) FROM part GROUP BY 1, 3",
        "SELECT region, city, count(*) FROM part GROUP BY 1, 1",
    ] {
        assert!(parse_select(sql).is_err(), "{sql}");
    }
}

#[test]
fn grouping_positions_do_not_change_projection_key_order() {
    let select = parse_select(
        "SELECT region, city, count(*) FROM part GROUP BY 1, 2"
    ).unwrap();
    match select.projection {
        Projection::GroupedMultiCount { columns } => {
            assert_eq!(columns, vec!["region", "city"]);
        }
        _ => panic!("expected grouped count"),
    }
}
