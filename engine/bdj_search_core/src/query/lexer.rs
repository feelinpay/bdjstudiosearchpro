#[derive(Debug, PartialEq, Clone)]
pub enum Token {
    Word(String),
    Quoted(String),
    Function { name: String, value: String },
    Pipe,
    Bang,
    OpenParen,
    CloseParen,
}

pub struct Lexer<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            chars: input.chars().peekable(),
        }
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        while let Some(&c) = self.chars.peek() {
            match c {
                ' ' | '\t' | '\r' | '\n' => {
                    self.chars.next();
                }
                '|' => {
                    self.chars.next();
                    tokens.push(Token::Pipe);
                }
                '!' => {
                    self.chars.next();
                    tokens.push(Token::Bang);
                }
                '(' => {
                    self.chars.next();
                    tokens.push(Token::OpenParen);
                }
                ')' => {
                    self.chars.next();
                    tokens.push(Token::CloseParen);
                }
                '"' => {
                    self.chars.next();
                    let mut s = String::new();
                    for ch in self.chars.by_ref() {
                        if ch == '"' {
                            break;
                        }
                        s.push(ch);
                    }
                    tokens.push(Token::Quoted(s));
                }
                _ => {
                    let mut s = String::new();
                    while let Some(&ch) = self.chars.peek() {
                        if ch.is_whitespace() || ch == '|' || ch == ')' || ch == '(' {
                            break;
                        }
                        s.push(ch);
                        self.chars.next();
                    }
                    if let Some((func, val)) = s.split_once(':') {
                        tokens.push(Token::Function {
                            name: func.to_lowercase(),
                            value: val.to_string(),
                        });
                    } else {
                        tokens.push(Token::Word(s));
                    }
                }
            }
        }
        tokens
    }
}
