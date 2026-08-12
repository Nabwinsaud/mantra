use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use sqlparser::{
    dialect::PostgreSqlDialect,
    keywords::Keyword,
    tokenizer::{Token, Tokenizer, Whitespace},
};

pub fn lines(source: &str) -> Vec<Line<'static>> {
    let dialect = PostgreSqlDialect {};
    let Ok(tokens) = Tokenizer::new(&dialect, source).tokenize() else {
        return source
            .lines()
            .map(|line| Line::raw(line.to_owned()))
            .collect();
    };
    let mut lines = vec![Line::default()];
    for token in tokens {
        if matches!(token, Token::Whitespace(Whitespace::Newline)) {
            lines.push(Line::default());
            continue;
        }
        let style = token_style(&token);
        lines
            .last_mut()
            .expect("at least one line")
            .spans
            .push(Span::styled(token.to_string(), style));
    }
    lines
}

fn token_style(token: &Token) -> Style {
    match token {
        Token::Word(word) if word.keyword != Keyword::NoKeyword => Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
        Token::Word(word) if word.quote_style.is_some() => Style::default().fg(Color::Cyan),
        Token::Number(..) => Style::default().fg(Color::LightBlue),
        Token::SingleQuotedString(_)
        | Token::DoubleQuotedString(_)
        | Token::DollarQuotedString(_)
        | Token::EscapedStringLiteral(_)
        | Token::NationalStringLiteral(_) => Style::default().fg(Color::Green),
        Token::Whitespace(Whitespace::SingleLineComment { .. })
        | Token::Whitespace(Whitespace::MultiLineComment(_)) => Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
        Token::Eq
        | Token::Neq
        | Token::Lt
        | Token::Gt
        | Token::LtEq
        | Token::GtEq
        | Token::Plus
        | Token::Minus
        | Token::Mul
        | Token::Div => Style::default().fg(Color::Yellow),
        _ => Style::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_multiline_sql_text() {
        let highlighted = lines("SELECT 1;\n-- hello");
        assert_eq!(highlighted.len(), 2);
        assert_eq!(
            highlighted[0]
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "SELECT 1;"
        );
    }
}
