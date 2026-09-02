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
                    // Una palabra llega hasta el primer espacio... salvo que se
                    // abran comillas.
                    //
                    // `ruta:"Mis Sets"` se partía en dos piezas, «ruta:"Mis» y
                    // «Sets"», así que el filtro buscaba una ruta llamada
                    // literalmente `"Mis` y devolvía cero resultados sin dar
                    // ningún error. Con rutas de Windows —que casi siempre
                    // llevan espacios: `C:\Users\David Zapata\...`— era el caso
                    // normal, no el raro.
                    //
                    // Dentro de comillas los espacios forman parte del valor y
                    // solo la comilla de cierre lo termina.
                    let mut s = String::new();
                    let mut dentro_de_comillas = false;
                    while let Some(&ch) = self.chars.peek() {
                        if ch == '"' {
                            dentro_de_comillas = !dentro_de_comillas;
                            s.push(ch);
                            self.chars.next();
                            continue;
                        }
                        if !dentro_de_comillas
                            && (ch.is_whitespace() || ch == '|' || ch == ')' || ch == '(')
                        {
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
