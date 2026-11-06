//! Keep CLI argument and format definitions separate from statement routing and execution.

use super::*;

pub(crate) fn run_sql_args(mut args: impl Iterator<Item = OsString>) -> Result<(), String> {
    let mut query = None;
    let mut file = None;
    let mut script = None;
    let mut read_stdin = false;
    let mut part = None;
    let mut catalog = None;
    let mut database = None;
    let mut format = None;
    let mut output_path = None;
    let mut show_ast = false;
    let mut explain = false;
    let mut time = false;
    let mut max_rows = None;
    while let Some(arg) = args.next() {
        if arg.to_str() == Some("--explain") {
            if explain {
                return Err("duplicate --explain option".into());
            }
            explain = true;
            continue;
        }
        if arg.to_str() == Some("--show-ast") {
            if show_ast {
                return Err("duplicate --show-ast option".into());
            }
            show_ast = true;
            continue;
        }
        if arg.to_str() == Some("--time") {
            if time {
                return Err("duplicate --time option".into());
            }
            time = true;
            continue;
        }
        if arg.to_str() == Some("--stdin") {
            if read_stdin {
                return Err("duplicate --stdin option".into());
            }
            read_stdin = true;
            continue;
        }
        if arg.to_str() == Some("--format") {
            if format.is_some() {
                return Err("duplicate --format option".into());
            }
            format = Some(SqlOutputFormat::from_arg(args.next())?);
        } else if arg.to_str() == Some("--max-rows") {
            if max_rows.is_some() {
                return Err("duplicate --max-rows option".into());
            }
            max_rows = Some(parse_max_rows(args.next())?);
        } else if arg.to_str() == Some("--output") {
            if output_path.is_some() {
                return Err("duplicate --output option".into());
            }
            output_path = Some(PathBuf::from(args.next().ok_or("missing --output path")?));
        } else if arg.to_str() == Some("--part") {
            if part.is_some() {
                return Err("duplicate --part option".into());
            }
            part = Some(PathBuf::from(
                args.next().ok_or("missing --part directory")?,
            ));
        } else if arg.to_str() == Some("--catalog") {
            if catalog.is_some() {
                return Err("duplicate --catalog option".into());
            }
            catalog = Some(PathBuf::from(
                args.next().ok_or("missing --catalog directory")?,
            ));
        } else if arg.to_str() == Some("--database") {
            if database.is_some() {
                return Err("duplicate --database option".into());
            }
            database = Some(PathBuf::from(
                args.next().ok_or("missing --database directory")?,
            ));
        } else if arg.to_str() == Some("--query") || arg.to_str() == Some("-q") {
            if query.is_some() || file.is_some() || script.is_some() {
                return Err("provide SQL text or --query, not both".into());
            }
            query = Some(
                args.next()
                    .ok_or("missing --query SQL text")?
                    .into_string()
                    .map_err(|_| "--query must be valid UTF-8")?,
            );
        } else if arg.to_str() == Some("--file") {
            if file.is_some() || query.is_some() || script.is_some() {
                return Err("provide SQL text or --file, not both".into());
            }
            file = Some(PathBuf::from(args.next().ok_or("missing SQL file")?));
        } else if arg.to_str() == Some("--script") {
            if script.is_some() || query.is_some() || file.is_some() {
                return Err("provide SQL text or --script, not both".into());
            }
            script = Some(PathBuf::from(args.next().ok_or("missing SQL script file")?));
        } else if query.is_none() && file.is_none() && script.is_none() {
            query = Some(
                arg.into_string()
                    .map_err(|_| "SQL statement must be valid UTF-8")?,
            );
        } else {
            return Err("unexpected SQL argument".into());
        }
    }
    if let Some(path) = script {
        if read_stdin
            || query.is_some()
            || file.is_some()
            || database.is_none()
            || part.is_some()
            || catalog.is_some()
            || format.is_some()
            || output_path.is_some()
            || max_rows.is_some()
            || show_ast
            || explain
            || time
        {
            return Err("--script requires --database and no other SQL options".into());
        }
        let database = database.expect("checked script database path");
        for (index, statement) in read_script(&path)?.iter().enumerate() {
            run_sql_args(
                [
                    OsString::from(statement),
                    OsString::from("--database"),
                    database.as_os_str().to_owned(),
                ]
                .into_iter(),
            )
            .map_err(|error| format!("SQL script statement {} failed: {error}", index + 1))?;
        }
        return Ok(());
    }
    if read_stdin && (query.is_some() || file.is_some()) {
        return Err("provide SQL text, --file, or --stdin, not both".into());
    }
    let mut query = if read_stdin {
        read_sql_input(std::io::stdin(), "SQL from stdin", "SQL from stdin")?
    } else if let Some(file) = file {
        let input = std::fs::File::open(&file)
            .map_err(|error| format!("cannot open {}: {error}", file.display()))?;
        read_sql_input(input, &file.display().to_string(), "SQL file")?
    } else {
        query.ok_or("missing SQL statement")?
    };
    let trimmed = query.trim_start();
    let explain_statement = trimmed
        .get(..7)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("EXPLAIN"))
        && trimmed
            .as_bytes()
            .get(7)
            .is_none_or(u8::is_ascii_whitespace);
    if explain_statement {
        if explain {
            return Err("do not combine EXPLAIN statement with --explain".into());
        }
        query = trimmed[7..].trim_start().to_owned();
        if query.is_empty() {
            return Err("EXPLAIN requires a SELECT statement".into());
        }
        explain = true;
    }
    // ClickHouse clients may select a supported text encoding directly in SQL.
    // Only inspect SELECT statements; large INSERT payloads have a separate lexer limit.
    let starts_with_select = scalar_subquery::starts_with_select(&query);
    if starts_with_select {
        let (statement, inline_format) =
            split_clickhouse_format(&query).map_err(|error| error.to_string())?;
        if let Some(name) = inline_format {
            if explain {
                return Err("FORMAT is not supported with EXPLAIN".into());
            }
            if format.is_some() {
                return Err("cannot combine SQL FORMAT with --format".into());
            }
            format = Some(SqlOutputFormat::from_arg(Some(OsString::from(name)))?);
            query = statement.to_owned();
        }
    }
    if database.is_some() && catalog.is_some() {
        return Err("--catalog and --database cannot be combined".into());
    }
    if let Some(database_dir) = database.as_deref() {
        if part.is_some() {
            return Err("--part cannot be combined with --database".into());
        }
        if let Some(ddl) = parse_ddl(&query).map_err(|error| error.to_string())? {
            if explain || time || format.is_some() || output_path.is_some() || max_rows.is_some() {
                return Err(
                    "database DDL cannot use --explain, --time, --format, --output, or --max-rows"
                        .into(),
                );
            }
            if show_ast {
                println!("{ddl:#?}");
                return Ok(());
            }
            return crate::database::apply_ddl(database_dir, &ddl);
        }
        if let Some(copy) =
            rusthouse_query::parse_copy_from(&query).map_err(|error| error.to_string())?
        {
            if explain || time || format.is_some() || output_path.is_some() || max_rows.is_some() {
                return Err(
                    "COPY cannot use --explain, --time, --format, --output, or --max-rows".into(),
                );
            }
            if show_ast {
                println!("{copy:#?}");
                return Ok(());
            }
            return crate::copy_from::run(database_dir, &copy);
        }
        if let Some(insert) =
            rusthouse_query::parse_insert_select(&query).map_err(|error| error.to_string())?
        {
            if explain || time || format.is_some() || output_path.is_some() || max_rows.is_some() {
                return Err(
                    "INSERT cannot use --explain, --time, --format, --output, or --max-rows".into(),
                );
            }
            if show_ast {
                println!("{insert:#?}");
                return Ok(());
            }
            return crate::insert_select::run(database_dir, &insert);
        }
        if let Some(insert) = parse_insert(&query).map_err(|error| error.to_string())? {
            if explain || time || format.is_some() || output_path.is_some() || max_rows.is_some() {
                return Err(
                    "INSERT cannot use --explain, --time, --format, --output, or --max-rows".into(),
                );
            }
            if show_ast {
                println!("{insert:#?}");
                return Ok(());
            }
            return crate::database::insert(database_dir, &insert);
        }
        if let Some(delete) = parse_delete(&query).map_err(|error| error.to_string())? {
            if explain || time || format.is_some() || output_path.is_some() || max_rows.is_some() {
                return Err("DELETE cannot use output or SELECT-only flags".into());
            }
            if show_ast {
                println!("{delete:#?}");
                return Ok(());
            }
            return crate::database::delete(database_dir, &delete);
        }
        if let Some(update) = parse_update(&query).map_err(|error| error.to_string())? {
            if explain || time || format.is_some() || output_path.is_some() || max_rows.is_some() {
                return Err("UPDATE cannot use output or SELECT-only flags".into());
            }
            if show_ast {
                println!("{update:#?}");
                return Ok(());
            }
            return crate::database::update(database_dir, &update);
        }
    }
    if let Some(catalog_path) = catalog {
        if explain {
            return Err("--explain is only supported for SELECT".into());
        }
        if time {
            return Err("--time cannot be combined with --catalog".into());
        }
        if let Some(ddl) = parse_ddl(&query).map_err(|error| error.to_string())? {
            if part.is_some() || output_path.is_some() || format.is_some() || max_rows.is_some() {
                return Err("DDL cannot use --part, --output, --format, or --max-rows".into());
            }
            if show_ast {
                println!("{ddl:#?}");
                return Ok(());
            }
            // Catalog-only schema changes also need cross-process serialization.
            // Otherwise simultaneous ALTER/RENAME operations can lose updates.
            let read_only = matches!(
                &ddl,
                rusthouse_query::Ddl::ShowTables
                    | rusthouse_query::Ddl::ShowTablesLike { .. }
                    | rusthouse_query::Ddl::ShowTablesNotLike { .. }
                    | rusthouse_query::Ddl::ShowCreateTable { .. }
                    | rusthouse_query::Ddl::DescribeTable { .. }
            );
            let _catalog_lock = if read_only && !catalog_path.is_dir() {
                None
            } else {
                Some(rusthouse_storage::acquire_database_lock(
                    &catalog_path,
                    !read_only,
                )?)
            };
            return crate::catalog::apply(&catalog_path, &ddl);
        }
        return Err("--catalog is only supported for DDL statements".into());
    }
    let mut _scalar_subquery_read_lock = None;
    if starts_with_select && !explain && !show_ast {
        let scalar_candidate = scalar_subquery::has_candidate(&query);
        if let Some(directory) = database.as_deref()
            && scalar_candidate
        {
            _scalar_subquery_read_lock = Some(crate::database::read_lock(directory)?);
            if let Some(rewritten) = scalar_subquery::rewrite(&query, directory)? {
                query = rewritten;
            }
        }
    }
    let scalar_derived = scalar_derived_cross::parse(&query)?;
    let mut scalar_derived_part = None;
    let mut scalar_query_started_at = None;
    let mut _scalar_read_lock = None;
    if let Some(plan) = scalar_derived.as_ref() {
        if explain || show_ast {
            return Err(
                "scalar derived aggregate queries cannot use --explain or --show-ast yet".into(),
            );
        }
        let directory = database
            .as_deref()
            .ok_or("scalar derived aggregate queries require --database")?;
        _scalar_read_lock = Some(crate::database::read_lock(directory)?);
        scalar_query_started_at = Some(std::time::Instant::now());
        scalar_derived_part = Some(scalar_derived_cross::materialize(directory, plan)?);
        query = "SELECT * FROM part".into();
    }
    if starts_with_select {
        let derived_set_count = if let Some(directory) = database.as_deref() {
            rusthouse_query::parse_derived_set_count_with_schema(&query, |table| {
                crate::catalog::read_schema(directory, table)
                    .map(|columns| {
                        columns
                            .into_iter()
                            .map(|column| rusthouse_query::DerivedColumn {
                                name: column.name,
                                data_type: column.data_type,
                            })
                            .collect()
                    })
                    .map_err(ParseError)
            })
            .map_err(|error| error.to_string())?
        } else {
            None
        };
        if let Some(plan) = derived_set_count {
            let directory = database
                .as_deref()
                .expect("schema resolver requires a database");
            if part.is_some() || explain || max_rows.is_some() {
                return Err(
                    "derived set count does not support --part, --explain, or --max-rows".into(),
                );
            }
            if !matches!(format.unwrap_or(SqlOutputFormat::Tsv), SqlOutputFormat::Tsv) {
                return Err("derived set count currently supports TSV output only".into());
            }
            if show_ast {
                if output_path.is_some() || time {
                    return Err("--show-ast cannot use --output or --time".into());
                }
                println!("{plan:#?}");
                return Ok(());
            }
            let _read_lock = crate::database::read_lock(directory)?;
            let paths = plan
                .branches()
                .iter()
                .map(|branch| {
                    [&branch.left, &branch.right]
                        .into_iter()
                        .chain(branch.steps.iter().map(|step| &step.table))
                        .map(|table| crate::database::select_source(directory, table))
                        .collect::<Result<Vec<_>, _>>()
                })
                .collect::<Result<Vec<_>, String>>()?;
            let mut output: Box<dyn std::io::Write> = match &output_path {
                Some(path) => Box::new(
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(path)
                        .map_err(|error| format!("cannot create {}: {error}", path.display()))?,
                ),
                None => Box::new(std::io::stdout()),
            };
            let start = std::time::Instant::now();
            let result = rusthouse_query::execute_derived_set_count(&plan, &paths, &mut output);
            if time {
                eprintln!("Elapsed: {:.3} ms", start.elapsed().as_secs_f64() * 1_000.0);
            }
            if result.is_err() {
                drop(output);
                if let Some(path) = output_path {
                    let _ = std::fs::remove_file(path);
                }
            }
            return result.map_err(|error| error.to_string());
        }
        if let Some(plan) =
            rusthouse_query::parse_bounded_except(&query).map_err(|error| error.to_string())?
        {
            let directory = database.as_deref().ok_or("EXCEPT requires --database")?;
            if part.is_some() || explain || max_rows.is_some() {
                return Err("EXCEPT does not support --part, --explain, or --max-rows".into());
            }
            if !matches!(format.unwrap_or(SqlOutputFormat::Tsv), SqlOutputFormat::Tsv) {
                return Err("bounded EXCEPT currently supports TSV output only".into());
            }
            if show_ast {
                if output_path.is_some() || time {
                    return Err("--show-ast cannot use --output or --time".into());
                }
                println!("{plan:#?}");
                return Ok(());
            }
            let _read_lock = crate::database::read_lock(directory)?;
            let paths = plan
                .selects
                .iter()
                .map(|select| match &select.source {
                    Some(Source::Table(table)) => crate::database::select_source(directory, table),
                    _ => Err("EXCEPT requires table sources".into()),
                })
                .collect::<Result<Vec<_>, String>>()?;
            let mut output: Box<dyn std::io::Write> = match &output_path {
                Some(path) => Box::new(
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(path)
                        .map_err(|error| format!("cannot create {}: {error}", path.display()))?,
                ),
                None => Box::new(std::io::stdout()),
            };
            let start = std::time::Instant::now();
            let result = rusthouse_query::execute_bounded_except(&plan, &paths, &mut output);
            if time {
                eprintln!("Elapsed: {:.3} ms", start.elapsed().as_secs_f64() * 1_000.0);
            }
            if result.is_err() {
                drop(output);
                if let Some(path) = output_path {
                    let _ = std::fs::remove_file(path);
                }
            }
            return result.map_err(|error| error.to_string());
        }
    }
    let implicit = has_implicit_join(&query).map_err(|error| error.to_string())?;
    let cte = has_cte_max_join(&query).map_err(|error| error.to_string())?;
    let derived = has_derived_join_count(&query).map_err(|error| error.to_string())?;
    let derived_union =
        rusthouse_query::has_derived_union(&query).map_err(|error| error.to_string())?;
    let substring_antijoin =
        rusthouse_query::has_substring_antijoin(&query).map_err(|error| error.to_string())?;
    let possible_join_exists = has_top_level_exists(&query).map_err(|error| error.to_string())?
        && (implicit || has_explicit_join(&query).map_err(|error| error.to_string())?);
    let possible_join_avg = has_join_correlated_avg(&query).map_err(|error| error.to_string())?
        && (implicit || has_explicit_join(&query).map_err(|error| error.to_string())?);
    let possible_join_min = has_join_correlated_min(&query).map_err(|error| error.to_string())?
        && (implicit || has_explicit_join(&query).map_err(|error| error.to_string())?);
    let possible_join_sum = has_join_correlated_sum(&query).map_err(|error| error.to_string())?
        && (implicit || has_explicit_join(&query).map_err(|error| error.to_string())?);
    let possible_nested_sum = has_nested_join_sum(&query).map_err(|error| error.to_string())?
        && (implicit || has_explicit_join(&query).map_err(|error| error.to_string())?);
    let implicit_lock = if implicit
        || cte
        || derived
        || derived_union
        || substring_antijoin
        || possible_join_exists
        || possible_join_avg
        || possible_join_min
        || possible_join_sum
        || possible_nested_sum
    {
        match database.as_deref() {
            Some(path) if path.is_dir() => Some(crate::database::read_lock(path)?),
            _ => None,
        }
    } else {
        None
    };
    if cte {
        if let Some(directory) = database.as_deref() {
            let correlated_avg =
                rusthouse_query::parse_cte_correlated_avg_with_schema(&query, |table| {
                    crate::catalog::read_schema(directory, table)
                        .map(|columns| {
                            columns
                                .into_iter()
                                .map(|column| rusthouse_query::CteOutputColumn {
                                    name: column.name,
                                    data_type: column.data_type,
                                })
                                .collect()
                        })
                        .map_err(ParseError)
                })
                .map_err(|error| error.to_string())?;
            if let Some(mut plan) = correlated_avg {
                if part.is_some() {
                    return Err("CTE correlated AVG cannot use --part".into());
                }
                if show_ast || explain {
                    return Err("CTE correlated AVG cannot use --show-ast or --explain yet".into());
                }
                if let Some(cap) = max_rows {
                    plan.cap_rows(cap);
                }
                return crate::join::run_cte_correlated_avg(
                    directory,
                    &plan,
                    format.unwrap_or(SqlOutputFormat::Tsv),
                    output_path,
                    time,
                    implicit_lock,
                );
            }
            let cte_join = rusthouse_query::parse_cte_multi_join_with_schema(&query, |table| {
                crate::catalog::read_schema(directory, table)
                    .map(|columns| {
                        columns
                            .into_iter()
                            .map(|column| rusthouse_query::CteOutputColumn {
                                name: column.name,
                                data_type: column.data_type,
                            })
                            .collect()
                    })
                    .map_err(ParseError)
            })
            .map_err(|error| error.to_string())?;
            if let Some(mut plan) = cte_join {
                if part.is_some() {
                    return Err("CTE JOIN cannot use --part".into());
                }
                if show_ast || explain {
                    return Err("CTE JOIN cannot use --show-ast or --explain yet".into());
                }
                if let Some(cap) = max_rows {
                    plan.cap_rows(cap);
                }
                return crate::join::run_cte_multi_join(
                    directory,
                    &plan,
                    format.unwrap_or(SqlOutputFormat::Tsv),
                    output_path,
                    time,
                    implicit_lock,
                );
            }
            let cte_union = rusthouse_query::parse_cte_union_select_with_schema(&query, |table| {
                crate::catalog::read_schema(directory, table)
                    .map(|columns| {
                        columns
                            .into_iter()
                            .map(|column| rusthouse_query::DerivedColumn {
                                name: column.name,
                                data_type: column.data_type,
                            })
                            .collect()
                    })
                    .map_err(ParseError)
            })
            .map_err(|error| error.to_string())?;
            if let Some(mut plan) = cte_union {
                if part.is_some() {
                    return Err("CTE UNION cannot use --part".into());
                }
                if show_ast || explain {
                    return Err("CTE UNION cannot use --show-ast or --explain yet".into());
                }
                if let Some(cap) = max_rows {
                    plan.cap_rows(cap);
                }
                return crate::join::run_derived_union_select(
                    directory,
                    &plan,
                    format.unwrap_or(SqlOutputFormat::Tsv),
                    output_path,
                    time,
                    implicit_lock,
                );
            }
            let plan = parse_cte_max_join(&query).map_err(|error| error.to_string())?;
            if let Some(mut plan) = plan {
                if part.is_some() {
                    return Err("CTE JOIN cannot use --part".into());
                }
                if show_ast || explain {
                    return Err("CTE JOIN cannot use --show-ast or --explain yet".into());
                }
                if let Some(cap) = max_rows {
                    plan.cap_rows(cap);
                }
                return crate::join::run_cte_max(
                    directory,
                    &plan,
                    format.unwrap_or(SqlOutputFormat::Tsv),
                    output_path,
                    time,
                    implicit_lock,
                );
            }
        }
    }
    if substring_antijoin {
        if let Some(directory) = database.as_deref() {
            let plan = rusthouse_query::parse_substring_antijoin_with_schema(&query, |table| {
                crate::catalog::read_schema(directory, table)
                    .map(|columns| columns.into_iter().map(|column| column.name).collect())
                    .map_err(ParseError)
            })
            .map_err(|error| error.to_string())?;
            if let Some(mut plan) = plan {
                if part.is_some() {
                    return Err("substring anti-join cannot use --part".into());
                }
                if show_ast || explain {
                    return Err("substring anti-join cannot use --show-ast or --explain yet".into());
                }
                if let Some(cap) = max_rows {
                    plan.cap_rows(cap);
                }
                return crate::join::run_substring_antijoin(
                    directory,
                    &plan,
                    format.unwrap_or(SqlOutputFormat::Tsv),
                    output_path,
                    time,
                    implicit_lock,
                );
            }
        }
    }
    if derived || derived_union {
        if let Some(directory) = database.as_deref() {
            if derived_union {
                let select =
                    rusthouse_query::parse_derived_union_select_with_schema(&query, |table| {
                        crate::catalog::read_schema(directory, table)
                            .map(|columns| {
                                columns
                                    .into_iter()
                                    .map(|column| rusthouse_query::DerivedColumn {
                                        name: column.name,
                                        data_type: column.data_type,
                                    })
                                    .collect()
                            })
                            .map_err(ParseError)
                    })
                    .map_err(|error| error.to_string())?;
                if let Some(mut plan) = select {
                    if part.is_some() {
                        return Err("derived UNION SELECT cannot use --part".into());
                    }
                    if show_ast || explain {
                        return Err(
                            "derived UNION SELECT cannot use --show-ast or --explain yet".into(),
                        );
                    }
                    if let Some(cap) = max_rows {
                        plan.cap_rows(cap);
                    }
                    return crate::join::run_derived_union_select(
                        directory,
                        &plan,
                        format.unwrap_or(SqlOutputFormat::Tsv),
                        output_path,
                        time,
                        implicit_lock,
                    );
                }
                let plan = rusthouse_query::parse_derived_union_join_with_schema(&query, |table| {
                    crate::catalog::read_schema(directory, table)
                        .map(|columns| {
                            columns
                                .into_iter()
                                .map(|column| rusthouse_query::DerivedColumn {
                                    name: column.name,
                                    data_type: column.data_type,
                                })
                                .collect()
                        })
                        .map_err(ParseError)
                })
                .map_err(|error| error.to_string())?;
                if let Some(mut plan) = plan {
                    if part.is_some() {
                        return Err("derived UNION JOIN cannot use --part".into());
                    }
                    if show_ast || explain {
                        return Err(
                            "derived UNION JOIN cannot use --show-ast or --explain yet".into()
                        );
                    }
                    if let Some(cap) = max_rows {
                        plan.cap_rows(cap);
                    }
                    return crate::join::run_derived_union(
                        directory,
                        &plan,
                        format.unwrap_or(SqlOutputFormat::Tsv),
                        output_path,
                        time,
                        implicit_lock,
                    );
                }
            }
            let sum = parse_derived_join_sum_with_schema(&query, |table| {
                crate::catalog::read_schema(directory, table)
                    .map(|columns| columns.into_iter().map(|column| column.name).collect())
                    .map_err(ParseError)
            })
            .map_err(|error| error.to_string())?;
            if let Some(mut join) = sum {
                if part.is_some() {
                    return Err("derived JOIN cannot use --part".into());
                }
                if show_ast || explain {
                    return Err("derived JOIN cannot use --show-ast or --explain yet".into());
                }
                if let Some(cap) = max_rows {
                    join.limit = Some(join.limit.map_or(cap, |limit| limit.min(cap)));
                }
                return crate::join::run(
                    directory,
                    &join,
                    format.unwrap_or(SqlOutputFormat::Tsv),
                    output_path,
                    time,
                    implicit_lock,
                );
            }
            let plan = parse_derived_join_count_with_schema(&query, |table| {
                crate::catalog::read_schema(directory, table)
                    .map(|columns| columns.into_iter().map(|column| column.name).collect())
                    .map_err(ParseError)
            })
            .map_err(|error| error.to_string())?;
            if let Some(mut plan) = plan {
                if part.is_some() {
                    return Err("derived JOIN cannot use --part".into());
                }
                if show_ast || explain {
                    return Err("derived JOIN cannot use --show-ast or --explain yet".into());
                }
                if let Some(cap) = max_rows {
                    plan.cap_rows(cap);
                }
                return crate::join::run_derived_count(
                    directory,
                    &plan,
                    format.unwrap_or(SqlOutputFormat::Tsv),
                    output_path,
                    time,
                    implicit_lock,
                );
            }
            let plan = rusthouse_query::parse_derived_grouped_join_with_schema(&query, |table| {
                crate::catalog::read_schema(directory, table)
                    .map(|columns| {
                        columns
                            .into_iter()
                            .map(|column| rusthouse_query::DerivedColumn {
                                name: column.name,
                                data_type: column.data_type,
                            })
                            .collect()
                    })
                    .map_err(ParseError)
            })
            .map_err(|error| error.to_string())?;
            if let Some(mut plan) = plan {
                if part.is_some() {
                    return Err("derived JOIN cannot use --part".into());
                }
                if show_ast || explain {
                    return Err("derived JOIN cannot use --show-ast or --explain yet".into());
                }
                if let Some(cap) = max_rows {
                    plan.cap_rows(cap);
                }
                return crate::join::run_derived_grouped(
                    directory,
                    &plan,
                    format.unwrap_or(SqlOutputFormat::Tsv),
                    output_path,
                    time,
                    implicit_lock,
                );
            }
            let plan = rusthouse_query::parse_derived_grouped_select_with_schema(&query, |table| {
                crate::catalog::read_schema(directory, table)
                    .map(|columns| {
                        columns
                            .into_iter()
                            .map(|column| rusthouse_query::DerivedColumn {
                                name: column.name,
                                data_type: column.data_type,
                            })
                            .collect()
                    })
                    .map_err(ParseError)
            })
            .map_err(|error| error.to_string())?;
            if let Some(mut plan) = plan {
                if part.is_some() {
                    return Err("derived grouped SELECT cannot use --part".into());
                }
                if show_ast || explain {
                    return Err(
                        "derived grouped SELECT cannot use --show-ast or --explain yet".into(),
                    );
                }
                if let Some(cap) = max_rows {
                    plan.cap_rows(cap);
                }
                return crate::join::run_derived_grouped_select(
                    directory,
                    &plan,
                    format.unwrap_or(SqlOutputFormat::Tsv),
                    output_path,
                    time,
                    implicit_lock,
                );
            }
        }
    }
    if possible_join_min {
        if let Some(directory) = database.as_deref() {
            let plan = parse_join_correlated_min_with_schema(&query, |table| {
                crate::catalog::read_schema(directory, table)
                    .map(|columns| columns.into_iter().map(|column| column.name).collect())
                    .map_err(ParseError)
            })
            .map_err(|error| error.to_string())?;
            if let Some(mut plan) = plan {
                if part.is_some() {
                    return Err("JOIN cannot use --part".into());
                }
                if show_ast || explain {
                    return Err("correlated JOIN MIN cannot use --show-ast or --explain yet".into());
                }
                if let Some(cap) = max_rows {
                    plan.cap_rows(cap);
                }
                return crate::join::run_correlated_min(
                    directory,
                    &plan,
                    format.unwrap_or(SqlOutputFormat::Tsv),
                    output_path,
                    time,
                    implicit_lock,
                );
            }
        }
    }
    if possible_nested_sum {
        if let Some(directory) = database.as_deref() {
            let plan = parse_nested_join_sum_with_schema(&query, |table| {
                crate::catalog::read_schema(directory, table)
                    .map(|columns| columns.into_iter().map(|column| column.name).collect())
                    .map_err(ParseError)
            })
            .map_err(|error| error.to_string())?;
            if let Some(mut plan) = plan {
                if part.is_some() {
                    return Err("JOIN cannot use --part".into());
                }
                if show_ast || explain {
                    return Err("nested JOIN SUM cannot use --show-ast or --explain yet".into());
                }
                if let Some(cap) = max_rows {
                    plan.cap_rows(cap);
                }
                return crate::join::run_nested_sum(
                    directory,
                    &plan,
                    format.unwrap_or(SqlOutputFormat::Tsv),
                    output_path,
                    time,
                    implicit_lock,
                );
            }
        }
    }
    if possible_join_sum {
        if let Some(directory) = database.as_deref() {
            let plan = parse_join_correlated_sum_with_schema(&query, |table| {
                crate::catalog::read_schema(directory, table)
                    .map(|columns| columns.into_iter().map(|column| column.name).collect())
                    .map_err(ParseError)
            })
            .map_err(|error| error.to_string())?;
            if let Some(mut plan) = plan {
                if part.is_some() {
                    return Err("JOIN cannot use --part".into());
                }
                if show_ast || explain {
                    return Err("correlated JOIN SUM cannot use --show-ast or --explain yet".into());
                }
                if let Some(cap) = max_rows {
                    plan.cap_rows(cap);
                }
                return crate::join::run_correlated_sum(
                    directory,
                    &plan,
                    format.unwrap_or(SqlOutputFormat::Tsv),
                    output_path,
                    time,
                    implicit_lock,
                );
            }
        }
    }
    if possible_join_avg {
        if let Some(directory) = database.as_deref() {
            let plan = parse_join_correlated_avg_with_schema(&query, |table| {
                crate::catalog::read_schema(directory, table)
                    .map(|columns| columns.into_iter().map(|column| column.name).collect())
                    .map_err(ParseError)
            })
            .map_err(|error| error.to_string())?;
            if let Some(mut plan) = plan {
                if part.is_some() {
                    return Err("JOIN cannot use --part".into());
                }
                if show_ast || explain {
                    return Err("correlated JOIN AVG cannot use --show-ast or --explain yet".into());
                }
                if let Some(cap) = max_rows {
                    plan.cap_rows(cap);
                }
                return crate::join::run_correlated_avg(
                    directory,
                    &plan,
                    format.unwrap_or(SqlOutputFormat::Tsv),
                    output_path,
                    time,
                    implicit_lock,
                );
            }
        }
    }
    if possible_join_exists {
        if let Some(directory) = database.as_deref() {
            let plan = parse_join_correlated_exists_with_schema(&query, |table| {
                crate::catalog::read_schema(directory, table)
                    .map(|columns| columns.into_iter().map(|column| column.name).collect())
                    .map_err(ParseError)
            })
            .map_err(|error| error.to_string())?;
            if let Some(mut plan) = plan {
                if part.is_some() {
                    return Err("JOIN cannot use --part".into());
                }
                if show_ast || explain {
                    return Err(
                        "correlated JOIN EXISTS cannot use --show-ast or --explain yet".into(),
                    );
                }
                if let Some(cap) = max_rows {
                    plan.cap_rows(cap);
                }
                return crate::join::run_correlated(
                    directory,
                    &plan,
                    format.unwrap_or(SqlOutputFormat::Tsv),
                    output_path,
                    time,
                    implicit_lock,
                );
            }
        }
    }
    let parsed_join = if implicit {
        if let Some(directory) = database.as_deref() {
            parse_join_with_schema(&query, |table| {
                crate::catalog::read_schema(directory, table)
                    .map(|columns| columns.into_iter().map(|column| column.name).collect())
                    .map_err(ParseError)
            })
        } else {
            parse_join(&query)
        }
    } else {
        parse_join(&query)
    }
    .map_err(|error| error.to_string())?;
    if let Some(mut join) = parsed_join {
        let database_dir = database.as_deref().ok_or("JOIN requires --database")?;
        if part.is_some() {
            return Err("JOIN cannot use --part".into());
        }
        if explain {
            if show_ast || time || format.is_some() || output_path.is_some() {
                return Err("--explain cannot be combined with other output options".into());
            }
            if let Some(cap) = max_rows {
                join.limit = Some(join.limit.map_or(cap, |limit| limit.min(cap)));
            }
            println!("Logical plan (no data read):\n{}", explain_join(&join));
            return Ok(());
        }
        if show_ast {
            if time || output_path.is_some() {
                return Err("--show-ast cannot be combined with --time or --output".into());
            }
            println!("{join:#?}");
            return Ok(());
        }
        if let Some(cap) = max_rows {
            join.limit = Some(join.limit.map_or(cap, |limit| limit.min(cap)));
        }
        return crate::join::run(
            database_dir,
            &join,
            format.unwrap_or(SqlOutputFormat::Tsv),
            output_path,
            time,
            implicit_lock,
        );
    }
    let correlated = parse_correlated_exists(&query).map_err(|error| error.to_string())?;
    if correlated.is_some() && (show_ast || explain) {
        return Err("correlated EXISTS cannot use --show-ast or --explain yet".into());
    }
    let mut select = if let Some(ref correlated) = correlated {
        correlated.outer.clone()
    } else {
        parse_select(&query).map_err(|error| error.to_string())?
    };
    if let Some((source, target)) = scalar_derived_part
        .as_ref()
        .and_then(|temporary| temporary.output_rename.clone())
    {
        select.rename = Some((source, target));
    }
    if explain {
        if show_ast || time || format.is_some() || output_path.is_some() {
            return Err(
                "--explain cannot be combined with --show-ast, --time, --format, or --output"
                    .into(),
            );
        }
        if let Some(cap) = max_rows {
            select.limit = Some(select.limit.map_or(cap, |limit| limit.min(cap)));
        }
        println!("Logical plan (no data read):\n{}", explain_select(&select));
        return Ok(());
    }
    if show_ast {
        if time {
            return Err("--time cannot be combined with --show-ast".into());
        }
        if output_path.is_some() {
            return Err("--output cannot be combined with --show-ast".into());
        }
        println!("{select:#?}");
        return Ok(());
    }
    if let Some(cap) = max_rows {
        select.limit = Some(select.limit.map_or(cap, |limit| limit.min(cap)));
    }
    // Hold a shared lock for the entire query, not just snapshot discovery.
    // DROP and RENAME must not remove part directories beneath active readers.
    if let Some(plan) = scalar_derived.as_ref() {
        select.limit = plan.limit;
        select.offset = plan.offset;
    }
    let _database_read_lock = if scalar_derived_part.is_none() {
        match (database.as_deref(), select.source.as_ref()) {
            (Some(directory), Some(Source::Table(_))) if directory.is_dir() => {
                Some(crate::database::read_lock(directory)?)
            }
            _ => None,
        }
    } else {
        None
    };
    if let Some(correlated) = correlated {
        let database_dir = database
            .as_deref()
            .ok_or("correlated EXISTS requires --database")?;
        if part.is_some() {
            return Err("correlated EXISTS cannot use --part".into());
        }
        select = exists::bind(database_dir, correlated)?;
        if let Some(cap) = max_rows {
            select.limit = Some(select.limit.map_or(cap, |limit| limit.min(cap)));
        }
    }
    if scalar_derived_part.is_some() {
        part = scalar_derived_part
            .as_ref()
            .map(|temporary| temporary.path().to_owned());
    } else if let (Some(database_dir), Some(Source::Table(table))) =
        (database.as_deref(), select.source.as_ref())
    {
        part = Some(crate::database::select_source(database_dir, table)?);
        select.source = Some(Source::Table("part".into()));
    }
    let mut output: Box<dyn std::io::Write> = match &output_path {
        Some(path) => Box::new(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|error| format!("cannot create {}: {error}", path.display()))?,
        ),
        None => Box::new(std::io::stdout()),
    };
    let start = scalar_query_started_at.unwrap_or_else(std::time::Instant::now);
    let result = match format.unwrap_or(SqlOutputFormat::Tsv) {
        SqlOutputFormat::Tsv => execute(&select, part.as_deref(), &mut output),
        SqlOutputFormat::TsvNoHeader => execute(
            &select,
            part.as_deref(),
            &mut csv_no_header::WithoutHeader::new(&mut output),
        ),
        SqlOutputFormat::Csv => execute(
            &select,
            part.as_deref(),
            &mut csv::CsvWriter::new(&mut output),
        ),
        SqlOutputFormat::CsvNoHeader => execute(
            &select,
            part.as_deref(),
            &mut csv::CsvWriter::new(csv_no_header::WithoutHeader::new(&mut output)),
        ),
        SqlOutputFormat::Markdown => execute(
            &select,
            part.as_deref(),
            &mut markdown::MarkdownWriter::new(&mut output),
        ),
        SqlOutputFormat::Html => {
            let mut writer = html::HtmlWriter::new(&mut output);
            execute(&select, part.as_deref(), &mut writer).and_then(|()| {
                writer
                    .finish()
                    .map(|_| ())
                    .map_err(rusthouse_query::ExecutionError::Io)
            })
        }
        SqlOutputFormat::Vertical => execute(
            &select,
            part.as_deref(),
            &mut vertical::VerticalWriter::new(&mut output),
        ),
        SqlOutputFormat::SqlValues => {
            let mut writer = sql_values::SqlValuesWriter::new(&mut output);
            execute(&select, part.as_deref(), &mut writer)
                .and_then(|()| writer.finish().map_err(rusthouse_query::ExecutionError::Io))
        }
        SqlOutputFormat::Jsonl => execute(
            &select,
            part.as_deref(),
            &mut jsonl::JsonlWriter::new(&mut output),
        ),
        SqlOutputFormat::JsonSeq => execute(
            &select,
            part.as_deref(),
            &mut json_seq::JsonSeqWriter::new(&mut output),
        ),
        SqlOutputFormat::Json => {
            let mut writer = json_array::JsonArrayWriter::new(&mut output);
            execute(&select, part.as_deref(), &mut writer).and_then(|()| {
                writer
                    .finish()
                    .map(|_| ())
                    .map_err(rusthouse_query::ExecutionError::Io)
            })
        }
        SqlOutputFormat::JsonCompactEachRow => {
            let mut writer = json_compact_each_row::JsonCompactEachRowWriter::new(&mut output);
            execute(&select, part.as_deref(), &mut writer).and_then(|()| {
                writer
                    .finish()
                    .map(|_| ())
                    .map_err(rusthouse_query::ExecutionError::Io)
            })
        }
        SqlOutputFormat::JsonPretty => {
            let mut writer = json_array::JsonArrayWriter::new_pretty(&mut output);
            execute(&select, part.as_deref(), &mut writer).and_then(|()| {
                writer
                    .finish()
                    .map(|_| ())
                    .map_err(rusthouse_query::ExecutionError::Io)
            })
        }
    };
    if time {
        eprintln!("Elapsed: {:.3} ms", start.elapsed().as_secs_f64() * 1_000.0);
    }
    if result.is_err() {
        drop(output);
        if let Some(path) = output_path {
            let _ = std::fs::remove_file(path);
        }
    }
    result.map_err(|error| error.to_string())
}
