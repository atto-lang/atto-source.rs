//! A deliberately bounded Oreslang parser for the shared v1 scalar conformance surface.
//! No implicit support for actors, async, heap ownership, or polymorphic calls.
use logos::Logos;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub functions: Vec<Function>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub exported: bool,
    pub params: Vec<String>,
    pub locals: Vec<Local>,
    pub expression: Expr,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Local {
    pub name: String,
    pub value: Expr,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Integer(i64),
    Name(String),
    Call { name: String, args: Vec<Expr> },
    Add(Box<Expr>, Box<Expr>),
    Subtract(Box<Expr>, Box<Expr>),
    Multiply(Box<Expr>, Box<Expr>),
    Negate(Box<Expr>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub offset: usize,
    pub class: &'static str,
    pub message: String,
}
impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at byte {}: {}",
            self.class, self.offset, self.message
        )
    }
}
impl std::error::Error for ParseError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Logos)]
enum Kind {
    #[token("pub")]
    Pub,
    #[token("define")]
    Define,
    #[token("static")]
    Static,
    #[token("fnc")]
    Fnc,
    #[token("return")]
    Return,
    #[token("const")]
    Const,
    #[token("val")]
    Val,
    #[token("int")]
    Int,
    #[token("as")]
    As,
    #[token("end")]
    End,
    #[token("->")]
    Arrow,
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token(";")]
    Semicolon,
    #[token(":")]
    Colon,
    #[token("=")]
    Equals,
    #[token(",")]
    Comma,
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[regex(r"[0-9][0-9_]*")]
    Number,
    #[regex(r"[A-Za-z_][A-Za-z0-9_]*")]
    Identifier,
    #[regex(r"[ \t\n\r\f]+", logos::skip)]
    #[regex(r"//[^\r\n]*", logos::skip, allow_greedy = true)]
    Whitespace,
}
#[derive(Clone, Debug)]
struct Token {
    kind: Kind,
    text: String,
    start: usize,
}
/// Strip nested block comments without moving token offsets. Unsupported Unicode
/// inside a comment becomes ASCII whitespace; outside stays byte-identical.
fn without_block_comments(source: &str) -> Result<String, ParseError> {
    let mut bytes = source.as_bytes().to_vec();
    let src = source.as_bytes();
    let mut i = 0;
    let mut depth = 0_u32;
    let mut open = 0_usize;
    while i < src.len() {
        // A block opener inside a // line comment is not a block comment.
        // Leave the line untouched for Logos to skip; preserve source offsets.
        if depth == 0 && i + 1 < src.len() && src[i] == b'/' && src[i + 1] == b'/' {
            while i < src.len() && src[i] != b'\n' && src[i] != b'\r' {
                i += 1;
            }
        } else if i + 1 < src.len() && src[i] == b'/' && src[i + 1] == b'*' {
            if depth == 0 {
                open = i;
            }
            depth += 1;
            bytes[i] = b' ';
            bytes[i + 1] = b' ';
            i += 2;
        } else if i + 1 < src.len() && src[i] == b'*' && src[i + 1] == b'/' && depth > 0 {
            depth -= 1;
            bytes[i] = b' ';
            bytes[i + 1] = b' ';
            i += 2;
        } else {
            if depth > 0 && src[i] != b'\n' && src[i] != b'\r' {
                bytes[i] = b' ';
            }
            i += 1;
        }
    }
    if depth != 0 {
        return Err(ParseError {
            offset: open,
            class: "unterminated_comment",
            message: "nested /* block comment is not closed".into(),
        });
    }
    String::from_utf8(bytes).map_err(|_| ParseError {
        offset: 0,
        class: "syntax_error",
        message: "comment preprocessing failed".into(),
    })
}

