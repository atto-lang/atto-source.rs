//! Lexer and parser for the validated initial scalar function subset.
//! Intended to be replaced by a full-fidelity Rowan CST + typed HIR.
use logos::Logos;
use std::fmt;
pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;
const MAX_TOKENS: usize = 4096;
const MAX_EXPR_DEPTH: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program { pub functions: Vec<Function> }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Function { pub name: String, pub expression: Expr }
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Integer(i64),
    Add(Box<Expr>, Box<Expr>),
    Subtract(Box<Expr>, Box<Expr>),
    Multiply(Box<Expr>, Box<Expr>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError { pub offset: usize, pub message: String }
impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "byte {}: {}", self.offset, self.message)
    }
}
impl std::error::Error for ParseError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Logos)]
enum Kind {
    #[token("static")] Static,
    #[token("fnc")] Fnc,
    #[token("return")] Return,
    #[token("int")] Int,
    #[token("as")] As,
    #[token("end")] End,
    #[token("->")] Arrow,
    #[token("(")] LParen,
    #[token(")")] RParen,
    #[token("{")] LBrace,
    #[token("}")] RBrace,
    #[token(";")] Semicolon,
    #[token("+")] Plus,
    #[token("-")] Minus,
    #[token("*")] Star,
    #[regex("[0-9]+")] Number,
    #[regex("[A-Za-z_][A-Za-z0-9_]*")] Identifier,
    #[regex(r"[ \t\n\r\f]+", logos::skip)]
    #[regex(r"//[^\n]*", logos::skip)]
    Whitespace,
}
#[derive(Clone, Debug)]
struct Token { kind: Kind, text: String, start: usize }
pub fn parse(source: &str) -> Result<Program, ParseError> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(ParseError { offset: MAX_SOURCE_BYTES, message: "source exceeds byte budget".into() });
    }
    let mut tokens = Vec::new();
    for (kind, range) in Kind::lexer(source).spanned() {
        let kind = kind.map_err(|_| ParseError {
            offset: range.start, message: "unsupported token".into()
        })?;
        if tokens.len() >= MAX_TOKENS {
            return Err(ParseError { offset: range.start, message: "token budget exceeded".into() });
        }
        tokens.push(Token { kind, text: source[range.clone()].to_owned(), start: range.start });
    }
    let mut parser = Parser { tokens, cursor: 0, source_len: source.len() };
    let mut functions = Vec::new();
    while parser.peek().is_some() { functions.push(parser.function()?); }
    if functions.is_empty() {
        return Err(ParseError { offset: 0, message: "expected a static fnc".into() });
    }
    Ok(Program { functions })
}
struct Parser { tokens: Vec<Token>, cursor: usize, source_len: usize }
impl Parser {
    fn peek(&self) -> Option<Kind> { self.tokens.get(self.cursor).map(|t| t.kind) }
    fn pos(&self) -> usize { self.tokens.get(self.cursor).map_or(self.source_len, |t| t.start) }
    fn expect(&mut self, kind: Kind) -> Result<Token, ParseError> {
        if self.peek() != Some(kind) {
            return Err(ParseError { offset: self.pos(),
                message: format!("expected {:?}, got {:?}", kind, self.peek()) });
        }
        let token = self.tokens[self.cursor].clone();
        self.cursor += 1;
        Ok(token)
    }
    fn function(&mut self) -> Result<Function, ParseError> {
        self.expect(Kind::Static)?;
        self.expect(Kind::Fnc)?;
        let name = self.expect(Kind::Identifier)?.text;
        self.expect(Kind::LParen)?;
        self.expect(Kind::RParen)?;
        let uses_end = match self.peek() {
            Some(Kind::Arrow) => {
                self.expect(Kind::Arrow)?;
                self.expect(Kind::Int)?;
                self.expect(Kind::LBrace)?;
                false
            }
            Some(Kind::As) => { self.expect(Kind::As)?; true }
            _ => return Err(ParseError { offset: self.pos(),
                message: "expected '-> int {' or 'as'".into() }),
        };
        self.expect(Kind::Return)?;
        let expression = self.expression()?;
        self.expect(Kind::Semicolon)?;
        if uses_end { self.expect(Kind::End)?; }
        else { self.expect(Kind::RBrace)?; }
        Ok(Function { name, expression })
    }
    fn expression(&mut self) -> Result<Expr, ParseError> { self.expression_at(0) }
    fn expression_at(&mut self, depth: usize) -> Result<Expr, ParseError> {
        self.check_depth(depth)?;
        let mut lhs = self.product_at(depth + 1)?;
        loop {
            lhs = match self.peek() {
                Some(Kind::Plus) => {
                    self.expect(Kind::Plus)?;
                    Expr::Add(Box::new(lhs), Box::new(self.product_at(depth + 1)?))
                }
                Some(Kind::Minus) => {
                    self.expect(Kind::Minus)?;
                    Expr::Subtract(Box::new(lhs), Box::new(self.product_at(depth + 1)?))
                }
                _ => break,
            };
        }
        Ok(lhs)
    }
    fn product_at(&mut self, depth: usize) -> Result<Expr, ParseError> {
        self.check_depth(depth)?;
        let mut lhs = self.primary_at(depth + 1)?;
        while self.peek() == Some(Kind::Star) {
            self.expect(Kind::Star)?;
            lhs = Expr::Multiply(Box::new(lhs), Box::new(self.primary_at(depth + 1)?));
        }
        Ok(lhs)
    }
    fn check_depth(&self, depth: usize) -> Result<(), ParseError> {
        if depth > MAX_EXPR_DEPTH {
            return Err(ParseError { offset: self.pos(), message: "expression nesting budget exceeded".into() });
        }
        Ok(())
    }
    fn primary_at(&mut self, depth: usize) -> Result<Expr, ParseError> {
        self.check_depth(depth)?;
        match self.peek() {
            Some(Kind::Number) => {
                let token = self.expect(Kind::Number)?;
                let number = token.text.parse::<i64>().map_err(|_| ParseError {
                    offset: token.start, message: "integer exceeds i64 range".into()
                })?;
                Ok(Expr::Integer(number))
            }
            Some(Kind::Minus) => {
                self.expect(Kind::Minus)?;
                Ok(Expr::Subtract(Box::new(Expr::Integer(0)), Box::new(self.primary_at(depth + 1)?)))
            }
            Some(Kind::LParen) => {
                self.expect(Kind::LParen)?;
                let expression = self.expression_at(depth + 1)?;
                self.expect(Kind::RParen)?;
                Ok(expression)
            }
            _ => Err(ParseError { offset: self.pos(),
                message: "expected integer or parenthesized expression".into() }),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precedence() {
        let ast = parse("static fnc answer() as return 1 + 2 * 3; end").unwrap();
        assert_eq!(ast.functions[0].expression,
            Expr::Add(Box::new(Expr::Integer(1)), Box::new(Expr::Multiply(
                Box::new(Expr::Integer(2)), Box::new(Expr::Integer(3))))));
    }
    #[test]
    fn comments_and_parentheses() {
        assert!(parse("// comment\nstatic fnc answer() -> int { return (40 + 2); }").is_ok());
    }
    #[test]
    fn rejects_trailing() {
        assert!(parse("static fnc a() as return 2; end garbage").is_err());
    }
    #[test]
    fn rejects_deep_parentheses_without_stack_exhaustion() {
        let source = format!("static fnc f() as return {}1{}; end",
            "(".repeat(MAX_EXPR_DEPTH + 5), ")".repeat(MAX_EXPR_DEPTH + 5));
        assert!(parse(&source).is_err());
    }
    #[test]
    fn rejects_excessive_tokens() {
        let source = format!("static fnc f() as return 1{}; end", "+1".repeat(MAX_TOKENS));
        assert!(parse(&source).is_err());
    }
    #[test]
    fn rejects_integer_out_of_range() {
        assert!(parse("static fnc a() as return 9223372036854775808; end").is_err());
    }
}
