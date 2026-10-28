use rusthouse_query::{execute, parse_select};
use rusthouse_storage::{Column, write_wide_part};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[test]
fn min_max_on_nullable_text_ignore_nulls_and_respect_filters() {
    let part = std::env::temp_dir().join(format!(
        "rusthouse-text-extrema-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let cols = [Column {
        name: "word".into(),
        data_type: "Nullable(String)".into(),
    }];
    let rows = [
        vec![Some("zeta".into())],
        vec![None],
        vec![Some("alpha".into())],
        vec![Some("beta".into())],
    ];
    write_wide_part(&part, &cols, &rows).unwrap();
    let run = |sql: &str| {
        let mut output = Vec::new();
        execute(&parse_select(sql).unwrap(), Some(&part), &mut output).unwrap();
        String::from_utf8(output).unwrap()
    };
    assert_eq!(run("SELECT MIN(word) FROM part"), "min(word)\nalpha\n");
    assert_eq!(run("SELECT MAX(word) FROM part"), "max(word)\nzeta\n");
    assert_eq!(
        run("SELECT MIN(word) FROM part WHERE word LIKE 'b%'"),
        "min(word)\nbeta\n"
    );
    assert_eq!(
        run("SELECT MAX(word) FROM part WHERE word LIKE 'missing%'"),
        "max(word)\nNULL\n"
    );
    let error = execute(
        &parse_select("SELECT SUM(word) FROM part").unwrap(),
        Some(&part),
        &mut Vec::new(),
    );
    assert!(error.is_err());
    std::fs::remove_dir_all(part).unwrap();
}