// Only a single underscore between decimal digits is admitted.
// Without this check, tokens such as 1__0 and 1_ silently normalize to 10 and
// 1, allowing malformed source to compile with a different spelling.
fn normalized_integer_digits(token: &Token) -> Result<String, ParseError> {
    let bytes = token.text.as_bytes();
    for index in 0..bytes.len() {
        if bytes[index] == b'_'
            && (index == 0
                || index + 1 == bytes.len()
                || !bytes[index - 1].is_ascii_digit()
                || !bytes[index + 1].is_ascii_digit())
        {
            return Err(ParseError {
                offset: token.start + index,
                class: "syntax_error",
                message: "invalid integer digit separator".into(),
            });
        }
    }
    Ok(token.text.replace('_', ""))
}

pub const MAX_SOURCE_BYTES: usize = 256 * 1024;
pub const MAX_TOKENS: usize = 2048;
pub const MAX_PAREN_NESTING: usize = 64;

/// Execution-domain words reserved by every Oreslang backend (Java and LLVM
/// reject them as identifiers too).
pub const EXECUTION_DOMAIN_WORDS: [&str; 3] = ["cpu", "gpu", "quantum"];

/// This compiler lowers no cpu/gpu/quantum region or CPU/GPU Task. Reject them
/// with a dedicated diagnostic before parsing, never an incidental syntax
/// error that a later grammar extension could turn into host execution.
fn reject_execution_domains(tokens: &[Token]) -> Result<(), ParseError> {
    for (index, token) in tokens.iter().enumerate() {
        if token.kind != Kind::Identifier {
            continue;
        }
        let opens_block = tokens.get(index + 1).map(|next| next.kind) == Some(Kind::LBrace);
        let message = if EXECUTION_DOMAIN_WORDS.contains(&token.text.as_str()) {
            format!(
                "execution domain '{}' is reserved and not lowered by this backend; no implicit CPU fallback",
                token.text
            )
        } else if opens_block && (token.text == "task" || token.text == "nb") {
            "execution task requires scheduler-aware Future lowering; no implicit inline execution"
                .into()
        } else {
            continue;
        };
        return Err(ParseError {
            offset: token.start,
            class: "execution_domain_unsupported",
            message,
        });
    }
    Ok(())
}

