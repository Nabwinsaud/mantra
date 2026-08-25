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
    forced: bool,
) -> Vec<String> {
    let needle = prefix(source, cursor);
    if !forced && needle.chars().count() < 2 {
        return Vec::new();
    }

    // Read the whole current statement, not only the text before the cursor. This matters while
    // editing a SELECT list: its FROM clause normally appears later in the SQL.
    let statement = crate::sql::statement::current(source, cursor);
    let relations = referenced_relations(statement);
    if let Some(qualified_items) = candidates_for_qualifier(needle, schema_items, &relations) {
        return qualified_items;
    }

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
    let sources = if expects_relation {
        relation_items.to_vec()
    } else if relations.is_empty() {
        // A global list of every database column is misleading. Until a relation is present in
        // the statement, offer syntax only.
        KEYWORDS.iter().map(|item| (*item).to_owned()).collect()
    } else {
        columns_for_relations(schema_items, &relations)
    };

    if needle.is_empty() {
        return sources.into_iter().take(8).collect();
    }

    let mut matcher = Matcher::new(Config::DEFAULT);
    Pattern::new(
        needle,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    )
    .match_list(sources, &mut matcher)
    .into_iter()
    .take(8)
    .map(|(candidate, _)| candidate)
    .collect()
}

fn candidates_for_qualifier(
    needle: &str,
    schema_items: &[String],
    relations: &[RelationReference],
) -> Option<Vec<String>> {
    let (qualifier, partial) = needle.rsplit_once('.')?;
    let relation = relations.iter().find(|relation| {
        relation
            .alias
            .as_deref()
            .is_some_and(|alias| alias.eq_ignore_ascii_case(qualifier))
            || relation.table.eq_ignore_ascii_case(qualifier)
            || relation.qualified_name().eq_ignore_ascii_case(qualifier)
    })?;
    let columns = columns_for_relation(schema_items, relation);

    if partial.is_empty() {
        return Some(
            columns
                .into_iter()
                .take(8)
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
        .take(8)
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct RelationReference {
    schema: Option<String>,
    table: String,
    alias: Option<String>,
}

impl RelationReference {
    fn qualified_name(&self) -> String {
        self.schema.as_ref().map_or_else(
            || self.table.clone(),
            |schema| format!("{schema}.{}", self.table),
        )
    }

    fn completion_qualifier(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.table)
    }
}

fn referenced_relations(source: &str) -> Vec<RelationReference> {
    let dialect = PostgreSqlDialect {};
    let Ok(tokens) = Tokenizer::new(&dialect, source).tokenize() else {
        return Vec::new();
    };
    let tokens = tokens
        .into_iter()
        .filter(|token| !matches!(token, Token::Whitespace(_)))
        .collect::<Vec<_>>();
    let mut relations = Vec::new();
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
        let mut schema = None;
        let mut table = first_part.value.clone();
        let mut next = index + 2;
        if tokens
            .get(next)
            .is_some_and(|token| token.to_string() == ".")
            && let Some(Token::Word(table_part)) = tokens.get(next + 1)
        {
            schema = Some(table);
            table = table_part.value.clone();
            next += 2;
        }
        if matches!(tokens.get(next), Some(Token::Word(word)) if word.value.eq_ignore_ascii_case("AS"))
        {
            next += 1;
        }
        let alias = if let Some(Token::Word(alias)) = tokens.get(next)
            && alias.keyword == Keyword::NoKeyword
        {
            Some(alias.value.clone())
        } else {
            None
        };
        let relation = RelationReference {
            schema,
            table,
            alias,
        };
        if !relations.contains(&relation) {
            relations.push(relation);
        }
        index = next;
    }
    relations
}

fn columns_for_relations(schema_items: &[String], relations: &[RelationReference]) -> Vec<String> {
    let qualify = relations.len() > 1;
    let mut columns = relations
        .iter()
        .flat_map(|relation| {
            columns_for_relation(schema_items, relation)
                .into_iter()
                .map(move |column| {
                    let column = postgres_identifier(&column);
                    if qualify {
                        format!("{}.{}", relation.completion_qualifier(), column)
                    } else {
                        column
                    }
                })
        })
        .collect::<Vec<_>>();
    columns.sort_unstable();
    columns.dedup();
    columns
}

fn columns_for_relation(schema_items: &[String], relation: &RelationReference) -> Vec<String> {
    let table_prefix = format!("{}.", relation.table);
    let qualified_prefix = relation
        .schema
        .as_ref()
        .map(|schema| format!("{schema}.{}.", relation.table));
    let mut columns = schema_items
        .iter()
        .filter_map(|item| {
            if let Some(prefix) = qualified_prefix.as_deref()
                && let Some(column) = strip_prefix_ignore_ascii_case(item, prefix)
                && !column.contains('.')
            {
                return Some(column.to_owned());
            }
            strip_prefix_ignore_ascii_case(item, &table_prefix)
                .filter(|column| !column.contains('.'))
                .map(str::to_owned)
        })
        .collect::<Vec<_>>();
    columns.sort_unstable();
    columns.dedup();
    columns
}

fn strip_prefix_ignore_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
        .then(|| &value[prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_fuzzy_sql_keywords() {
        let items = candidates("SEL", 3, &[], &[], false);
        assert_eq!(items.first().map(String::as_str), Some("SELECT"));
    }

    #[test]
    fn relation_context_excludes_columns() {
        let all = vec!["userId".into(), "users".into()];
        let relations = vec!["users".into()];
        let items = candidates("SELECT * FROM use", 17, &all, &relations, false);
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
        let items = candidates(sql, sql.len(), &all, &[], false);
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
        let items = candidates(sql, sql.len(), &all, &[], false);
        assert_eq!(items, vec!["u.email", "u.id"]);
    }

    #[test]
    fn select_columns_come_only_from_the_table_named_after_the_cursor() {
        let all = vec![
            "tool_calls.id".into(),
            "tool_calls.arguments".into(),
            "tool_calls.result".into(),
            "messages.content".into(),
            "agents.display_name".into(),
        ];
        let sql = "SELECT id, display_name, con FROM tool_calls";
        let cursor = sql.find("con").unwrap() + 3;

        let items = candidates(sql, cursor, &all, &[], false);

        assert!(items.is_empty());
        assert!(!items.iter().any(|item| item == "content"));
    }

    #[test]
    fn forced_completion_lists_columns_for_a_later_from_clause() {
        let all = vec![
            "tool_calls.id".into(),
            "tool_calls.arguments".into(),
            "messages.content".into(),
        ];
        let sql = "SELECT  FROM tool_calls";
        let cursor = "SELECT ".len();

        let items = candidates(sql, cursor, &all, &[], true);

        assert_eq!(items, vec!["arguments", "id"]);
    }

    #[test]
    fn forced_from_completion_lists_relations_not_global_columns() {
        let all = vec!["messages.content".into()];
        let relations = vec!["messages".into(), "tool_calls".into()];
        let sql = "SELECT * FROM ";

        let items = candidates(sql, sql.len(), &all, &relations, true);

        assert_eq!(items, relations);
    }

    #[test]
    fn quotes_identifiers_that_postgres_would_case_fold() {
        assert_eq!(postgres_identifier("created_at"), "created_at");
        assert_eq!(postgres_identifier("createdAt"), "\"createdAt\"");
        assert_eq!(postgres_identifier("order-item"), "\"order-item\"");
    }
}
