//! Tokenizer of the mission files' script text. Source is treated as Latin-1 (one char per byte).

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    /// `%name`, including `::` segments.
    Var(String),
    Int(u32),
    Float(f64),
    Str(String),
    Tagged(String),
    /// Punctuation, operators and keywords (`if`, `SPC`, `$=`, ...).
    Sym(&'static str),
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub line: u32,
}

const KEYWORDS: &[&str] = &[
    "function", "package", "datablock", "new", "if", "else", "while", "for", "do", "switch", "switch$", "case", "or",
    "default", "return", "break", "continue", "true", "false", "SPC", "TAB", "NL",
];

// Longest first so that maximal munch works with a simple prefix scan.
const SYMBOLS: &[&str] = &[
    "!$=", "<<=", ">>=", "::", "==", "!=", "<=", ">=", "&&", "||", "<<", ">>", "+=", "-=", "*=", "/=", "%=", "&=", "|=",
    "^=", "++", "--", "$=", "+", "-", "*", "/", "%", "^", "&", "|", "~", "!", "<", ">", "=", ".", ",", ";", ":", "?",
    "(", ")", "[", "]", "{", "}", "@",
];

fn ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

pub fn tokenize(src: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut line = 1;
    let mut out = Vec::new();
    let err = |line: u32, msg: &str| Err(format!("line {line}: {msg}"));
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i < chars.len() && !(chars[i] == '*' && chars.get(i + 1) == Some(&'/')) {
                if chars[i] == '\n' {
                    line += 1;
                }
                i += 1;
            }
            i += 2;
            continue;
        }
        let start_line = line;
        let tok = if (c == '$' || c == '%') && chars.get(i + 1).is_some_and(|&n| ident_start(n) || n == ':') {
            let start = i;
            i += 1;
            loop {
                if chars.get(i).is_some_and(|&c| ident_char(c)) {
                    i += 1;
                } else if chars.get(i) == Some(&':') && chars.get(i + 1) == Some(&':') {
                    i += 2;
                } else if chars.get(i) == Some(&':') && chars.get(i + 1).is_some_and(|&c| ident_start(c)) {
                    // A single colon inside a variable name is accepted too.
                    i += 1;
                } else {
                    break;
                }
            }
            Tok::Var(chars[start..i].iter().collect())
        } else if ident_start(c) {
            let start = i;
            while chars.get(i).is_some_and(|&c| ident_char(c)) {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if word == "switch" && chars.get(i) == Some(&'$') {
                i += 1;
                Tok::Sym("switch$")
            } else if let Some(k) = KEYWORDS.iter().find(|&&k| k == word) {
                Tok::Sym(k)
            } else {
                Tok::Ident(word)
            }
        } else if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit())) {
            let start = i;
            if c == '0' && matches!(chars.get(i + 1), Some('x' | 'X')) {
                i += 2;
                while chars.get(i).is_some_and(|c| c.is_ascii_hexdigit()) {
                    i += 1;
                }
                let s: String = chars[start + 2..i].iter().collect();
                match u32::from_str_radix(&s, 16) {
                    Ok(v) => Tok::Int(v),
                    Err(_) => return err(line, "bad hex literal"),
                }
            } else {
                let mut is_float = false;
                while chars.get(i).is_some_and(|c| c.is_ascii_digit()) {
                    i += 1;
                }
                if chars.get(i) == Some(&'.') && chars.get(i + 1).is_some_and(|c| c.is_ascii_digit()) {
                    is_float = true;
                    i += 1;
                    while chars.get(i).is_some_and(|c| c.is_ascii_digit()) {
                        i += 1;
                    }
                }
                if matches!(chars.get(i), Some('e' | 'E')) {
                    let mut j = i + 1;
                    if matches!(chars.get(j), Some('+' | '-')) {
                        j += 1;
                    }
                    if chars.get(j).is_some_and(|c| c.is_ascii_digit()) {
                        is_float = true;
                        i = j;
                        while chars.get(i).is_some_and(|c| c.is_ascii_digit()) {
                            i += 1;
                        }
                    }
                }
                let s: String = chars[start..i].iter().collect();
                if is_float {
                    Tok::Float(s.parse().map_err(|_| format!("line {line}: bad float {s}"))?)
                } else {
                    match s.parse::<u32>() {
                        Ok(v) => Tok::Int(v),
                        Err(_) => Tok::Float(s.parse().map_err(|_| format!("line {line}: bad number {s}"))?),
                    }
                }
            }
        } else if c == '"' || c == '\'' {
            i += 1;
            let mut s = String::new();
            loop {
                let Some(&ch) = chars.get(i) else { return err(start_line, "unterminated string") };
                i += 1;
                if ch == c {
                    break;
                }
                if ch == '\n' {
                    line += 1;
                }
                if ch != '\\' {
                    s.push(ch);
                    continue;
                }
                let Some(&e) = chars.get(i) else { return err(line, "bad escape") };
                i += 1;
                match e {
                    'n' => s.push('\n'),
                    't' => s.push('\t'),
                    'r' => s.push('\r'),
                    'x' => {
                        let hex: String = chars.get(i..i + 2).map(|h| h.iter().collect()).unwrap_or_default();
                        let v = u8::from_str_radix(&hex, 16).map_err(|_| format!("line {line}: bad \\x escape"))?;
                        s.push(v as char);
                        i += 2;
                    }
                    'c' => {
                        // Colour codes: \c0..\c9 and \cr, \cp, \co.
                        const MAP: [u8; 10] = [0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0xb, 0xc, 0xe, 0xf];
                        let k = chars.get(i).copied().unwrap_or(' ');
                        i += 1;
                        let v = match k {
                            '0'..='9' => MAP[k as usize - '0' as usize],
                            'r' => 0xf,
                            'p' => 0x10,
                            'o' => 0x11,
                            _ => return err(line, "bad \\c escape"),
                        };
                        s.push(v as char);
                    }
                    other => s.push(other),
                }
            }
            if c == '"' {
                Tok::Str(s)
            } else {
                Tok::Tagged(s)
            }
        } else if let Some(sym) = SYMBOLS.iter().find(|s| {
            let n = s.len();
            chars.get(i..i + n).is_some_and(|w| w.iter().copied().eq(s.chars()))
        }) {
            i += sym.len();
            Tok::Sym(sym)
        } else {
            return err(line, &format!("unexpected character {c:?}"));
        };
        out.push(Token { tok, line: start_line });
    }
    out.push(Token { tok: Tok::Eof, line });
    Ok(out)
}