pub fn parse(source: &str) -> Result<Program, ParseError> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(ParseError {
            offset: MAX_SOURCE_BYTES,
            class: "resource_limit",
            message: "Oreslang source exceeds 256 KiB prototype limit".into(),
        });
    }
    // Trojan Source controls can reorder displayed text even when hidden in
    // ignored comments or string literals. Reject prior to trivia stripping.
    if let Some((offset, _)) = source.char_indices().find(|(_, c)|
        matches!(*c, '\u{061c}' | '\u{200e}' | '\u{200f}' |
            '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')) {
        return Err(ParseError {
            offset,
            class: "unsafe_unicode",
            message: "bidirectional Unicode formatting controls are forbidden".into(),
        });
    }
    let prepared = without_block_comments(source)?;
    let mut tokens = Vec::new();
    let mut paren_depth = 0_usize;
    for (kind, range) in Kind::lexer(&prepared).spanned() {
        let kind = kind.map_err(|_| ParseError {
            offset: range.start,
            class: "syntax_error",
            message: "unsupported token".into(),
        })?;
        if tokens.len() >= MAX_TOKENS {
            return Err(ParseError {
                offset: range.start,
                class: "resource_limit",
                message: "token count exceeds prototype limit".into(),
            });
        }
        match kind {
            Kind::LParen => {
                paren_depth += 1;
                if paren_depth > MAX_PAREN_NESTING {
                    return Err(ParseError {
                        offset: range.start,
                        class: "resource_limit",
                        message: "parenthesis depth exceeds prototype limit".into(),
                    });
                }
            }
            Kind::RParen => {
                paren_depth = paren_depth.saturating_sub(1);
            }
            _ => {}
        }
        tokens.push(Token {
            kind,
            text: prepared[range.clone()].to_owned(),
            start: range.start,
        });
    }
    reject_execution_domains(&tokens)?;
    let mut parser = Parser {
        tokens,
        cursor: 0,
        source_len: source.len(),
    };
    let mut functions = Vec::new();
    while parser.peek().is_some() {
        functions.push(parser.function()?);
    }
    if functions.is_empty() {
        return Err(ParseError {
            offset: 0,
            class: "syntax_error",
            message: "expected a function declaration".into(),
        });
    }
    Ok(Program { functions })
}
struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
    source_len: usize,
}
impl Parser {
    fn peek(&self) -> Option<Kind> {
        self.tokens.get(self.cursor).map(|t| t.kind)
    }
    fn pos(&self) -> usize {
        self.tokens
            .get(self.cursor)
            .map_or(self.source_len, |t| t.start)
    }
    fn expect(&mut self, kind: Kind) -> Result<Token, ParseError> {
        if self.peek() != Some(kind) {
            return Err(ParseError {
                offset: self.pos(),
                class: "syntax_error",
                message: format!("expected {:?}, got {:?}", kind, self.peek()),
            });
        }
        let token = self.tokens[self.cursor].clone();
        self.cursor += 1;
        Ok(token)
    }
    fn function(&mut self) -> Result<Function, ParseError> {
        // Canonical AttoLang permits both "define pub fnc" and
        // "pub define fnc"; the declaration still must be a scalar fnc.
        let leading_define = self.peek() == Some(Kind::Define);
        if leading_define {
            self.expect(Kind::Define)?;
        }
        let exported = self.peek() == Some(Kind::Pub);
        if exported {
            self.expect(Kind::Pub)?;
        }
        if !leading_define && self.peek() == Some(Kind::Define) {
            self.expect(Kind::Define)?;
        }
        if self.peek() == Some(Kind::Static) {
            self.expect(Kind::Static)?;
        }
        self.expect(Kind::Fnc)?;
        let name = self.expect(Kind::Identifier)?.text;
        self.expect(Kind::LParen)?;
        let mut params = Vec::new();
        if self.peek() != Some(Kind::RParen) {
            loop {
                self.expect(Kind::Int)?;
                params.push(self.expect(Kind::Identifier)?.text);
                if self.peek() == Some(Kind::Comma) {
                    self.expect(Kind::Comma)?;
                } else {
                    break;
                }
            }
        }
        self.expect(Kind::RParen)?;
        let uses_end = match self.peek() {
            Some(Kind::Arrow) | Some(Kind::Colon) => {
                if self.peek() == Some(Kind::Arrow) {
                    self.expect(Kind::Arrow)?;
                } else {
                    self.expect(Kind::Colon)?;
                }
                self.expect(Kind::Int)?;
                self.expect(Kind::LBrace)?;
                false
            }
            Some(Kind::As) => {
                self.expect(Kind::As)?;
                true
            }
            _ => {
                return Err(ParseError {
                    offset: self.pos(),
                    class: "syntax_error",
                    message: "expected ': int {', '-> int {' or 'as'".into(),
                })
            }
        };
        let mut locals = Vec::new();
        // Canonical scalar locals are immutable by default. All admitted
        // spellings lower to the same typed local; this does not admit mut/let
        // rebinding or infer any non-integer type.
        while matches!(self.peek(), Some(Kind::Const | Kind::Val | Kind::Int)) {
            if self.peek() == Some(Kind::Const) {
                self.expect(Kind::Const)?;
            } else if self.peek() == Some(Kind::Val) {
                self.expect(Kind::Val)?;
            }
            let local_name = if self.peek() == Some(Kind::Int) {
                self.expect(Kind::Int)?;
                self.expect(Kind::Identifier)?.text
            } else {
                let name = self.expect(Kind::Identifier)?.text;
                if self.peek() == Some(Kind::Colon) {
                    self.expect(Kind::Colon)?;
                    self.expect(Kind::Int)?;
                }
                name
            };
            self.expect(Kind::Equals)?;
            let value = self.expression()?;
            self.expect(Kind::Semicolon)?;
            locals.push(Local {
                name: local_name,
                value,
            });
        }
        // A source-level immutable binding must never silently turn into a
        // write; the Rust scalar compiler does not yet lower let/mut.
        if let (Some(name), Some(op)) = (
            self.tokens.get(self.cursor),
            self.tokens.get(self.cursor + 1),
        ) {
            let is_bound = name.kind == Kind::Identifier
                && (params.iter().any(|param| param == &name.text)
                    || locals.iter().any(|local| local.name == name.text));
            let compound_rhs = self.tokens.get(self.cursor + 2).map(|token| token.kind);
            let is_assignment = op.kind == Kind::Equals
                || (matches!(op.kind, Kind::Plus | Kind::Minus)
                    && compound_rhs == Some(Kind::Equals));
            if is_bound && is_assignment {
                return Err(ParseError {
                    offset: name.start,
                    class: "immutable_binding",
                    message: "immutable binding cannot be reassigned; let/mut unsupported".into(),
                });
            }
        }
        self.expect(Kind::Return)?;
        let expression = self.expression()?;
        self.expect(Kind::Semicolon)?;
        if uses_end {
            self.expect(Kind::End)?;
        } else {
            self.expect(Kind::RBrace)?;
        }
        Ok(Function {
            name,
            exported,
            params,
            locals,
            expression,
        })
    }
    fn expression(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.product()?;
        loop {
            lhs = match self.peek() {
                Some(Kind::Plus) => {
                    self.expect(Kind::Plus)?;
                    Expr::Add(Box::new(lhs), Box::new(self.product()?))
                }
                Some(Kind::Minus) => {
                    self.expect(Kind::Minus)?;
                    Expr::Subtract(Box::new(lhs), Box::new(self.product()?))
                }
                _ => break,
            };
        }
        Ok(lhs)
    }
    fn product(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.primary()?;
        while self.peek() == Some(Kind::Star) {
            self.expect(Kind::Star)?;
            lhs = Expr::Multiply(Box::new(lhs), Box::new(self.primary()?));
        }
        Ok(lhs)
    }
    fn primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek() {
            Some(Kind::Number) => {
                let t = self.expect(Kind::Number)?;
                let integer = normalized_integer_digits(&t)?
                    .parse::<i64>()
                    .map_err(|_| ParseError {
                        offset: t.start,
                        class: "integer_overflow",
                        message: "i64 integer literal overflow".into(),
                    })?;
                Ok(Expr::Integer(integer))
            }
            Some(Kind::Identifier) => {
                let name = self.expect(Kind::Identifier)?.text;
                if self.peek() != Some(Kind::LParen) {
                    return Ok(Expr::Name(name));
                }
                self.expect(Kind::LParen)?;
                let mut args = Vec::new();
                if self.peek() != Some(Kind::RParen) {
                    loop {
                        args.push(self.expression()?);
                        if self.peek() == Some(Kind::Comma) {
                            self.expect(Kind::Comma)?;
                        } else {
                            break;
                        }
                    }
                }
                self.expect(Kind::RParen)?;
                Ok(Expr::Call { name, args })
            }
            Some(Kind::Minus) => {
                let minus = self.expect(Kind::Minus)?;
                // The signed i64 minimum is valid even though its positive
                // magnitude is outside i64.
                if self.peek() == Some(Kind::Number) {
                    let token = self.tokens[self.cursor].clone();
                    let magnitude = normalized_integer_digits(&token)?;
                    if magnitude == "9223372036854775808" {
                        self.expect(Kind::Number)?;
                        return Ok(Expr::Integer(i64::MIN));
                    }
                }
                let expr = self.primary()?;
                if let Expr::Integer(value) = expr {
                    return value.checked_neg().map(Expr::Integer).ok_or(ParseError {
                        offset: minus.start,
                        class: "integer_overflow",
                        message: "negated literal cannot fit in i64".into(),
                    });
                }
                Ok(Expr::Negate(Box::new(expr)))
            }
            Some(Kind::LParen) => {
                self.expect(Kind::LParen)?;
                let value = self.expression()?;
                self.expect(Kind::RParen)?;
                Ok(value)
            }
            _ => Err(ParseError {
                offset: self.pos(),
                class: "syntax_error",
                message: "expected integer, name, call, or parenthesized expression".into(),
            }),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_excessive_parenthesis_nesting_before_recursive_descent() {
        let expression = format!(
            "{}1{}",
            "(".repeat(MAX_PAREN_NESTING + 1),
            ")".repeat(MAX_PAREN_NESTING + 1)
        );
        let src = format!("fnc f(): int {{ return {expression}; }}");
        assert_eq!(parse(&src).unwrap_err().class, "resource_limit");
    }

    #[test]
    fn rejects_excessive_tokens_before_deep_ast_construction() {
        let expression = "1+".repeat(MAX_TOKENS) + "1";
        let src = format!("fnc f(): int {{ return {expression}; }}");
        assert_eq!(parse(&src).unwrap_err().class, "resource_limit");
    }

    #[test]
    fn rejects_oversized_source_before_preprocessing_comments() {
        let src = " ".repeat(MAX_SOURCE_BYTES + 1);
        assert_eq!(parse(&src).unwrap_err().class, "resource_limit");
    }

    #[test]
    fn canonical_function_call_and_underscore() {
        let ast = parse("pub fnc main(): int { return add(4_0, 2); }\nfnc add(int a, int b): int { return a+b; }").unwrap();
        assert_eq!(ast.functions[0].name, "main");
        assert_eq!(ast.functions[1].params, vec!["a", "b"]);
        assert!(matches!(ast.functions[0].expression, Expr::Call { .. }));
    }
    #[test]
    fn precedence_and_legacy_end() {
        let ast = parse("static fnc answer() as return 1 + 2 * 3; end").unwrap();
        assert!(matches!(ast.functions[0].expression, Expr::Add(..)));
    }
    #[test]
    fn nested_comments_and_minimum() {
        let ast = parse(
            "/* outer /* inner */ end */ pub fnc main(): int { return -9223372036854775808; }",
        )
        .unwrap();
        assert_eq!(ast.functions[0].expression, Expr::Integer(i64::MIN));
    }
    #[test]
    fn unclosed_comment_reports_class() {
        let err = parse("pub fnc main(): int { return 42; } /* unclosed").unwrap_err();
        assert_eq!(err.class, "unterminated_comment");
    }
    #[test]
    fn overflow_reports_class() {
        let err = parse("pub fnc main(): int { return 9223372036854775808; }").unwrap_err();
        assert_eq!(err.class, "integer_overflow");
    }
    #[test]
    fn const_locals() {
        let ast =
            parse("fnc owned(int owned): int { const local: int = owned; return local; }").unwrap();
        assert_eq!(ast.functions[0].locals.len(), 1);
    }
    #[test]
    fn ignores_double_slash_comment() {
        assert!(parse("// first\npub fnc main(): int { return 40+2; }").is_ok());
    }

    #[test]
    fn line_comment_block_opener_does_not_swallow_source() {
        let source = "// /* ignored until newline\npub fnc main(): int { return 42; }";
        let parsed = parse(source).expect("block opener inside // must be inert");
        assert_eq!(parsed.functions.len(), 1);
        assert_eq!(parsed.functions[0].expression, Expr::Integer(42));
    }

    #[test]
    fn line_comment_does_not_hide_real_unterminated_block() {
        let source = "// /* inert\n/* real unterminated block";
        let error = parse(source).expect_err("real unclosed block must still fail");
        assert_eq!(error.class, "unterminated_comment");
        assert_eq!(error.offset, source.find("\n/*").unwrap() + 1);
    }

    #[test]
    fn block_comment_does_not_turn_nested_line_markers_into_line_comments() {
        let source = "/* // /* nested */ outer */ pub fnc main(): int { return 42; }";
        let parsed = parse(source).expect("nested blocks remain valid");
        assert_eq!(parsed.functions.len(), 1);
        assert_eq!(parsed.functions[0].expression, Expr::Integer(42));
    }

    #[test]
    fn carriage_return_line_comment_is_opaque_to_block_scanner() {
        let source = "// /* not a block opener\rpub fnc main(): int { return 42; }";
        let parsed = parse(source).expect("CR terminates a line comment");
        assert_eq!(parsed.functions[0].expression, Expr::Integer(42));
    }
}
