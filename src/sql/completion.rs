use nucleo_matcher::{
    Config, Matcher,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};

const KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "JOIN",
    "LEFT JOIN",
    "RIGHT JOIN",
    "INNER JOIN",
    "ON",
    "GROUP BY",
    "ORDER BY",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "INSERT INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE FROM",
    "RETURNING",
    "WITH",
    "AS",
    "DISTINCT",
    "UNION",
    "UNION ALL",
    "CREATE TABLE",
    "ALTER TABLE",
    "DROP TABLE",
    "BEGIN",
    "COMMIT",
    "ROLLBACK",
    "EXPLAIN",
    "ANALYZE",
    "NULL",
    "TRUE",
    "FALSE",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
];

pub fn prefix(source: &str, cursor: usize) -> &str {
    let before = &source[..cursor];
    let start = before
        .char_indices()
        .rev()
        .find(|(_, character)| {
            !character.is_alphanumeric() && *character != '_' && *character != '.'
        })
        .map_or(0, |(index, character)| index + character.len_utf8());
    &before[start..]
}

pub fn candidates(
    source: &str,
    cursor: usize,
    schema_items: &[String],
    relation_items: &[String],
) -> Vec<String> {
    let needle = prefix(source, cursor);
    if needle.chars().count() < 2 {
        return Vec::new();
    }
    let mut matcher = Matcher::new(Config::DEFAULT);
    let before_prefix = &source[..cursor - needle.len()];
    let expects_relation = before_prefix
        .split_whitespace()
        .next_back()
        .is_some_and(|word| {
            matches!(
                word.to_ascii_uppercase().as_str(),
                "FROM" | "JOIN" | "UPDATE" | "INTO" | "TABLE"
            )
        });
    let mut sources = KEYWORDS
        .iter()
        .map(|item| (*item).to_owned())
        .collect::<Vec<_>>();
    sources.extend(if expects_relation {
        relation_items.iter().cloned()
    } else {
        schema_items.iter().cloned()
    });
    Pattern::new(
        needle,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    )
    .match_list(sources, &mut matcher)
    .into_iter()
    .take(6)
    .map(|(candidate, _)| candidate)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_fuzzy_sql_keywords() {
        let items = candidates("SEL", 3, &[], &[]);
        assert_eq!(items.first().map(String::as_str), Some("SELECT"));
    }

    #[test]
    fn relation_context_excludes_columns() {
        let all = vec!["userId".into(), "users".into()];
        let relations = vec!["users".into()];
        let items = candidates("SELECT * FROM use", 17, &all, &relations);
        assert_eq!(items.first().map(String::as_str), Some("users"));
        assert!(!items.iter().any(|item| item == "userId"));
    }
}
