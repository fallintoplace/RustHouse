use super::*;

#[test]
fn cheap_conditions_run_before_expensive_like_conditions() {
    let mut conditions = vec![
        Condition {
            index: 0,
            test: Test::Like {
                pattern: compile_like("match").unwrap(),
                case_insensitive: false,
                negated: false,
            },
        },
        Condition {
            index: 1,
            test: Test::IsNull(false),
        },
    ];
    order_compiled_conditions(&mut conditions);
    assert_eq!(conditions[0].index, 1);
    assert_eq!(conditions[1].index, 0);
}
