use nucleo_matcher::{
    Config, Matcher,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};
use sqlparser::{
    dialect::PostgreSqlDialect,
    keywords::Keyword,
    tokenizer::{Token, Tokenizer},
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
    if let Some(alias_items) = candidates_for_alias(source, cursor, needle, schema_items) {
        return alias_items;
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

fn candidates_for_alias(
    source: &str,
    cursor: usize,
    needle: &str,
    schema_items: &[String],
) -> Option<Vec<String>> {
    let (qualifier, partial) = needle.rsplit_once('.')?;
    let table = aliases(&source[..cursor])
        .into_iter()
        .find(|(alias, _)| alias.eq_ignore_ascii_case(qualifier))?
        .1;
    let table_prefix = format!("{table}.");
    let mut columns = schema_items
        .iter()
        .filter_map(|item| item.strip_prefix(&table_prefix))
        .filter(|column| !column.contains('.'))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    columns.sort_unstable();
    columns.dedup();

    if partial.is_empty() {
        return Some(
            columns
                .into_iter()
                .take(6)
                .map(|column| format!("{qualifier}.{}", postgres_identifier(&column)))
                .collect(),
        );
    }
    let mut matcher = Matcher::new(Config::DEFAULT);
    Some(
        Pattern::new(
            partial,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        )
        .match_list(columns, &mut matcher)
        .into_iter()
        .take(6)
        .map(|(column, _)| format!("{qualifier}.{}", postgres_identifier(&column)))
        .collect(),
    )
}

fn postgres_identifier(identifier: &str) -> String {
    let safe = identifier
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_lowercase() || character == '_')
        && identifier.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        });
    if safe {
        identifier.into()
    } else {
        format!("\"{}\"", identifier.replace('"', "\"\""))
    }
}

fn aliases(source: &str) -> Vec<(String, String)> {
    let dialect = PostgreSqlDialect {};
    let Ok(tokens) = Tokenizer::new(&dialect, source).tokenize() else {
        return Vec::new();
    };
    let tokens = tokens
        .into_iter()
        .filter(|token| !matches!(token, Token::Whitespace(_)))
        .collect::<Vec<_>>();
    let mut aliases = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let introduces_relation = matches!(&tokens[index], Token::Word(word)
            if matches!(word.value.to_ascii_uppercase().as_str(), "FROM" | "JOIN" | "UPDATE" | "INTO"));
        if !introduces_relation {
            index += 1;
            continue;
        }
        let Some(Token::Word(first_part)) = tokens.get(index + 1) else {
            index += 1;
            continue;
        };
        let mut table = first_part.value.clone();
        let mut next = index + 2;
        if tokens
            .get(next)
            .is_some_and(|token| token.to_string() == ".")
            && let Some(Token::Word(table_part)) = tokens.get(next + 1)
        {
            table = table_part.value.clone();
            next += 2;
        }
        if matches!(tokens.get(next), Some(Token::Word(word)) if word.value.eq_ignore_ascii_case("AS"))
        {
            next += 1;
        }
        if let Some(Token::Word(alias)) = tokens.get(next)
            && alias.keyword == Keyword::NoKeyword
        {
            aliases.push((alias.value.clone(), table));
        }
        index = next;
    }
    aliases
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

    #[test]
    fn resolves_aliases_to_only_their_table_columns() {
        let all = vec![
            "user_roles.userId".into(),
            "user_roles.roleId".into(),
            "users.id".into(),
            "refresh_tokens.userId".into(),
        ];
        let sql = "SELECT * FROM user_roles ur JOIN users u ON ur.user";
        let items = candidates(sql, sql.len(), &all, &[]);
        assert_eq!(items, vec!["ur.\"userId\""]);
    }

    #[test]
    fn resolves_each_join_alias_independently() {
        let all = vec![
            "user_roles.userId".into(),
            "users.id".into(),
            "users.email".into(),
        ];
        let sql = "SELECT * FROM user_roles ur JOIN users u ON u.";
        let items = candidates(sql, sql.len(), &all, &[]);
        assert_eq!(items, vec!["u.email", "u.id"]);
    }

    #[test]
    fn quotes_identifiers_that_postgres_would_case_fold() {
        assert_eq!(postgres_identifier("created_at"), "created_at");
        assert_eq!(postgres_identifier("createdAt"), "\"createdAt\"");
        assert_eq!(postgres_identifier("order-item"), "\"order-item\"");
    }
}
