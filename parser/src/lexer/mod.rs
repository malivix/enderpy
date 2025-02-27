use unicode_id_start::{is_id_continue, is_id_start};

use crate::{
    error::LexError,
    token::{Kind, Token, TokenValue},
};

#[derive(Debug)]
pub struct Lexer {
    /// The source code
    source: String,
    /// The current position in the source code
    current: usize,
    current_line: u16,
    /// Keeps track of whether the lexer is at the start of a line
    start_of_line: bool,
    /// keeps track of the indentation level
    /// the first element is always 0
    /// because the first line is always at indentation level 0
    indent_stack: Vec<usize>,
    nesting: i8,
    // This stack means we are in a fstring that started with
    // character at the top of the stack
    fstring_stack: Vec<String>,
    // This is a counter used to keep track of how many
    // brackets we are inside
    // Each time we see a left bracket we increment this
    // Each time we see a right bracket we decrement this
    // This is used to match brackets in fstrings
    inside_fstring_bracket: i8,

    // TODO: Hacky way to handle emitting multiple de indents
    next_token_is_dedent: u8,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        Self {
            source: source.to_string(),
            current: 0,
            current_line: 1,
            start_of_line: true,
            indent_stack: vec![0],
            nesting: 0,
            fstring_stack: vec![],
            inside_fstring_bracket: 0,
            next_token_is_dedent: 0,
        }
    }

    pub fn next_token(&mut self) -> Token {
        if self.next_token_is_dedent > 0 {
            self.next_token_is_dedent -= 1;
            return Token {
                kind: Kind::Dedent,
                value: TokenValue::None,
                start: self.current,
                end: self.current,
            };
        }

        let start = self.current;
        let kind = match self.next_kind() {
            Ok(kind) => kind,
            Err(e) => {
                return Token {
                    kind: Kind::Error,
                    value: TokenValue::Str(e.to_string()),
                    start,
                    end: match e {
                        //  If the string is not terminated it means that we consumed all the
                        // characters  in the source code and we are at the
                        // end of the file  so we return the position of
                        // character before the end of the file
                        LexError::StringNotTerminated => self.current - 1,
                        _ => self.current,
                    },
                };
            }
        };

        // Ignore whitespace
        if kind == Kind::WhiteSpace {
            return self.next_token();
        }
        let raw_value = self.extract_raw_token_value(start);
        let value = self.parse_token_value(kind, raw_value);
        let end = self.current;

        Token {
            kind,
            value,
            start,
            end,
        }
    }

    // peek_token is a side-effect free version of next_token
    pub fn peek_token(&mut self) -> Token {
        let current = self.current;
        let current_line = self.current_line;
        let nesting = self.nesting;
        let fstring_stack = self.fstring_stack.clone();
        let start_of_line = self.start_of_line;
        let inside_fstring_bracket = self.inside_fstring_bracket;
        let next_token_is_dedent = self.next_token_is_dedent;
        let token = self.next_token();
        self.current = current;
        self.current_line = current_line;
        self.nesting = nesting;
        self.fstring_stack = fstring_stack;
        self.start_of_line = start_of_line;
        self.inside_fstring_bracket = inside_fstring_bracket;
        self.next_token_is_dedent = next_token_is_dedent;
        token
    }

    pub fn next_fstring_token(&mut self) -> Option<Kind> {
        if self.inside_fstring_bracket > 0 {
            if self.peek() == Some('}') && self.double_peek() != Some('}') {
                self.next();
                self.inside_fstring_bracket -= 1;
                return Some(Kind::RightBracket);
            }
            // if we are inside a bracket return none
            // and let the other tokens be matched
            return None;
        }
        let mut consumed_str_in_fstring = String::new();
        while let Some(curr) = self.next() {
            let str_finisher = self.fstring_stack.last().unwrap();
            if curr == '{' {
                if let Some('{') = self.peek() {
                    consumed_str_in_fstring.push(curr);
                    consumed_str_in_fstring.push('{');
                    self.double_next();
                    continue;
                }

                self.inside_fstring_bracket += 1;
                return Some(Kind::LeftBracket);
            }
            match str_finisher.len() {
                1 => {
                    if curr == str_finisher.chars().next().unwrap() {
                        self.fstring_stack.pop();
                        return Some(Kind::FStringEnd);
                    }
                }
                3 => {
                    if curr == str_finisher.chars().next().unwrap()
                        && self.peek() == Some(str_finisher.chars().nth(1).unwrap())
                        && self.double_peek() == Some(str_finisher.chars().nth(2).unwrap())
                    {
                        self.fstring_stack.pop();
                        self.double_next();
                        return Some(Kind::FStringEnd);
                    }
                }
                _ => {}
            }

            let peeked_char = self.peek()?;
            if peeked_char == '{' && self.double_peek() != Some('{') {
                return Some(Kind::FStringMiddle);
            }
            // if last consumed_str_in_fstring is a backslash
            if consumed_str_in_fstring.ends_with('\\') {
                consumed_str_in_fstring.push(curr);
                continue;
            }
            match str_finisher.len() {
                1 => {
                    if peeked_char == str_finisher.chars().next().unwrap() {
                        return Some(Kind::FStringMiddle);
                    }
                }
                3 => {
                    if peeked_char == str_finisher.chars().next().unwrap()
                        && self.double_peek() == Some(str_finisher.chars().nth(1).unwrap())
                        && self.triple_peek() == Some(str_finisher.chars().nth(2).unwrap())
                    {
                        return Some(Kind::FStringMiddle);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn next_kind(&mut self) -> Result<Kind, LexError> {
        if self.start_of_line {
            if let Some(indent_kind) = self.match_indentation()? {
                self.start_of_line = false; // WHY!?
                return Ok(indent_kind);
            }
        }
        if !self.fstring_stack.is_empty() {
            if let Some(kind) = self.next_fstring_token() {
                return Ok(kind);
            }
        }

        while let Some(c) = self.next() {
            self.start_of_line = false; // WHY AGAIN?!

            match c {
                // Numbers
                '0'..='9' => return self.match_numeric_literal(),
                // Identifiers & Keywords
                id_start @ 'a'..='z' | id_start @ 'A'..='Z' | id_start @ '_' => {
                    return self.match_id_keyword(id_start);
                }
                // String Literals
                str_start @ '"' | str_start @ '\'' => {
                    self.skip_to_str_end(str_start)?;
                    return Ok(Kind::StringLiteral);
                }
                // Operators
                '+' => match self.peek() {
                    Some('=') => {
                        self.next();
                        return Ok(Kind::AddAssign);
                    }
                    _ => return Ok(Kind::Plus),
                },
                '-' => match self.peek() {
                    Some('>') => {
                        self.next();
                        return Ok(Kind::Arrow);
                    }
                    Some('=') => {
                        self.next();
                        return Ok(Kind::SubAssign);
                    }
                    _ => return Ok(Kind::Minus),
                },
                '*' => match self.peek() {
                    Some('=') => {
                        self.next();
                        return Ok(Kind::MulAssign);
                    }
                    Some('*') => match self.double_peek() {
                        Some('=') => {
                            self.double_next();
                            return Ok(Kind::PowAssign);
                        }
                        _ => {
                            self.next();
                            return Ok(Kind::Pow);
                        }
                    },
                    _ => return Ok(Kind::Mul),
                },
                '/' => match self.peek() {
                    Some('/') => {
                        if let Some('=') = self.double_peek() {
                            self.double_next();
                            return Ok(Kind::IntDivAssign);
                        }
                        self.next();
                        return Ok(Kind::IntDiv);
                    }
                    Some('=') => {
                        self.next();
                        return Ok(Kind::DivAssign);
                    }
                    _ => return Ok(Kind::Div),
                },
                '%' => match self.peek() {
                    Some('=') => {
                        self.next();
                        return Ok(Kind::ModAssign);
                    }
                    _ => return Ok(Kind::Mod),
                },
                '@' => match self.peek() {
                    Some('=') => {
                        self.next();
                        return Ok(Kind::MatrixMulAssign);
                    }
                    _ => return Ok(Kind::MatrixMul),
                },
                '&' => match self.peek() {
                    Some('=') => {
                        self.next();
                        return Ok(Kind::BitAndAssign);
                    }
                    _ => return Ok(Kind::BitAnd),
                },
                '|' => match self.peek() {
                    Some('=') => {
                        self.next();
                        return Ok(Kind::BitOrAssign);
                    }
                    _ => return Ok(Kind::BitOr),
                },
                '^' => match self.peek() {
                    Some('=') => {
                        self.next();
                        return Ok(Kind::BitXorAssign);
                    }
                    _ => return Ok(Kind::BitXor),
                },
                '~' => return Ok(Kind::BitNot),
                ':' => match self.peek() {
                    Some('=') => {
                        self.next();
                        return Ok(Kind::Walrus);
                    }
                    _ => return Ok(Kind::Colon),
                },
                '!' => {
                    if let Some('=') = self.peek() {
                        self.next();
                        return Ok(Kind::NotEq);
                    }
                }
                // Delimiters
                '(' => {
                    self.nesting += 1;
                    return Ok(Kind::LeftParen);
                }
                ')' => {
                    self.nesting -= 1;
                    return Ok(Kind::RightParen);
                }
                '[' => {
                    self.nesting += 1;
                    return Ok(Kind::LeftBrace);
                }
                ']' => {
                    self.nesting -= 1;
                    return Ok(Kind::RightBrace);
                }
                '{' => {
                    self.nesting += 1;
                    return Ok(Kind::LeftBracket);
                }
                '}' => {
                    self.nesting -= 1;
                    return Ok(Kind::RightBracket);
                }
                ',' => return Ok(Kind::Comma),
                '.' => {
                    if let Some('.') = self.peek() {
                        if let Some('.') = self.double_peek() {
                            self.double_next();
                            return Ok(Kind::Ellipsis);
                        }
                    }
                    return Ok(Kind::Dot);
                }
                ';' => return Ok(Kind::SemiColon),
                '=' => match self.peek() {
                    Some('=') => {
                        self.next();
                        return Ok(Kind::Eq);
                    }
                    _ => return Ok(Kind::Assign),
                },
                '#' => {
                    // consume until new line
                    while let Some(c) = self.peek() {
                        if c == '\n' || c == '\r' {
                            break;
                        }
                        self.next();
                    }
                    return Ok(Kind::Comment);
                }
                '\\' => return Ok(Kind::BackSlash),
                '$' => return Ok(Kind::Dollar),
                '?' => return Ok(Kind::QuestionMark),
                '`' => return Ok(Kind::BackTick),
                '<' => match self.peek() {
                    Some('<') => match self.double_peek() {
                        Some('=') => {
                            self.double_next();
                            return Ok(Kind::ShiftLeftAssign);
                        }
                        _ => {
                            self.next();
                            return Ok(Kind::LeftShift);
                        }
                    },
                    Some('=') => {
                        self.next();
                        return Ok(Kind::LessEq);
                    }
                    _ => return Ok(Kind::Less),
                },
                '>' => match self.peek() {
                    Some('>') => match self.double_peek() {
                        Some('=') => {
                            self.double_next();
                            return Ok(Kind::ShiftRightAssign);
                        }
                        _ => {
                            self.next();
                            return Ok(Kind::RightShift);
                        }
                    },
                    Some('=') => {
                        self.next();
                        return Ok(Kind::GreaterEq);
                    }
                    _ => return Ok(Kind::Greater),
                },
                '\n' | '\r' => {
                    self.current_line += 1;
                    // Expressions in parentheses, square brackets or curly braces can be split over
                    // more than one physical line without using backslashes.
                    // The indentation of the continuation lines is not important. Blank
                    // continuation lines are allowed. There is no NEWLINE token
                    // between implicit continuation lines.
                    if self.nesting == 0 {
                        self.start_of_line = true;
                        return Ok(Kind::NewLine);
                    } else {
                        return Ok(Kind::WhiteSpace);
                    }
                }
                c if match_whitespace(c) => return Ok(Kind::WhiteSpace),
                _ => {}
            }
        }

        Ok(Kind::Eof)
    }

    fn match_id_keyword(&mut self, id_start: char) -> Result<Kind, LexError> {
        if let Some(str_kind) = self.match_str(id_start)? {
            return Ok(str_kind);
        }
        let id = self.extract_id(id_start);
        Ok(self.match_keyword(&id))
    }

    fn extract_id(&mut self, id_start: char) -> String {
        let mut id = String::new();
        id.push(id_start);
        while let Some(c) = self.peek() {
            match c {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '_' => {
                    id.push(c);
                    self.next();
                }
                _ => {
                    // Some unicode characters are valid in identifiers
                    // https://boshen.github.io/javascript-parser-in-rust/docs/lexer/#identifiers-and-unicode
                    if is_id_start(c) & is_id_continue(c) {
                        id.push(c);
                        self.next();
                    } else {
                        break;
                    }
                }
            }
        }
        id
    }

    fn match_str(&mut self, id_start: char) -> Result<Option<Kind>, LexError> {
        match id_start {
            'r' | 'R' => match self.peek() {
                // check if is start of string
                Some('b') | Some('B') => match self.double_peek() {
                    Some(str_start @ '"') | Some(str_start @ '\'') => {
                        self.double_next();
                        self.skip_to_str_end(str_start)?;
                        return Ok(Some(Kind::RawBytes));
                    }
                    _ => {}
                },
                Some('f') | Some('F') => match self.double_peek() {
                    Some(str_start @ '"') | Some(str_start @ '\'') => {
                        self.double_next();
                        let fstring_start = self.create_f_string_start(str_start);
                        self.fstring_stack.push(fstring_start);
                        return Ok(Some(Kind::RawFStringStart));
                    }
                    _ => {}
                },
                Some(str_start @ '"') | Some(str_start @ '\'') => {
                    self.next();
                    self.skip_to_str_end(str_start)?;
                    return Ok(Some(Kind::StringLiteral));
                }
                _ => {}
            },
            'b' | 'B' => match self.peek() {
                Some('r') | Some('R') => match self.double_peek() {
                    Some(str_start @ '"') | Some(str_start @ '\'') => {
                        self.double_next();
                        self.skip_to_str_end(str_start)?;
                        return Ok(Some(Kind::RawBytes));
                    }
                    _ => {}
                },
                Some(str_start @ '"') | Some(str_start @ '\'') => {
                    self.next();
                    self.skip_to_str_end(str_start)?;
                    return Ok(Some(Kind::Bytes));
                }
                _ => {}
            },
            'f' | 'F' => match self.peek() {
                Some('r') | Some('R') => match self.double_peek() {
                    Some(str_start @ '"') | Some(str_start @ '\'') => {
                        self.double_next();
                        let fstring_start = self.create_f_string_start(str_start);
                        self.fstring_stack.push(fstring_start);
                        return Ok(Some(Kind::RawFStringStart));
                    }
                    _ => {}
                },
                Some(str_start @ '"') | Some(str_start @ '\'') => {
                    self.next();
                    let fstring_start = self.create_f_string_start(str_start);
                    self.fstring_stack.push(fstring_start);
                    return Ok(Some(Kind::FStringStart));
                }
                _ => {}
            },
            'u' | 'U' => match self.peek() {
                Some(str_start @ '"') | Some(str_start @ '\'') => {
                    self.next();
                    self.skip_to_str_end(str_start)?;
                    return Ok(Some(Kind::Unicode));
                }
                _ => {}
            },
            _ => {}
        };
        Ok(None)
    }

    fn extract_raw_token_value(&mut self, start: usize) -> String {
        self.source[start..self.current].to_string()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.peek();
        if let Some(c) = c {
            self.current += c.len_utf8();
        }
        c
    }

    fn double_next(&mut self) -> Option<char> {
        self.next();
        self.next()
    }

    fn peek(&self) -> Option<char> {
        self.source[self.current..].chars().next()
    }

    fn double_peek(&self) -> Option<char> {
        self.source[self.current..].chars().nth(1)
    }

    fn triple_peek(&self) -> Option<char> {
        self.source[self.current..].chars().nth(2)
    }

    fn match_keyword(&self, ident: &str) -> Kind {
        match ident {
            "False" => Kind::False,
            "None" => Kind::None,
            "True" => Kind::True,
            "and" => Kind::And,
            "as" => Kind::As,
            "assert" => Kind::Assert,
            "async" => Kind::Async,
            "await" => Kind::Await,
            "break" => Kind::Break,
            "class" => Kind::Class,
            "continue" => Kind::Continue,
            "def" => Kind::Def,
            "del" => Kind::Del,
            "elif" => Kind::Elif,
            "else" => Kind::Else,
            "except" => Kind::Except,
            "finally" => Kind::Finally,
            "for" => Kind::For,
            "from" => Kind::From,
            "global" => Kind::Global,
            "if" => Kind::If,
            "import" => Kind::Import,
            "in" => Kind::In,
            "is" => Kind::Is,
            "lambda" => Kind::Lambda,
            "nonlocal" => Kind::Nonlocal,
            "not" => Kind::Not,
            "or" => Kind::Or,
            "pass" => Kind::Pass,
            "raise" => Kind::Raise,
            "return" => Kind::Return,
            "try" => Kind::Try,
            "while" => Kind::While,
            "with" => Kind::With,
            "yield" => Kind::Yield,
            _ => Kind::Identifier,
        }
    }

    fn skip_to_str_end(&mut self, str_start: char) -> Result<(), LexError> {
        // string start position is current position - 1 because we already consumed the
        // quote
        let _str_start_pos = self.current - 1;
        let mut string_terminated = false;
        let mut last_read_char = str_start;
        // Check if string starts with triple quotes
        // if string started with triple quotes, we need to read 3 characters at a time
        if self.peek() == Some(str_start) && self.double_peek() == Some(str_start) {
            self.next();
            while let Some(c) = self.next() {
                if c == str_start
                    && self.peek() == Some(str_start)
                    && self.double_peek() == Some(str_start)
                    && last_read_char != '\\'
                {
                    string_terminated = true;
                    self.double_next();
                    break;
                }
                last_read_char = c;
            }
        } else {
            while let Some(c) = self.next() {
                if c == str_start && last_read_char != '\\' {
                    string_terminated = true;
                    break;
                }
                last_read_char = c;
            }
        }

        if !string_terminated {
            Err(LexError::StringNotTerminated)
        } else {
            Ok(())
        }
    }

    fn match_numeric_literal(&mut self) -> Result<Kind, LexError> {
        // first number is already consumed
        let _numeric_literal_start = self.current - 1;
        match self.peek() {
            Some('b') | Some('B') => {
                self.next();
                while let Some(c) = self.peek() {
                    match c {
                        '0' | '1' => {
                            self.next();
                        }
                        '_' => {
                            self.next();
                        }
                        _ => return Err(LexError::InvalidDigitInBinaryLiteral(c)),
                    }
                }
                return Ok(Kind::Binary);
            }
            Some('o') | Some('O') => {
                self.next();
                while let Some(c) = self.peek() {
                    match c {
                        '0'..='7' => {
                            self.next();
                        }
                        '_' => {
                            self.next();
                        }
                        _ => return Err(LexError::InvalidDigitInOctalLiteral(c)),
                    }
                }
                return Ok(Kind::Octal);
            }
            Some('x') | Some('X') => {
                self.next();
                while let Some(c) = self.peek() {
                    match c {
                        '0'..='9' | 'a'..='f' | 'A'..='F' => {
                            self.next();
                        }
                        '_' => {
                            self.next();
                        }
                        _ => return Err(LexError::InvalidDigitInHexadecimalLiteral(c)),
                    }
                }
                return Ok(Kind::Hexadecimal);
            }
            _ => {}
        }

        let mut is_imaginary = false;
        while let Some(c) = self.peek() {
            match c {
                '.' => {
                    let mut has_exponent = false;
                    self.next();
                    while let Some(c) = self.peek() {
                        match c {
                            '0'..='9' => {
                                self.next();
                            }
                            '_' => {
                                self.next();
                            }
                            'e' | 'E' => {
                                has_exponent = true;
                                self.next();
                                match self.peek() {
                                    Some('+') | Some('-') => {
                                        self.next();
                                    }
                                    Some('0'..='9') => {
                                        self.next();
                                    }
                                    _ => return Err(LexError::InvalidDigitInDecimalLiteral),
                                }
                            }
                            'j' | 'J' => {
                                is_imaginary = true;
                                self.next();
                                break;
                            }
                            _ => break,
                        }
                    }
                    if has_exponent {
                        if is_imaginary {
                            return Ok(Kind::ImaginaryExponentFloat);
                        }
                        return Ok(Kind::ExponentFloat);
                    }
                    if is_imaginary {
                        return Ok(Kind::ImaginaryPointFloat);
                    }
                    return Ok(Kind::PointFloat);
                }
                'e' | 'E' => {
                    self.next();
                    match self.peek() {
                        Some('+') | Some('-') => {
                            self.next();
                        }
                        _ => {}
                    }
                    while let Some(c) = self.peek() {
                        match c {
                            '0'..='9' | '_' => {
                                self.next();
                            }
                            'j' | 'J' => {
                                is_imaginary = true;
                                self.next();
                                break;
                            }
                            _ => break,
                        }
                    }
                    if is_imaginary {
                        return Ok(Kind::ImaginaryExponentFloat);
                    }
                    return Ok(Kind::ExponentFloat);
                }
                '0'..='9' => {
                    self.next();
                }
                '_' => {
                    self.next();
                }
                'j' | 'J' => {
                    is_imaginary = true;
                    self.next();
                    break;
                }
                _ => break,
            }
        }

        if is_imaginary {
            return Ok(Kind::ImaginaryInteger);
        }

        Ok(Kind::Integer)
    }

    fn match_indentation(&mut self) -> Result<Option<Kind>, LexError> {
        use std::cmp::Ordering;
        let mut spaces_count = 0;
        while let Some(c) = self.peek() {
            match c {
                '\t' => {
                    spaces_count += 4;
                    self.next();
                }
                ' ' => {
                    spaces_count += 1;
                    self.next();
                }
                _ => {
                    break;
                }
            }
        }
        if spaces_count == 0 {
            // When there are no spaces and only a new line
            // like the following
            // if True:
            //
            //   print("Hello")
            //
            // We should not consider that new line as dedent
            //
            // But also if this part is the last part of the file
            // we should not consider it as dedent
            // Thanks python
            if self.peek() == Some('\n') && self.double_peek().is_some() {
                return Ok(None);
            }
        }
        if let Some(top) = self.indent_stack.last() {
            match spaces_count.cmp(top) {
                Ordering::Less => {
                    // loop over indent stack from the top and check if this element matches the
                    // new indentation level if nothing matches then it is an error
                    // do not pop the element from the stack
                    let mut indentation_matches_outer_evel = false;
                    for top in self.indent_stack.iter().rev() {
                        if top == &spaces_count {
                            indentation_matches_outer_evel = true;
                            break;
                        }
                    }
                    if !indentation_matches_outer_evel {
                        return Err(LexError::UnindentDoesNotMatchAnyOuterIndentationLevel);
                    }
                    Ok(Some(Kind::Dedent))
                }
                // Returning whitespace to ignore these spaces
                Ordering::Equal => Ok(Some(Kind::WhiteSpace)),
                Ordering::Greater => {
                    self.indent_stack.push(spaces_count);
                    Ok(Some(Kind::Indent))
                }
            }
        } else {
            Ok(None)
        }
    }

    fn parse_token_value(&mut self, kind: Kind, kind_value: String) -> TokenValue {
        use std::cmp::Ordering;
        match kind {
            Kind::Integer
            | Kind::Hexadecimal
            | Kind::Binary
            | Kind::PointFloat
            | Kind::Octal
            | Kind::ExponentFloat
            | Kind::ImaginaryInteger
            | Kind::ImaginaryExponentFloat
            | Kind::ImaginaryPointFloat => TokenValue::Number(kind_value),
            Kind::Identifier => TokenValue::Str(kind_value),
            Kind::StringLiteral
            | Kind::FStringStart
            | Kind::FStringMiddle
            | Kind::FStringEnd
            | Kind::RawBytes
            | Kind::RawFStringStart
            | Kind::Bytes
            | Kind::Unicode
            | Kind::Comment => TokenValue::Str(kind_value),
            Kind::Dedent => {
                let mut spaces_count = 0;
                for c in kind_value.chars() {
                    match c {
                        '\t' => {
                            spaces_count += 4;
                        }
                        ' ' => {
                            spaces_count += 1;
                        }
                        _ => {
                            break;
                        }
                    }
                }
                let mut de_indents = 0;
                while let Some(top) = self.indent_stack.last() {
                    match top.cmp(&spaces_count) {
                        Ordering::Greater => {
                            self.indent_stack.pop();
                            de_indents += 1;
                        }
                        Ordering::Equal => break,
                        // We only see a Kind::Dedent when the indentation level is less than the
                        // top of the stack. So this should never happen and if it happens it's a
                        // bug in code not an error for the user
                        Ordering::Less => {
                            unreachable!()
                        }
                    }
                }
                if de_indents != 1 {
                    // minus 1 because the dedent with actual Indent value is already added
                    // This is super hacky and I don't like it
                    self.next_token_is_dedent += de_indents - 1;
                }
                TokenValue::Indent(de_indents.into())
            }
            Kind::Indent => TokenValue::Indent(1),
            Kind::Error => TokenValue::Str(kind_value),
            _ => TokenValue::None,
        }
    }

    fn create_f_string_start(&mut self, str_start: char) -> String {
        let mut start_of_fstring = String::from(str_start);
        if self.peek() == Some(str_start) && self.double_peek() == Some(str_start) {
            start_of_fstring.push(str_start);
            start_of_fstring.push(str_start);
            self.double_next();
        }
        start_of_fstring
    }
}

fn match_whitespace(c: char) -> bool {
    c.is_whitespace() && c != '\n' && c != '\r'
}

#[cfg(test)]
mod tests {
    use std::fs;

    use insta::{assert_debug_snapshot, glob};

    use super::{Kind, Lexer};
    use crate::error::LexError;

    fn snapshot_test_lexer(snap_name: &str, inputs: &[&str]) -> Result<(), LexError> {
        for (i, test_input) in inputs.iter().enumerate() {
            let mut lexer = Lexer::new(test_input);
            let mut tokens = vec![];
            loop {
                let token = lexer.next_token();
                if token.kind == Kind::Eof {
                    break;
                }
                tokens.push(token);
            }
            let snap_file_name = format!("{}-{}", snap_name, i);
            insta::with_settings!({
                description => test_input.to_string(), // the template source code
                omit_expression => true // do not include the default expression
            }, {
                    assert_debug_snapshot!(snap_file_name, tokens);
            });
        }
        Ok(())
    }

    #[test]
    fn test_lexer() {
        snapshot_test_lexer(
            "test-lexer",
            &[
                "1+2",
                "a+b",
                "a + b",
                "+=2",
                "xX = 2",
                "if else elif",
                "()",
                "[]",
                "{}:",
                ".",
                ",",
                ";",
                "@",
                "=",
                "\\",
                "#",
                "$",
                "?",
                "`",
                "->",
                "+=",
                "-=",
                "*=",
                "/=",
                "%=",
                "@=",
                "&=",
                "|=",
                "^=",
                "//=",
                "<<=",
                ">>=",
                "**=",
                "**",
                "//",
                "<<",
                ">>",
                "+",
                "-",
                "*",
                "**",
                "/",
                "//",
                "%",
                "@",
                "<<",
                ">>",
                "&",
                "|",
                "^",
                "~",
                ":=",
                "<",
                ">",
                "<=",
                ">=",
                "==",
                "!=",
            ],
        )
        .unwrap();

        // keywords
        snapshot_test_lexer(
            "keywords",
            &[
                "False None True and as assert async await",
                "break class continue def del elif else except",
                "finally for from global if import in is lambda",
                "nonlocal not or pass raise return try while with yield",
            ],
        )
        .unwrap();

        // Test identifiers
        snapshot_test_lexer(
            "identifiers",
            &["a", "a_a", "_a", "a_", "a_a_a", "a_a_", "ಠ_ಠ"],
        )
        .unwrap();
        // Invalid identifiers
        snapshot_test_lexer("invalid-identifiers", &["🦀"]).unwrap();

        // Test numeric literals
        // Binary
        snapshot_test_lexer(
            "numeric-binary",
            &[
                "0b0", "0b1", "0b10", "0b11", "0b100", "0b101", "0b110", "0b111",
            ],
        )
        .unwrap();

        // Octal
        snapshot_test_lexer(
            "numeric-octal",
            &["0o0", "0o1", "0o2", "0o3", "0o4", "0o5", "0o6", "0o7"],
        )
        .unwrap();

        // Hexadecimal
        snapshot_test_lexer(
            "numeric-hexadecimal",
            &[
                "0x0", "0x1", "0x2", "0x3", "0x4", "0x5", "0x6", "0x7", "0x8", "0x9", "0xa", "0xb",
                "0xc", "0xd", "0xe", "0xf", "0xA", "0xB", "0xC", "0xD", "0xE", "0xF",
            ],
        )
        .unwrap();
        // Point float
        snapshot_test_lexer("numeric-floating-point", &["0.0 0.1 00.0 00.1 0.1j 0.01J"]).unwrap();

        // Exponent float
        snapshot_test_lexer("numeric-floating-exponent", &["0e0 0e-1 0e+2 0e+3j 0e+3J"]).unwrap();

        // Integer
        snapshot_test_lexer("numeric-integer", &["11 33 1j 1_000_000j"]).unwrap();

        // Strings
        snapshot_test_lexer(
            "string-literals",
            &[
                "\"hello\"  ",
                "\"world\"",
                "\"\"",
                "a = \"hello\"",
                "'hello'",
                "\"\"\"hello\"\"\"",
                "'''hello'''",
            ],
        )
        .unwrap();

        // Bytes
        snapshot_test_lexer(
            "bytes-literals",
            &[
                "b\"hello\"",
                "b\"world\"",
                "b\"\"",
                "a = b\"hello\"",
                "b'hello'",
                "b\"\"\"hello\"\"\"",
                "b'''hello'''",
            ],
        )
        .unwrap();

        // Raw strings
        snapshot_test_lexer(
            "raw-string-literals",
            &[
                "r\"hello\"",
                "r\"world\"",
                "r\"\"",
                "a = r\"hello\"",
                "r'hello'",
                "r\"\"\"hello\"\"\"",
                "r'''hello'''",
            ],
        )
        .unwrap();

        // Raw bytes
        snapshot_test_lexer(
            "raw-bytes-literals",
            &[
                "rb\"hello\"",
                "rb\"world\"",
                "rb\"\"",
                "a = rb\"hello\"",
                "rb'hello'",
                "rb\"\"\"hello\"\"\"",
                "rb'''hello'''",
            ],
        )
        .unwrap();

        // Unicode strings
        snapshot_test_lexer(
            "unicode-string-literals",
            &[
                "u\"hello\"",
                "u\"world\"",
                "u\"\"",
                "a = u\"hello\"",
                "u'hello'",
                "u\"\"\"hello\"\"\"",
                "u'''hello'''",
            ],
        )
        .unwrap();

        snapshot_test_lexer(
            "import",
            &["import a", "import a.b", "import a.b.c", "import a from b"],
        )
        .unwrap();

        snapshot_test_lexer(
            "newline-in-nested-structure",
            &["(a,

)"],
        )
        .unwrap();
    }

    #[test]
    fn test_indentation() {
        // Indentation
        snapshot_test_lexer(
            "indentation",
            &[
                "if True:
            pass\n",
                "if True:
    pass
else:
    pass",
                "if True:
    if True:
        pass
def",
                "def f(x):
    y = z

    print(y)
",
                "if a:

    f = c

    # Path: test_local.py	
",
            ],
        )
        .unwrap();
    }

    #[test]
    fn test_fstring() {
        // F-strings
        snapshot_test_lexer(
            "f-string-literals",
            &[
                "f\"hello\"",
                "f'hello_{var}'",
                "f\"world\"",
                "f\"\"",
                "a = f\"hello\"",
                "f\"\"\"hello\"\"\"",
                "f'''hello'''",
                "f\"{{hey}}\"",
                "f\"oh_{{hey}}\"",
                "f'a' 'c'",
                // unsupported
                // "f'hello_{f'''{a}'''}'",
            ],
        )
        .unwrap();

        // Raw F-strings
        snapshot_test_lexer(
            "raw-f-string-literals",
            &[
                "rf\"hello\"",
                "rf\"world\"",
                "rf\"\"",
                "a = rf\"hello\"",
                "rf'hello_{var}'",
                "rf\"\"\"hello\"\"\"",
                "rf'''hello'''",
            ],
        )
        .unwrap();
    }

    #[test]
    fn test_ellipsis() {
        snapshot_test_lexer(
            "ellipsis",
            &[
                "...",
                "def a():
    ...",
            ],
        )
        .unwrap();
    }

    #[test]
    fn test_unterminated_string_double_quotes() {
        snapshot_test_lexer(
            "unterminated-string",
            &["\"hello", "'hello", "'''hello''", "'''hello'"],
        )
        .unwrap();
    }

    fn snapshot_test_lexer_and_errors(test_case: &str) {
        let mut lexer = Lexer::new(test_case);
        let mut tokens = vec![];
        loop {
            let token = lexer.next_token();
            if token.kind == Kind::Eof {
                break;
            }
            tokens.push(token);
        }
        insta::with_settings!({
            description => test_case.to_string(), // the template source code
            omit_expression => true // do not include the default expression
        }, {
                assert_debug_snapshot!(tokens);
        });
    }

    #[test]
    fn test_complete() {
        glob!("../../test_data", "inputs/*.py", |path| {
            let test_case = fs::read_to_string(path).unwrap();
            snapshot_test_lexer_and_errors(&test_case);
        })
    }

    #[test]
    fn test_one_liners() {
        glob!("../../test_data", "inputs/one_liners/*.py", |path| {
            let input = fs::read_to_string(path).unwrap();
            for test_case in input.split("\n\n") {
                snapshot_test_lexer_and_errors(test_case);
            }
        });
    }
}
