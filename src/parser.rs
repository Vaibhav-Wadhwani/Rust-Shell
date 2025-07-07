// parser.rs

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QuoteType { None, Single, Double }

pub fn shell_split_shell_like(line: &str) -> Vec<(String, QuoteType)> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    enum State { Normal, Single, Double }
    let mut state = State::Normal;
    let mut last_quote = QuoteType::None;
    let mut token_quote = QuoteType::None;
    while let Some(ch) = chars.next() {
        match state {
            State::Normal => match ch {
                '\'' => {
                    state = State::Single;
                    if cur.is_empty() {
                        token_quote = QuoteType::Single;
                    }
                    last_quote = QuoteType::Single;
                },
                '"' => {
                    state = State::Double;
                    if cur.is_empty() {
                        token_quote = QuoteType::Double;
                    }
                    last_quote = QuoteType::Double;
                },
                '\\' => {
                    if let Some(&next) = chars.peek() {
                        cur.push(next);
                        chars.next();
                    }
                }
                c if c.is_whitespace() => {
                    if !cur.is_empty() {
                        tokens.push((cur.clone(), token_quote));
                        cur.clear();
                        last_quote = QuoteType::None;
                        token_quote = QuoteType::None;
                    }
                }
                _ => {
                    cur.push(ch);
                },
            },
            State::Single => match ch {
                '\'' => {
                    state = State::Normal;
                },
                _ => {
                    cur.push(ch);
                },
            },
            State::Double => match ch {
                '"' => {
                    state = State::Normal;
                },
                '\\' => {
                    if let Some(&next) = chars.peek() {
                        match next {
                            '\\' | '"' | '$' => {
                                cur.push(next);
                                chars.next();
                            }
                            '\'' => {
                                cur.push('\'');
                                chars.next();
                            }
                            _ => {
                                cur.push('\\');
                                cur.push(next);
                                chars.next();
                            }
                        }
                    } else {
                        cur.push('\\');
                    }
                }
                _ => {
                    cur.push(ch);
                },
            },
        }
    }
    if !cur.is_empty() {
        tokens.push((cur, token_quote));
    }
    tokens
}

pub fn shell_split_literal(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => {
                if let Some(&next) = chars.peek() {
                    cur.push(next);
                    chars.next();
                }
            }
            c if c.is_whitespace() => {
                if !cur.is_empty() {
                    tokens.push(cur.clone());
                    cur.clear();
                }
            }
            _ => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    tokens
}

pub fn unescape_backslashes(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if let Some(&next) = chars.peek() {
                match next {
                    ' ' | '\\' | '\t' | '\n' | '\'' => {
                        result.push(next);
                        chars.next();
                    }
                    _ => {
                        result.push('\\');
                        result.push(next);
                        chars.next();
                    }
                }
            } else {
                result.push('\\');
            }
        } else {
            result.push(ch);
        }
    }
    result
} 