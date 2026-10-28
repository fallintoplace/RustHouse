//! Analytics report queries can mix all supported numeric metrics by group.
use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn grouped_stats_are_computed_in_one_scan() {
    let path = std::env::temp_dir().join(format!(
        "rusthouse-grouped-analytics-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let columns = [
        Column { name: "region".into(), data_type: "String".into() },
        Column { name: "value".into(), data_type: "UInt64".into() },
    ];
    let rows = [("east","0"),("west","1"),("east","2"),("west","3"),("east","4")];
    let records = rows.iter().map(|(region, value)| {
        vec![Some((*region).into()),Some((*value).into())]
    }).collect::<Vec<_>>();
    write_wide_part(&path,&columns,&records).unwrap();
    let run = |sql: &str| -> String {
        let mut output = Vec::new();
        execute(&parse_select(sql).unwrap(), Some(&path), &mut output).unwrap();
        String::from_utf8(output).unwrap()
    };
    assert_eq!(
        run("SELECT region, sum(value), countIf(value), sumSquares(value), median(value) FROM part GROUP BY region ORDER BY region"),
        "region\tsum(value)\tcountIf(value)\tsumSquares(value)\tmedian(value)\neast\t6\t2\t20\t2\nwest\t4\t2\t10\t2\n"
    );
    assert_eq!(
        run("SELECT region, groupBitOr(value), groupBitAnd(value), groupBitXor(value), count(value) FROM part GROUP BY region ORDER BY region"),
        "region\tgroupBitOr(value)\tgroupBitAnd(value)\tgroupBitXor(value)\tcount(value)\neast\t6\t0\t6\t3\nwest\t3\t1\t2\t2\n"
    );
    let output = run("SELECT region, avg(value), varPop(value), stddevPop(value), rms(value) FROM part GROUP BY region ORDER BY region");
    let actual = output.lines().skip(1).map(|line| line.split('\t').collect::<Vec<_>>()).collect::<Vec<_>>();
    assert_eq!(actual.len(), 2);
    assert_eq!(actual[0][0], "east");
    assert_eq!(actual[1][0], "west");
    for (group, means, variances, root_variances, rms) in [
        (&actual[0],2.0,8.0/3.0,(8.0_f64/3.0).sqrt(),(20.0_f64/3.0).sqrt()),
        (&actual[1],2.0,1.0,1.0,5.0_f64.sqrt()),
    ] {
        for (cell, expected) in group.iter().skip(1).zip([means,variances,root_variances,rms]) {
            assert!((cell.parse::<f64>().unwrap()-expected).abs()<1e-10);
        }
    }
    fs::remove_dir_all(path).unwrap();
}

#[test]
fn parser_accepts_multi_metric_analytics_families_with_group_key() {
    for name in ["countIf","product","sumSquares","range","rms","varPop","varSamp",
        "stddevPop","stddevSamp","median","any","anyLast","groupBitAnd","groupBitOr","groupBitXor"] {
        let query = format!("SELECT region, sum(value), {name}(value) FROM part GROUP BY region");
        assert!(parse_select(&query).is_ok(), "{query}");
    }
}
