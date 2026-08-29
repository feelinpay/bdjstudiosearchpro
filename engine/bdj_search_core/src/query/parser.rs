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
            Some(Token::Word(w)) => QueryAst::Term(w),
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
                let exts = value.split('|').map(|s| s.trim_start_matches('.').to_lowercase()).collect();
                QueryAst::Ext(exts)
            }
            "size" | "tam" => {
                // Parsing basic size ranges
                QueryAst::Size { min: None, max: None }
            }
            "path" | "ruta" => QueryAst::Path(value.to_string()),
            "parent" | "carpeta" => QueryAst::Parent(value.to_string()),
            "type" | "tipo" => QueryAst::FileType(value.to_lowercase()),
            "regex" => QueryAst::Regex(value.to_string()),
            "case" | "may" => QueryAst::Case(value.to_string()),
            "file" | "archivo" => QueryAst::FileOnly,
            "folder" | "carpeta_solo" => QueryAst::FolderOnly,
            _ => QueryAst::Term(format!("{}:{}", name, value)),
        }
    }
}
