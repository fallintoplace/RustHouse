//! A typed representation of the SQL rewrite selected for each shard.
//! Keep rewrite ordering and existing error responses stable.
use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShardSqlPlan {
    Sorted,
    MultiGroupedDistinct,
    GroupedDistinct,
    DistinctRows,
    Scan,
    GroupedAggregate,
    GroupedCount,
    Rms,
    Avg,
    Range,
    Uniq,
    CountDistinct,
    Passthrough,
}

pub(super) fn is_supported_distributed_select(select: &Select) -> bool {
    is_safe_scan(select)
        || is_safe_count(select)
        || is_safe_distinct_scan(select)
        || is_safe_multi_grouped_distinct(select)
        || is_safe_grouped_distinct(select)
        || is_safe_count_distinct(select)
        || is_safe_uniq(select)
        || is_safe_sorted_scan(select)
        || is_safe_count_if(select)
        || is_safe_grouped_count(select)
        || is_safe_grouped_multi_count(select)
        || is_safe_grouped_multi_count_column(select)
        || is_safe_grouped_multi_bitwise(select)
        || is_safe_grouped_multi_product(select)
        || is_safe_grouped_multi_sumsquares(select)
        || is_safe_grouped_multi_countif(select)
        || is_safe_grouped_multi_extremum(select)
        || is_safe_grouped_multi_sum(select)
        || is_safe_grouped_sum(select)
        || is_safe_grouped_extremum(select)
        || is_safe_grouped_product(select)
        || is_safe_grouped_bitwise(select)
        || is_safe_sum(select)
        || is_safe_sum_squares(select)
        || is_safe_product(select)
        || is_safe_rms(select)
        || is_safe_avg(select)
        || is_safe_range(select)
        || is_safe_bitwise(select)
        || is_safe_extremum(select)
}

impl ShardSqlPlan {
    fn for_select(select: &Select) -> Self {
        if is_safe_sorted_scan(select) {
            Self::Sorted
        } else if is_safe_multi_grouped_distinct(select) {
            Self::MultiGroupedDistinct
        } else if is_safe_grouped_distinct(select) {
            Self::GroupedDistinct
        } else if is_safe_distinct_scan(select) {
            Self::DistinctRows
        } else if is_safe_scan(select) {
            Self::Scan
        } else if is_safe_grouped_sum(select)
            || is_safe_grouped_extremum(select)
            || is_safe_grouped_product(select)
            || is_safe_grouped_bitwise(select)
        {
            Self::GroupedAggregate
        } else if is_safe_grouped_count(select) {
            Self::GroupedCount
        } else if is_safe_rms(select) {
            Self::Rms
        } else if is_safe_avg(select) {
            Self::Avg
        } else if is_safe_range(select) {
            Self::Range
        } else if is_safe_uniq(select) {
            Self::Uniq
        } else if matches!(select.projection, Projection::CountDistinct { .. }) {
            Self::CountDistinct
        } else {
            Self::Passthrough
        }
    }
}

pub(super) fn rewrite_shard_sql(sql: &str, select: &Select) -> Result<String, &'static str> {
    match ShardSqlPlan::for_select(select) {
        ShardSqlPlan::Sorted => {
            sorted_scan_sql(sql, select).ok_or("unsupported distributed sort\n")
        },
        ShardSqlPlan::MultiGroupedDistinct => {
            multi_grouped_distinct_shard_sql(sql, select).ok_or("unsupported grouped DISTINCT\n")
        },
        ShardSqlPlan::GroupedDistinct => {
            grouped_distinct_shard_sql(sql, select).ok_or("unsupported grouped DISTINCT\n")
        },
        ShardSqlPlan::DistinctRows => {
            unpaged_scan_sql(sql, select).ok_or("unsupported distributed DISTINCT pagination\n")
        },
        ShardSqlPlan::Scan => {
            bounded_scan_sql(sql, select).ok_or("unsupported distributed pagination\n")
        },
        ShardSqlPlan::GroupedAggregate => {
            grouped_count_shard_sql(sql, select).ok_or("unsupported global grouped aggregate\n")
        },
        ShardSqlPlan::GroupedCount => {
            grouped_count_shard_sql(sql, select).ok_or("unsupported global grouped query\n")
        },
        ShardSqlPlan::Rms => {
            rms_shard_sql(sql, select).ok_or("unsupported distributed RMS\n")
        },
        ShardSqlPlan::Avg => {
            avg_shard_sql(sql, select).ok_or("unsupported distributed AVG\n")
        },
        ShardSqlPlan::Range => {
            range_shard_sql(sql, select).ok_or("unsupported distributed RANGE\n")
        },
        ShardSqlPlan::Uniq => {
            uniq_state_shard_sql(sql, select).ok_or("unsupported distributed uniq query\n")
        },
        ShardSqlPlan::CountDistinct => {
            distinct_shard_sql(sql, select).ok_or("unsupported distributed DISTINCT query\n")
        },
        ShardSqlPlan::Passthrough => Ok(sql.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_keep_original_shard_sql() {
        let sql = "SELECT count(*) FROM events";
        let select = parse_select(sql).unwrap();
        assert!(is_supported_distributed_select(&select));
        assert_eq!(ShardSqlPlan::for_select(&select), ShardSqlPlan::Passthrough);
        assert_eq!(rewrite_shard_sql(sql, &select).unwrap(), sql);
    }

    #[test]
    fn numbers_are_not_shard_tables() {
        let select = parse_select("SELECT number FROM numbers(10)").unwrap();
        assert!(!is_supported_distributed_select(&select));
    }
}
