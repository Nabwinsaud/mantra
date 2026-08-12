use sqlparser::{
    dialect::PostgreSqlDialect,
    tokenizer::{Location, Token, Tokenizer},
};

pub fn current(source: &str, cursor: usize) -> &str {
    let dialect = PostgreSqlDialect {};
    let Ok(tokens) = Tokenizer::new(&dialect, source).tokenize_with_location() else {
        return source;
    };
    let semicolons = tokens
        .iter()
        .filter(|token| token.token == Token::SemiColon)
        .map(|token| location_to_byte(source, token.span.start))
        .collect::<Vec<_>>();
    // An editor cursor commonly sits just after `;`. Treat that position as
    // belonging to the statement that ends at the semicolon, not an empty one.
    let anchor = if cursor > 0 && source[..cursor].ends_with(';') {
        cursor - 1
    } else {
        cursor
    };
    let start = semicolons
        .iter()
        .copied()
        .take_while(|position| *position < anchor)
        .last()
        .map_or(0, |position| position + 1);
    let end = semicolons
        .iter()
        .copied()
        .find(|position| *position >= anchor)
        .map_or(source.len(), |position| position + 1);
    source[start..end].trim()
}

fn location_to_byte(source: &str, location: Location) -> usize {
    let line_start = source
        .split_inclusive('\n')
        .take(location.line.saturating_sub(1) as usize)
        .map(str::len)
        .sum::<usize>();
    line_start
        + source[line_start..]
            .chars()
            .take(location.column.saturating_sub(1) as usize)
            .map(char::len_utf8)
            .sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_statement_containing_cursor() {
        let sql = "SELECT 1;\n\nSELECT * FROM users;";
        assert_eq!(current(sql, 4), "SELECT 1;");
        assert_eq!(current(sql, 20), "SELECT * FROM users;");
    }

    #[test]
    fn ignores_semicolons_inside_strings() {
        let sql = "SELECT ';' AS value; SELECT 2;";
        assert_eq!(current(sql, 8), "SELECT ';' AS value;");
    }

    #[test]
    fn cursor_immediately_after_semicolon_selects_previous_statement() {
        let sql = "SELECT * FROM users;";
        assert_eq!(current(sql, sql.len()), sql);
    }
}
