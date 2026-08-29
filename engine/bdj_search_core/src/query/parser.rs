use super::ast::QueryAst;
use super::lexer::{Lexer, Token};

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    pub fn parse(input: &str) -> QueryAst {
        let mut lexer = Lexer::new(input);
        let tokens = lexer.tokenize();
        if tokens.is_empty() {
            return QueryAst::Term(String::new());
        }
        let mut parser = Parser::new(tokens);
        parser.parse_or_expr()
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token> {
        if self.pos < self.tokens.len() {
            let t = self.tokens[self.pos].clone();
            self.pos += 1;
            Some(t)
        } else {
            None
        }
    }

    fn parse_or_expr(&mut self) -> QueryAst {
        let mut terms = vec![self.parse_and_expr()];
        while let Some(Token::Pipe) = self.peek() {
            self.next();
            terms.push(self.parse_and_expr());
        }
        if terms.len() == 1 {
            terms.remove(0)
        } else {
            QueryAst::Or(terms)
        }
    }

    fn parse_and_expr(&mut self) -> QueryAst {
        let mut terms = Vec::new();
        while let Some(tok) = self.peek() {
            if *tok == Token::Pipe || *tok == Token::CloseParen {
                break;
            }
            terms.push(self.parse_unary_expr());
        }
        if terms.is_empty() {
            QueryAst::Term(String::new())
        } else if terms.len() == 1 {
            terms.remove(0)
        } else {
            QueryAst::And(terms)
        }
    }

    fn parse_unary_expr(&mut self) -> QueryAst {
        if let Some(Token::Bang) = self.peek() {
            self.next();
            QueryAst::Not(Box::new(self.parse_primary()))
        } else {
            self.parse_primary()
        }
    }

    fn parse_primary(&mut self) -> QueryAst {
        match self.next() {
            Some(Token::Word(w)) => {
                // If it looks like a path e.g. "D:\Music" or "C:\"
                if (w.len() >= 2 && w.chars().nth(1) == Some(':')) || w.contains('\\') || w.contains('/') {
                    QueryAst::Path(w)
                } else {
                    QueryAst::Term(w)
                }
            }
            Some(Token::Quoted(q)) => QueryAst::Exact(q),
            Some(Token::Function { name, value }) => self.parse_function(&name, &value),
            Some(Token::OpenParen) => {
                let inner = self.parse_or_expr();
                if let Some(Token::CloseParen) = self.peek() {
                    self.next();
                }
                inner
            }
            _ => QueryAst::Term(String::new()),
        }
    }

    fn parse_function(&self, name: &str, value: &str) -> QueryAst {
        match name {
            "ext" => {
                let exts = value
                    .split('|')
                    .map(|s| s.trim_start_matches('.').to_ascii_lowercase())
                    .collect();
                QueryAst::Ext(exts)
            }
            "size" | "tam" => Self::parse_size_query(value),
            "dm" | "mod" => Self::parse_date_query(value, true),
            "dc" | "creado" => Self::parse_date_query(value, false),
            "path" | "ruta" => QueryAst::Path(value.to_string()),
            "parent" | "carpeta" => QueryAst::Parent(value.to_string()),
            "type" | "tipo" => QueryAst::FileType(value.to_ascii_lowercase()),
            "regex" => QueryAst::Regex(value.to_string()),
            "case" | "may" => QueryAst::Case(value.to_string()),
            "file" | "archivo" => QueryAst::FileOnly,
            "folder" | "carpeta_solo" => QueryAst::FolderOnly,
            _ => QueryAst::Term(format!("{}:{}", name, value)),
        }
    }

    pub fn parse_size_query(value: &str) -> QueryAst {
        let trimmed = value.trim();
        if let Some((left, right)) = trimmed.split_once("..") {
            let min = Self::parse_size_bytes(left);
            let max = Self::parse_size_bytes(right);
            QueryAst::Size { min, max }
        } else if let Some(rest) = trimmed.strip_prefix(">=") {
            let min = Self::parse_size_bytes(rest);
            QueryAst::Size { min, max: None }
        } else if let Some(rest) = trimmed.strip_prefix('>') {
            let min = Self::parse_size_bytes(rest).map(|v| v + 1);
            QueryAst::Size { min, max: None }
        } else if let Some(rest) = trimmed.strip_prefix("<=") {
            let max = Self::parse_size_bytes(rest);
            QueryAst::Size { min: None, max }
        } else if let Some(rest) = trimmed.strip_prefix('<') {
            let max = Self::parse_size_bytes(rest).map(|v| v.saturating_sub(1));
            QueryAst::Size { min: None, max }
        } else {
            let exact = Self::parse_size_bytes(trimmed);
            QueryAst::Size { min: exact, max: exact }
        }
    }

    pub fn parse_size_bytes(s: &str) -> Option<u64> {
        let s = s.trim().to_ascii_lowercase();
        let (num_str, multiplier) = if s.ends_with("tb") || s.ends_with('t') {
            (s.trim_end_matches("tb").trim_end_matches('t'), 1024 * 1024 * 1024 * 1024u64)
        } else if s.ends_with("gb") || s.ends_with('g') {
            (s.trim_end_matches("gb").trim_end_matches('g'), 1024 * 1024 * 1024u64)
        } else if s.ends_with("mb") || s.ends_with('m') {
            (s.trim_end_matches("mb").trim_end_matches('m'), 1024 * 1024u64)
        } else if s.ends_with("kb") || s.ends_with('k') {
            (s.trim_end_matches("kb").trim_end_matches('k'), 1024u64)
        } else if s.ends_with('b') {
            (s.trim_end_matches('b'), 1u64)
        } else {
            (s.as_str(), 1u64)
        };

        num_str.trim().parse::<u64>().ok().map(|n| n * multiplier)
    }

    pub fn parse_date_query(value: &str, is_modified: bool) -> QueryAst {
        let s = value.trim().to_ascii_lowercase();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as u32)
            .unwrap_or(1787961600); // Aug 2026 fallback

        let (min, max) = match s.as_str() {
            "hoy" | "today" => (Some(now.saturating_sub(86400)), Some(now)),
            "ayer" | "yesterday" => (Some(now.saturating_sub(172800)), Some(now.saturating_sub(86400))),
            "estasemana" | "thisweek" => (Some(now.saturating_sub(7 * 86400)), Some(now)),
            "estemes" | "thismonth" => (Some(now.saturating_sub(30 * 86400)), Some(now)),
            "esteaño" | "thisyear" => (Some(now.saturating_sub(365 * 86400)), Some(now)),
            _ => {
                if let Some((left, right)) = s.split_once("..") {
                    (Self::parse_date_secs(left), Self::parse_date_secs(right))
                } else if let Some(rest) = s.strip_prefix('>') {
                    (Self::parse_date_secs(rest), None)
                } else if let Some(rest) = s.strip_prefix('<') {
                    (None, Self::parse_date_secs(rest))
                } else {
                    let ts = Self::parse_date_secs(&s);
                    (ts, ts.map(|t| t + 86400))
                }
            }
        };

        if is_modified {
            QueryAst::DateModified { min, max }
        } else {
            QueryAst::DateCreated { min, max }
        }
    }

    pub fn parse_date_secs(s: &str) -> Option<u32> {
        let parts: Vec<&str> = s.trim().split('-').collect();
        if parts.len() == 3 {
            let y: i32 = parts[0].parse().ok()?;
            let m: u32 = parts[1].parse().ok()?;
            let d: u32 = parts[2].parse().ok()?;
            // Simplified Unix epoch calculation for dates
            let days_since_1970 = (y - 1970) * 365 + ((y - 1968) / 4) + (m as i32 - 1) * 30 + d as i32;
            Some((days_since_1970.max(0) as u32) * 86400)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parser_grammar() {
        let ast = Parser::parse("michael jackson");
        assert_eq!(
            ast,
            QueryAst::And(vec![
                QueryAst::Term("michael".to_string()),
                QueryAst::Term("jackson".to_string())
            ])
        );

        let ast_or = Parser::parse("wav | flac");
        assert_eq!(
            ast_or,
            QueryAst::Or(vec![
                QueryAst::Term("wav".to_string()),
                QueryAst::Term("flac".to_string())
            ])
        );

        let ast_not = Parser::parse("!remix ext:wav");
        assert_eq!(
            ast_not,
            QueryAst::And(vec![
                QueryAst::Not(Box::new(QueryAst::Term("remix".to_string()))),
                QueryAst::Ext(vec!["wav".to_string()])
            ])
        );
    }

    #[test]
    fn test_size_parsing() {
        let ast = Parser::parse("size:>100mb");
        assert_eq!(ast, QueryAst::Size { min: Some(100 * 1024 * 1024 + 1), max: None });

        let ast_range = Parser::parse("tam:10mb..50mb");
        assert_eq!(ast_range, QueryAst::Size { min: Some(10 * 1024 * 1024), max: Some(50 * 1024 * 1024) });
    }
}
