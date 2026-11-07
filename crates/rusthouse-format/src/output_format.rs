//! Output format names shared by the CLI and HTTP frontend.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StandardOutputFormat {
    Tsv,
    TsvNoHeader,
    Csv,
    CsvNoHeader,
    Jsonl,
    Json,
    JsonPretty,
    JsonCompactEachRow,
    SqlValues,
    Markdown,
}

impl StandardOutputFormat {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "tsv" | "TabSeparatedWithNames" => Self::Tsv,
            "tsv-no-header" | "TabSeparated" => Self::TsvNoHeader,
            "csv" | "CSVWithNames" => Self::Csv,
            "csv-no-header" | "CSV" => Self::CsvNoHeader,
            "jsonl" | "JSONStringsEachRow" => Self::Jsonl,
            "json" => Self::Json,
            "json-pretty" => Self::JsonPretty,
            "json-compact-each-row" | "JSONCompactStringsEachRow" => Self::JsonCompactEachRow,
            "sql-values" => Self::SqlValues,
            "markdown" => Self::Markdown,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::StandardOutputFormat as Format;

    #[test]
    fn shared_formats_accept_clickhouse_aliases() {
        for (name, kind) in [
            ("TabSeparatedWithNames", Format::Tsv),
            ("TabSeparated", Format::TsvNoHeader),
            ("CSVWithNames", Format::Csv),
            ("CSV", Format::CsvNoHeader),
            ("JSONStringsEachRow", Format::Jsonl),
            ("JSONCompactStringsEachRow", Format::JsonCompactEachRow),
        ] {
            assert_eq!(Format::parse(name), Some(kind));
        }
    }

    #[test]
    fn format_names_remain_case_sensitive() {
        assert_eq!(Format::parse("TSV"), None);
        assert_eq!(Format::parse("json-seq"), None);
        assert_eq!(Format::parse("unknown"), None);
    }
}
