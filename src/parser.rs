//! Recursive-descent parser for the browser AttoLang subset.
//!
//! The grammar follows `docs/grammar.ebnf` (Oreslang v0.6) in the Java
//! reference compiler and spec chapter 16. Constructs outside the admitted
//! subset are recognised and rejected with `E_UNSUPPORTED` rather than being
//! mis-parsed or silently approximated.

use crate::ast::*;
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{Tok, Token};
use std::collections::BTreeMap;

pub const MAX_DEPTH: u32 = 128;
pub const MAX_FUNCS: usize = 1024;
pub const MAX_DECLS: usize = 256;
pub const MAX_VARIANTS: usize = 256;
pub const MAX_PARAMS: usize = 32;
/// Bound on the height of any syntax tree the parser builds. Left-associative
/// operator chains (`a + b + ...`) and `elif` arms are parsed iteratively but
/// produce trees that the checker, emitter and `Drop` walk recursively, so
/// each chain link counts against this budget as well as each nested level.
pub const MAX_TREE_HEIGHT: u32 = 256;

/// Words reserved by the reference lexer (spec chapter 16 keyword table).
const RESERVED: &[&str] = &[
    "define",
    "def",
    "module",
    "namespace",
    "contract",
    "conforms",
    "class",
    "enum",
    "struct",
    "interface",
    "trait",
    "type",
    "types",
    "typeof",
    "abstract",
    "extends",
    "implements",
    "impl",
    "import",
    "from",
    "as",
    "fnc",
    "routine",
    "async",
    "suspends",
    "generator",
    "yield",
    "nlex",
    "return",
    "self",
    "super",
    "static",
    "pub",
    "private",
    "const",
    "val",
    "let",
    "mut",
    "if",
    "then",
    "else",
    "elif",
    "elseif",
    "fi",
    "do",
    "done",
    "end",
    "while",
    "for",
    "of",
    "loop",
    "block",
    "break",
    "continue",
    "switch",
    "match",
    "matches",
    "over",
    "default",
    "try",
    "catch",
    "finally",
    "throw",
    "raise",
    "panic",
    "recover",
    "trap",
    "defer",
    "actor",
    "isoactor",
    "shared",
    "untrusted",
    "spawn",
    "stop",
    "rt",
    "await",
    "nb",
    "cb",
    "select",
    "readch",
    "writech",
    "eq",
    "neq",
    "is",
    "new",
    "true",
    "false",
    "null",
    "void",
    "obj",
    "arr",
    "cpu",
    "gpu",
    "quantum",
];

/// Statement-leading keywords that name real AttoLang features this compiler
/// does not lower yet. They are rejected, never approximated.
const UNSUPPORTED_STMT: &[&str] = &[
    "match",
    "switch",
    "try",
    "select",
    "defer",
    "throw",
    "raise",
    "panic",
    "recover",
    "trap",
    "spawn",
    "await",
    "actor",
    "isoactor",
    "define",
    "import",
    "async",
    "yield",
    "guard",
    "nb",
    "cb",
    "readch",
    "writech",
    "rt",
    "class",
    "struct",
    "module",
    "namespace",
    "interface",
    "trait",
    "type",
    "generator",
    "critical",
    "stop",
];

const UNSUPPORTED_TYPES: &[&str] = &[
    "String",
    "string",
    "float",
    "f32",
    "f64",
    "decimal",
    "complex",
    "complex64",
    "complex128",
    "bigint",
    "i8",
    "i16",
    "i32",
    "i64",
    "u8",
    "u16",
    "u32",
    "u64",
    "uint",
    "Symbol",
    "Option",
    "Result",
    "Array",
    "List",
    "Vector",
    "Slice",
    "Tuple",
    "FixedArray",
    "FixedList",
    "Map",
    "Set",
    "char",
    "byte",
];

pub fn is_reserved(name: &str) -> bool {
    RESERVED.contains(&name)
}

pub struct Parser {
    toks: Vec<Token>,
    i: usize,
    depth: u32,
    /// Height of the tree path being built; see [`MAX_TREE_HEIGHT`].
    height: u32,
    /// Nonzero while parsing an if/while condition head, where `,` and `|` are
    /// boolean AND / OR (spec chapter 16).
    cond: u32,
    enum_names: BTreeMap<String,u16>,
}

type R<T> = Result<T, Diagnostic>;

fn unsupported(pos: Pos, what: &str) -> Diagnostic {
    Diagnostic::new(pos, "E_UNSUPPORTED", format!("{what} is not supported by the browser compiler yet"))
}

impl Parser {
    pub fn new(toks: Vec<Token>) -> Self {
        Parser {
            toks,
            i: 0,
            depth: 0,
            height: 0,
            cond: 0,
            enum_names: BTreeMap::new(),
        }
    }

    fn peek(&self) -> &Tok {
        &self.toks[self.i].tok
    }
    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks[(self.i + n).min(self.toks.len() - 1)].tok
    }
    fn pos(&self) -> Pos {
        self.toks[self.i].pos
    }
    fn advance(&mut self) -> Token {
        let t = self.toks[self.i].clone();
        if self.i < self.toks.len() - 1 {
            self.i += 1;
        }
        t
    }
    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if s == w)
    }
    fn word_at(&self, n: usize) -> Option<&str> {
        match self.peek_at(n) {
            Tok::Ident(s) => Some(s.as_str()),
            _ => None,
        }
    }
    fn eat(&mut self, t: &Tok) -> bool {
        if self.peek() == t {
            self.advance();
            true
        } else {
            false
        }
    }
    fn eat_word(&mut self, w: &str) -> bool {
        if self.is_word(w) {
            self.advance();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, t: Tok, what: &str) -> R<()> {
        if self.eat(&t) {
            Ok(())
        } else {
            Err(self.expected(what))
        }
    }
    fn expect_word(&mut self, w: &str) -> R<()> {
        if self.eat_word(w) {
            Ok(())
        } else {
            Err(self.expected(&format!("'{w}'")))
        }
    }
    fn expected(&self, what: &str) -> Diagnostic {
        let found = describe(self.peek());
        let code = match (what, self.peek()) {
            (_, Tok::Eof) => "E_EOF",
            ("';'", _) => "E_SEMICOLON",
            _ => "E_SYNTAX",
        };
        Diagnostic::new(self.pos(), code, format!("expected {what}, found {found}"))
    }
    fn enter(&mut self) -> R<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Diagnostic::new(
                self.pos(),
                "E_NESTING",
                format!("nesting deeper than {MAX_DEPTH} levels"),
            ));
        }
        self.grow(1)
    }
    fn leave(&mut self) {
        self.depth -= 1;
        self.height -= 1;
    }
    /// Account for `n` more levels of tree height (one chain link or arm).
    fn grow(&mut self, n: u32) -> R<()> {
        self.height += n;
        if self.height > MAX_TREE_HEIGHT {
            return Err(Diagnostic::new(
                self.pos(),
                "E_NESTING",
                format!("expression or elif chain deeper than {MAX_TREE_HEIGHT} levels"),
            ));
        }
        Ok(())
    }

    fn ident(&mut self, what: &str) -> R<(String, Pos)> {
        let pos = self.pos();
        match self.peek().clone() {
            Tok::Ident(s) => {
                if is_reserved(&s) {
                    return Err(Diagnostic::new(
                        pos,
                        "E_RESERVED",
                        format!("'{s}' is a reserved word and cannot be used as {what}"),
                    ));
                }
                self.advance();
                Ok((s, pos))
            }
            _ => Err(self.expected(what)),
        }
    }

    // ---------------------------------------------------------------- types

    fn is_type_word(&self, n: usize) -> bool {
        matches!(self.word_at(n), Some("int" | "bool" | "boolean" | "void"))
            || self.word_at(n).is_some_and(|w| UNSUPPORTED_TYPES.contains(&w))
    }

    fn parse_type(&mut self) -> R<Type> {
        let pos = self.pos();
        let name = match self.peek() {
            Tok::Ident(s) => s.clone(),
            _ => return Err(self.expected("a type")),
        };
        self.advance();
        let ty = match name.as_str() {
            "int" => Type::Int,
            "bool" | "boolean" => Type::Bool,
            "void" => Type::Void,
            n if UNSUPPORTED_TYPES.contains(&n) => {
                return Err(Diagnostic::new(
                    pos,
                    "E_UNSUPPORTED_TYPE",
                    format!("type '{n}' is not supported by the browser compiler yet"),
                ))
            }
            n => match self.enum_names.get(n) { Some(id) => Type::Enum(*id), None => return Err(Diagnostic::new(pos,"E_UNKNOWN_TYPE",format!("unknown type '{n}'"))) },
        };
        if matches!(self.peek(), Tok::Lt | Tok::LBracket | Tok::Question | Tok::Pipe) {
            return Err(unsupported(self.pos(), "generic, optional, union and sequence types"));
        }
        Ok(ty)
    }

    // ------------------------------------------------------------ top level

    pub fn parse_program(&mut self) -> R<Program> {
        let mut funcs=Vec::new();
        let mut decls=Vec::new();
        while *self.peek()!=Tok::Eof {
            if funcs.len()>=MAX_FUNCS {
                return Err(Diagnostic::new(self.pos(),"E_SOURCE_LIMIT",
                    format!("more than {MAX_FUNCS} functions")));
            }
            // Both pub define class and define pub class are accepted.
            let n=usize::from(matches!(self.word_at(0),Some("pub"|"private")));
            let define=self.word_at(n)==Some("define");
            let at=n+1+usize::from(define &&
                matches!(self.word_at(n+1),Some("pub"|"private")));
            let is_decl=define && matches!(self.word_at(at),
                Some("class"|"actor"|"enum"|"struct"|"interface"|"trait"|"module"));
            if is_decl { self.parse_decl(&mut funcs,&mut decls)?; }
            else { funcs.push(self.parse_func()?); }
        }
        Ok(Program{funcs,decls})
    }

    fn parse_decl(&mut self,funcs:&mut Vec<Func>,decls:&mut Vec<Declaration>) -> R<()> {
        let start=self.pos();
        if decls.len()>=MAX_DECLS {
            return Err(Diagnostic::new(start,"E_SOURCE_LIMIT","too many top-level declarations"));
        }
        self.eat_word("pub");
        self.eat_word("private");
        self.expect_word("define")?;
        self.eat_word("pub");
        self.eat_word("private");
        let kind=match self.word_at(0) {
            Some("class")=>DeclKind::Class,
            Some("actor")=>DeclKind::Actor,
            Some("enum")=>DeclKind::Enum,
            Some("struct")=>DeclKind::Struct,
            Some("interface")=>DeclKind::Interface,
            Some("trait")=>DeclKind::Trait,
            Some("module")=>DeclKind::Module,
            _=>return Err(self.expected("a type declaration kind")),
        };
        self.advance();
        let (name,pos)=self.ident("a declaration name")?;
        if decls.iter().any(|d|d.name==name) {
            return Err(Diagnostic::new(pos,"E_DUPLICATE",
                format!("duplicate top-level declaration '{name}'")));
        }
        if matches!(name.as_str(),"int"|"bool"|"void"|"stdio"|"And"|"Or"|"Xor") {
            return Err(Diagnostic::new(pos,"E_RESERVED","a builtin cannot be redeclared"));
        }
        if *self.peek()==Tok::Lt {
            return Err(unsupported(self.pos(),"generic type parameters"));
        }
        if matches!(self.word_at(0),Some("extends"|"implements"|"impl"|"conforms")) {
            return Err(unsupported(self.pos(),"inheritance and interface conformance"));
        }
        let mut variants=Vec::new();
        if kind==DeclKind::Enum {
            self.expect_word("as")?;
            if self.is_word("end") {
                return Err(Diagnostic::new(self.pos(),"E_SYNTAX","enum requires variants"));
            }
            loop {
                if variants.len()>=MAX_VARIANTS {
                    return Err(Diagnostic::new(self.pos(),"E_SOURCE_LIMIT","too many enum variants"));
                }
                let (variant,vpos)=self.ident("an enum variant")?;
                if variants.contains(&variant) {
                    return Err(Diagnostic::new(vpos,"E_DUPLICATE",
                        format!("duplicate enum variant '{name}.{variant}'")));
                }
                variants.push(variant);
                if !self.eat(&Tok::Comma) {break;}
                if self.is_word("end") {break;}
            }
            self.expect_word("end")?;
            self.enum_names.insert(name.clone(),decls.len() as u16);
        } else {
            let braces=self.eat(&Tok::LBrace);
            if !braces {self.expect_word("as")?;}
            loop {
                if (braces && self.eat(&Tok::RBrace)) ||
                   (!braces && self.eat_word("end")) {break;}
                if *self.peek()==Tok::Eof {
                    return Err(self.expected(if braces {"'}'"} else {"'end'"}));
                }
                let static_method=matches!(kind,DeclKind::Class|DeclKind::Actor) &&
                    (self.is_word("static") ||
                    (matches!(self.word_at(0),Some("pub"|"private")) &&
                     self.word_at(1)==Some("static")));
                let module_fn=kind==DeclKind::Module &&
                    (matches!(self.word_at(0),Some("fnc"|"routine")) ||
                     (matches!(self.word_at(0),Some("pub"|"private")) &&
                      matches!(self.word_at(1),Some("fnc"|"routine"))));
                if static_method || module_fn {
                    if funcs.len()>=MAX_FUNCS {
                        return Err(Diagnostic::new(self.pos(),"E_SOURCE_LIMIT","too many functions"));
                    }
                    if static_method {
                        self.eat_word("pub");
                        self.eat_word("private");
                        self.expect_word("static")?;
                    }
                    let mut f=self.parse_func()?;
                    f.name=format!("{name}.{}",f.name);
                    funcs.push(f);
                } else {
                    let subject=match kind {
                        DeclKind::Actor=>"actor instance methods, state and scheduling",
                        DeclKind::Class=>"class instance methods, fields and constructors",
                        DeclKind::Module=>"nested modules and module bindings",
                        DeclKind::Struct=>"struct fields and methods",
                        DeclKind::Interface=>"interface contracts",
                        DeclKind::Trait=>"trait methods",
                        DeclKind::Enum=>unreachable!(),
                    };
                    return Err(unsupported(self.pos(),subject));
                }
            }
        }
        decls.push(Declaration{name,pos:start,kind,variants});
        Ok(())
    }

    fn parse_func(&mut self) -> R<Func> {
        if *self.peek() == Tok::At {
            return Err(unsupported(self.pos(), "annotations"));
        }
        let start = self.pos();
        self.eat_word("pub");
        if self.eat_word("define") { self.eat_word("pub"); }
        if self.is_word("private") {
            return Err(unsupported(self.pos(), "'private' visibility"));
        }
        for m in [
            "async",
            "suspend",
            "suspends",
            "nlex",
            "abstract",
            "structural",
            "static",
            "generator",
        ] {
            if self.is_word(m) {
                return Err(unsupported(self.pos(), &format!("'{m}' callables")));
            }
        }
        if !(self.eat_word("fnc") || self.eat_word("routine")) {
            if let Some(w) = self.word_at(0) {
                if UNSUPPORTED_STMT.contains(&w) || matches!(w, "val" | "let" | "const") {
                    return Err(unsupported(self.pos(), &format!("top-level '{w}'")));
                }
            }
            return Err(Diagnostic::new(
                self.pos(),
                "E_TOPLEVEL",
                format!("expected a 'fnc' declaration, found {}", describe(self.peek())),
            ));
        }
        let (name, pos) = self.ident("a function name")?;
        if *self.peek() == Tok::Lt {
            return Err(unsupported(self.pos(), "generic functions"));
        }
        self.expect(Tok::LParen, "'('")?;
        let mut params: Vec<Param> = Vec::new();
        if *self.peek() != Tok::RParen {
            loop {
                if params.len() >= MAX_PARAMS {
                    return Err(Diagnostic::new(
                        self.pos(),
                        "E_SOURCE_LIMIT",
                        format!("more than {MAX_PARAMS} parameters"),
                    ));
                }
                let p = self.parse_param()?;
                if params.iter().any(|q| q.name == p.name) {
                    return Err(Diagnostic::new(
                        p.pos,
                        "E_DUPLICATE",
                        format!("duplicate parameter name '{}'", p.name),
                    ));
                }
                params.push(p);
                if !self.eat(&Tok::Comma) {
                    break;
                }
            }
        }
        self.expect(Tok::RParen, "')'")?;
        // grammar.ebnf: return_spec = [ ( ":" | "->" ) type_ref ] ; body = block
        let ret = if self.eat(&Tok::Colon) || self.eat(&Tok::Arrow) {
            self.parse_type()?
        } else {
            Type::Void
        };
        let body = self.parse_block()?;
        let _ = start;
        Ok(Func {
            name,
            pos,
            params,
            ret,
            body,
        })
    }

    fn parse_param(&mut self) -> R<Param> {
        let pos = self.pos();
        if self.is_word("structural") {
            return Err(unsupported(pos, "structural parameters"));
        }
        // compatibility form `name: type [mut]`
        if matches!(self.peek(), Tok::Ident(_)) && *self.peek_at(1) == Tok::Colon {
            let (name, pos) = self.ident("a parameter name")?;
            self.advance();
            let ty = self.parse_type()?;
            let mutable = self.eat_word("mut");
            return self.finish_param(name, ty, mutable, pos);
        }
        let ty = self.parse_type()?;
        let mutable = self.eat_word("mut");
        let (name, pos) = self.ident("a parameter name")?;
        self.finish_param(name, ty, mutable, pos)
    }

    fn finish_param(&self, name: String, ty: Type, mutable: bool, pos: Pos) -> R<Param> {
        if ty == Type::Void {
            return Err(Diagnostic::new(pos, "E_TYPE", "a parameter cannot have type void"));
        }
        if *self.peek() == Tok::Assign {
            return Err(unsupported(self.pos(), "default parameter values"));
        }
        Ok(Param { name, ty, mutable, pos })
    }

    // ----------------------------------------------------------- statements

    fn parse_block(&mut self) -> R<Vec<Stmt>> {
        self.expect(Tok::LBrace, "'{'")?;
        self.enter()?;
        let mut out = Vec::new();
        while *self.peek() != Tok::RBrace {
            if *self.peek() == Tok::Eof {
                return Err(self.expected("'}'"));
            }
            out.push(self.parse_stmt()?);
        }
        self.advance();
        self.leave();
        Ok(out)
    }

    fn parse_stmts_until(&mut self, stops: &[&str]) -> R<Vec<Stmt>> {
        self.enter()?;
        let mut out = Vec::new();
        loop {
            if let Some(w) = self.word_at(0) {
                if stops.contains(&w) {
                    break;
                }
            }
            if *self.peek() == Tok::Eof {
                let want: Vec<String> = stops.iter().map(|s| format!("'{s}'")).collect();
                return Err(self.expected(&want.join(" or ")));
            }
            if *self.peek() == Tok::RBrace {
                return Err(self.expected(&format!("'{}'", stops.last().unwrap())));
            }
            out.push(self.parse_stmt()?);
        }
        self.leave();
        Ok(out)
    }

    fn parse_loop_body(&mut self) -> R<Vec<Stmt>> {
        if *self.peek() == Tok::LBrace {
            self.parse_block()
        } else if self.eat_word("do") {
            let body = self.parse_stmts_until(&["done"])?;
            self.expect_word("done")?;
            Ok(body)
        } else {
            Err(self.expected("'do' or '{'"))
        }
    }

    /// Java `Parser.consumeStatementTerminator`: a `;`, a safe statement
    /// boundary (`}` `fi` `end` `else` `elseif` `done` or end of input), or an
    /// implicit newline terminator (the next token starts on a later line).
    fn terminator(&mut self) -> R<()> {
        if self.eat(&Tok::Semi) {
            return Ok(());
        }
        let boundary = matches!(self.peek(), Tok::RBrace | Tok::Eof)
            || matches!(
                self.word_at(0),
                Some("fi" | "end" | "done" | "elif" | "elseif" | "else" | "catch" | "finally")
            );
        let newline = self.i > 0 && *self.peek() != Tok::Eof && self.toks[self.i - 1].pos.line < self.pos().line;
        if boundary || newline {
            Ok(())
        } else {
            Err(self.expected("';'"))
        }
    }

    fn parse_stmt(&mut self) -> R<Stmt> {
        let pos = self.pos();
        if *self.peek() == Tok::At {
            return Err(unsupported(pos, "annotations"));
        }
        if *self.peek() == Tok::LBrace || *self.peek() == Tok::LBracket {
            return Err(unsupported(pos, "destructuring"));
        }
        let word = self.word_at(0).map(str::to_owned);
        match word.as_deref() {
            Some("val" | "const" | "let") => {
                let s = self.parse_binding()?;
                self.terminator()?;
                Ok(s)
            }
            Some(_) if self.is_type_word(0) && matches!(self.peek_at(1), Tok::Ident(_)) => Err(Diagnostic::new(
                pos,
                "E_SYNTAX",
                "a local binding starts with 'val', 'const' or 'let' (for example 'val x: int = 1;')",
            )),
            Some("return") => {
                self.advance();
                let at_end = matches!(self.peek(), Tok::Semi | Tok::RBrace)
                    || matches!(self.word_at(0), Some("fi" | "end" | "done" | "elif" | "elseif" | "else"));
                let value = if at_end { None } else { Some(self.parse_expr()?) };
                self.terminator()?;
                Ok(Stmt::Return(value, pos))
            }
            Some("if") => self.parse_if(),
            Some("while") => {
                self.advance();
                let cond = self.parse_cond_head()?;
                self.eat(&Tok::Semi);
                let body = self.parse_loop_body()?;
                Ok(Stmt::While { cond, body, pos })
            }
            Some("loop") => {
                self.advance();
                let body = self.parse_loop_body()?;
                Ok(Stmt::Loop { body, pos })
            }
            Some("for") => self.parse_for(),
            Some("block") => {
                self.advance();
                Ok(Stmt::Block(self.parse_block()?))
            }
            Some("break") => {
                self.advance();
                self.terminator()?;
                Ok(Stmt::Break(pos))
            }
            Some("continue") => {
                self.advance();
                self.terminator()?;
                Ok(Stmt::Continue(pos))
            }
            Some("do") if matches!(self.word_at(1), Some("match" | "switch" | "select" | "nb")) => {
                Err(unsupported(pos, &format!("'do {}'", self.word_at(1).unwrap())))
            }
            Some(w) if UNSUPPORTED_STMT.contains(&w) => Err(unsupported(pos, &format!("'{w}'"))),
            Some("fnc" | "routine") => Err(unsupported(pos, "nested functions")),
            _ => {
                let e = self.parse_expr()?;
                if *self.peek() == Tok::Assign {
                    let at = self.pos();
                    self.advance();
                    let name = match e.kind {
                        ExprKind::Var(n) => n,
                        _ => return Err(Diagnostic::new(at, "E_ASSIGN_TARGET", "only a local variable can be assigned")),
                    };
                    let value = self.parse_expr()?;
                    if *self.peek() == Tok::Assign {
                        return Err(unsupported(self.pos(), "chained assignment"));
                    }
                    self.terminator()?;
                    return Ok(Stmt::Assign { name, value, pos });
                }
                if matches!(self.peek(), Tok::PlusPlus | Tok::MinusMinus) {
                    return Err(Diagnostic::new(
                        self.pos(),
                        "E_UNSUPPORTED",
                        "'++' and '--' exist only in a for update clause",
                    ));
                }
                self.terminator()?;
                Ok(Stmt::Expr(e))
            }
        }
    }

    /// grammar.ebnf `binding_decl`: `("val"|"const"|"let") (IDENT ":" type_ref | [type_ref] IDENT) "=" expression`.
    fn parse_binding(&mut self) -> R<Stmt> {
        let pos = self.pos();
        let kind = if self.eat_word("val") {
            BindKind::Val
        } else if self.eat_word("const") {
            BindKind::Const
        } else if self.eat_word("let") {
            BindKind::Let
        } else {
            return Err(self.expected("'val', 'const' or 'let'"));
        };
        // Java Parser.parseReferentMutability: `const mut` / `let mut` request a
        // mutable referent (no effect on scalar values); `val` already means
        // `const mut`, so `val mut` is a duplicate qualifier.
        if self.is_word("mut") {
            if kind == BindKind::Val {
                return Err(Diagnostic::new(
                    self.pos(),
                    "E_SYNTAX",
                    "'val' already means 'const mut'; 'val mut' is a duplicate mutability qualifier",
                ));
            }
            self.advance();
        }
        let (name, ty) = if matches!(self.peek(), Tok::Ident(_)) && *self.peek_at(1) == Tok::Colon {
            let (name, _) = self.ident("a variable name")?;
            self.advance();
            (name, Some(self.parse_type()?))
        } else if matches!(self.peek(), Tok::Ident(_)) && matches!(self.peek_at(1), Tok::Ident(_)) {
            let ty = self.parse_type()?;
            let (name, _) = self.ident("a variable name")?;
            (name, Some(ty))
        } else {
            let (name, _) = self.ident("a variable name")?;
            (name, None)
        };
        if ty == Some(Type::Void) {
            return Err(Diagnostic::new(pos, "E_TYPE", "a variable cannot have type void"));
        }
        if *self.peek() != Tok::Assign {
            return Err(Diagnostic::new(
                self.pos(),
                "E_UNINITIALIZED",
                format!("binding '{name}' needs an initializer"),
            ));
        }
        self.advance();
        let init = self.parse_expr()?;
        Ok(Stmt::Bind { kind, name, ty, init, pos })
    }

    fn parse_if(&mut self) -> R<Stmt> {
        let pos = self.pos();
        self.expect_word("if")?;
        let cond = self.parse_cond_head()?;
        let mut arms = Vec::new();
        let mut otherwise = None;
        if *self.peek() == Tok::LBrace {
            arms.push((cond, self.parse_block()?));
            loop {
                if self.eat_word("elif")
                    || self.eat_word("elseif")
                    || (self.is_word("else") && self.word_at(1) == Some("if") && {
                        self.advance();
                        self.advance();
                        true
                    })
                {
                    self.grow(1)?;
                    let c = self.parse_cond_head()?;
                    arms.push((c, self.parse_block()?));
                } else if self.eat_word("else") {
                    otherwise = Some(self.parse_block()?);
                    break;
                } else {
                    break;
                }
            }
        } else {
            self.eat(&Tok::Semi);
            if !(self.eat_word("then") || self.eat_word("do")) {
                return Err(self.expected("'then'"));
            }
            let stops = ["elif", "elseif", "else", "fi"];
            arms.push((cond, self.parse_stmts_until(&stops)?));
            loop {
                let is_elif = self.eat_word("elif")
                    || self.eat_word("elseif")
                    || (self.is_word("else") && self.word_at(1) == Some("if") && {
                        self.advance();
                        self.advance();
                        true
                    });
                if is_elif {
                    self.grow(1)?;
                    let c = self.parse_cond_head()?;
                    self.eat(&Tok::Semi);
                    if !(self.eat_word("then") || self.eat_word("do")) {
                        return Err(self.expected("'then'"));
                    }
                    arms.push((c, self.parse_stmts_until(&stops)?));
                } else if self.eat_word("else") {
                    otherwise = Some(self.parse_stmts_until(&["fi"])?);
                    break;
                } else {
                    break;
                }
            }
        }
        if !self.eat_word("fi") {
            return Err(Diagnostic::new(
                self.pos(),
                if *self.peek() == Tok::Eof { "E_EOF" } else { "E_FI" },
                format!(
                    "expected 'fi' to close the if started at {}:{}, found {}",
                    pos.line,
                    pos.col,
                    describe(self.peek())
                ),
            ));
        }
        // each elif arm nests one level deeper in the lowered if/else chain
        self.height -= arms.len() as u32 - 1;
        Ok(Stmt::If { arms, otherwise, pos })
    }

    fn parse_for(&mut self) -> R<Stmt> {
        let pos = self.pos();
        self.expect_word("for")?;
        let paren = self.eat(&Tok::LParen);
        if matches!(self.word_at(0), Some("const" | "let")) && self.word_at(2) == Some("of") {
            return Err(unsupported(self.pos(), "'for ... of' loops"));
        }
        let init = if *self.peek() == Tok::Semi {
            None
        } else if matches!(self.word_at(0), Some("val" | "const" | "let")) {
            Some(Box::new(self.parse_binding()?))
        } else if self.is_type_word(0) && matches!(self.peek_at(1), Tok::Ident(_)) {
            // grammar.ebnf for_init: type_ref IDENT "=" expression (immutable loop variable is useless; treat as let)
            let pos = self.pos();
            let ty = self.parse_type()?;
            let (name, _) = self.ident("a loop variable")?;
            self.expect(Tok::Assign, "'='")?;
            let init = self.parse_expr()?;
            Some(Box::new(Stmt::Bind {
                kind: BindKind::Let,
                name,
                ty: Some(ty),
                init,
                pos,
            }))
        } else {
            Some(Box::new(self.parse_simple_assign()?))
        };
        self.expect(Tok::Semi, "';'")?;
        let cond = if *self.peek() == Tok::Semi {
            None
        } else {
            Some(self.parse_cond()?)
        };
        self.expect(Tok::Semi, "';'")?;
        let update_end = if paren { Tok::RParen } else { Tok::LBrace };
        let update = if *self.peek() == update_end || (!paren && self.is_word("do")) {
            None
        } else {
            let upos = self.pos();
            let (name, _) = self.ident("a loop variable")?;
            if self.eat(&Tok::PlusPlus) {
                Some(Box::new(incr(name, upos, BinOp::Add)))
            } else if self.eat(&Tok::MinusMinus) {
                Some(Box::new(incr(name, upos, BinOp::Sub)))
            } else {
                self.expect(Tok::Assign, "'++', '--' or '='")?;
                let value = self.parse_expr()?;
                Some(Box::new(Stmt::Assign { name, value, pos: upos }))
            }
        };
        if paren {
            self.expect(Tok::RParen, "')'")?;
        }
        let body = self.parse_loop_body()?;
        Ok(Stmt::For {
            init,
            cond,
            update,
            body,
            pos,
        })
    }

    fn parse_simple_assign(&mut self) -> R<Stmt> {
        let pos = self.pos();
        let (name, _) = self.ident("a variable name")?;
        self.expect(Tok::Assign, "'='")?;
        let value = self.parse_expr()?;
        Ok(Stmt::Assign { name, value, pos })
    }

    // ---------------------------------------------------------- expressions

    fn parse_cond_head(&mut self) -> R<Expr> {
        let c = self.parse_cond()?;
        if *self.peek() == Tok::Assign {
            return Err(Diagnostic::new(
                self.pos(),
                "E_ASSIGN_CONDITION",
                "assignment is not allowed in a condition; use 'eq' to compare",
            ));
        }
        if self.is_word("as") {
            return Err(unsupported(self.pos(), "conditional 'as let' bindings"));
        }
        Ok(c)
    }

    /// `condition = condition_and { "|" condition_and } ;
    ///  condition_and = equality { "," equality } ;`
    fn parse_cond(&mut self) -> R<Expr> {
        self.cond += 1;
        let r = (|| {
            let mut links = 0;
            let mut left = self.parse_cond_and()?;
            while *self.peek() == Tok::Pipe {
                let pos = self.pos();
                self.advance();
                self.grow(1)?;
                links += 1;
                let right = self.parse_cond_and()?;
                left = Expr {
                    kind: ExprKind::Binary(BinOp::Or, Box::new(left), Box::new(right)),
                    pos,
                };
            }
            self.height -= links;
            Ok(left)
        })();
        self.cond -= 1;
        r
    }

    fn parse_cond_and(&mut self) -> R<Expr> {
        let mut links = 0;
        let mut left = self.parse_equality()?;
        while *self.peek() == Tok::Comma {
            let pos = self.pos();
            self.advance();
            self.grow(1)?;
            links += 1;
            let right = self.parse_equality()?;
            left = Expr {
                kind: ExprKind::Binary(BinOp::And, Box::new(left), Box::new(right)),
                pos,
            };
        }
        self.height -= links;
        Ok(left)
    }

    pub fn parse_expr(&mut self) -> R<Expr> {
        self.enter()?;
        let saved = self.cond;
        self.cond = 0;
        let r = self.parse_ternary();
        self.cond = saved;
        self.leave();
        r
    }

    fn parse_ternary(&mut self) -> R<Expr> {
        let c = self.parse_equality()?;
        match self.peek() {
            Tok::Pipe => {
                return Err(Diagnostic::new(
                    self.pos(),
                    "E_UNSUPPORTED",
                    "'|' is a boolean OR only inside an if/while condition; use Or(a, b) elsewhere",
                ))
            }
            Tok::PipeGt => return Err(unsupported(self.pos(), "the '|>' pipeline operator")),
            Tok::Question => {
                let pos = self.pos();
                self.advance();
                let a = self.parse_expr()?;
                self.expect(Tok::Colon, "':'")?;
                self.enter()?;
                let b = self.parse_ternary();
                self.leave();
                return Ok(Expr {
                    kind: ExprKind::Ternary(Box::new(c), Box::new(a), Box::new(b?)),
                    pos,
                });
            }
            _ => {}
        }
        Ok(c)
    }

    fn parse_equality(&mut self) -> R<Expr> {
        let mut links = 0;
        let mut left = self.parse_comparison()?;
        loop {
            let pos = self.pos();
            let op = match self.peek() {
                Tok::EqEq => BinOp::Eq,
                Tok::NotEq => BinOp::Ne,
                Tok::Ident(s) if s == "eq" => BinOp::Eq,
                Tok::Ident(s) if s == "neq" => BinOp::Ne,
                Tok::Bang if self.word_at(1) == Some("eq") => {
                    self.advance();
                    BinOp::Ne
                }
                Tok::Ident(s) if s == "is" => return Err(unsupported(pos, "the identity operator 'is'")),
                _ => break,
            };
            self.advance();
            self.grow(1)?;
            links += 1;
            let right = self.parse_comparison()?;
            left = Expr {
                kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
                pos,
            };
        }
        self.height -= links;
        Ok(left)
    }

    fn parse_comparison(&mut self) -> R<Expr> {
        let mut links = 0;
        let mut left = self.parse_additive()?;
        loop {
            let pos = self.pos();
            let op = match self.peek() {
                Tok::Lt => BinOp::Lt,
                Tok::Le => BinOp::Le,
                Tok::Gt => BinOp::Gt,
                Tok::Ge => BinOp::Ge,
                Tok::Ident(s) if s == "as" || s == "matches" => return Err(unsupported(pos, &format!("'{s}'"))),
                _ => break,
            };
            self.advance();
            self.grow(1)?;
            links += 1;
            let right = self.parse_additive()?;
            left = Expr {
                kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
                pos,
            };
        }
        self.height -= links;
        Ok(left)
    }

    fn parse_additive(&mut self) -> R<Expr> {
        let mut links = 0;
        let mut left = self.parse_multiplicative()?;
        loop {
            let pos = self.pos();
            let op = match self.peek() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            self.grow(1)?;
            links += 1;
            let right = self.parse_multiplicative()?;
            left = Expr {
                kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
                pos,
            };
        }
        self.height -= links;
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> R<Expr> {
        let mut links = 0;
        let mut left = self.parse_unary()?;
        loop {
            let pos = self.pos();
            let op = match self.peek() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                _ => break,
            };
            self.advance();
            self.grow(1)?;
            links += 1;
            let right = self.parse_unary()?;
            left = Expr {
                kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
                pos,
            };
        }
        self.height -= links;
        Ok(left)
    }

    fn parse_unary(&mut self) -> R<Expr> {
        self.enter()?;
        let r = self.parse_unary_inner();
        self.leave();
        r
    }

    fn parse_unary_inner(&mut self) -> R<Expr> {
        let pos = self.pos();
        let op = match self.peek() {
            Tok::Minus => UnOp::Neg,
            Tok::Plus => UnOp::Plus,
            Tok::Bang => UnOp::Not,
            Tok::Amp => return Err(unsupported(pos, "borrow expressions")),
            Tok::PlusPlus | Tok::MinusMinus => {
                return Err(Diagnostic::new(
                    pos,
                    "E_UNSUPPORTED",
                    "'++' and '--' exist only in a for update clause",
                ))
            }
            Tok::Ident(s) if s == "await" => return Err(unsupported(pos, "'await'")),
            _ => return self.parse_postfix(),
        };
        self.advance();
        let inner = self.parse_unary()?;
        Ok(Expr {
            kind: ExprKind::Unary(op, Box::new(inner)),
            pos,
        })
    }

    fn parse_postfix(&mut self) -> R<Expr> {
        let pos = self.pos();
        let mut e = self.parse_primary()?;
        loop {
            match self.peek() {
                Tok::LParen => {
                    let name = match &e.kind {
                        ExprKind::Var(n) => n.clone(),
                        _ => return Err(unsupported(self.pos(), "calling a computed value")),
                    };
                    self.advance();
                    let mut args = Vec::new();
                    if *self.peek() != Tok::RParen {
                        loop {
                            if matches!(self.peek(), Tok::Ident(_)) && *self.peek_at(1) == Tok::Colon {
                                return Err(unsupported(self.pos(), "named arguments"));
                            }
                            if *self.peek() == Tok::Dot {
                                return Err(unsupported(self.pos(), "spread arguments"));
                            }
                            args.push(self.parse_expr()?);
                            if args.len() > MAX_PARAMS {
                                return Err(Diagnostic::new(self.pos(), "E_SOURCE_LIMIT", "too many call arguments"));
                            }
                            if !self.eat(&Tok::Comma) {
                                break;
                            }
                        }
                    }
                    self.expect(Tok::RParen, "')' to close the call")?;
                    e = Expr {
                        kind: ExprKind::Call(name, args),
                        pos,
                    };
                    if matches!(self.peek(), Tok::LParen) {
                        return Err(unsupported(self.pos(), "calling a computed value"));
                    }
                }
                Tok::Dot => {
                    let at = self.pos();
                    let base = match &e.kind {
                        ExprKind::Var(n) if !n.contains('.') => n.clone(),
                        _ => return Err(unsupported(at,"chained member access")),
                    };
                    self.advance();
                    let (member, _) = self.ident("a static member name")?;
                    e = Expr { kind:ExprKind::Var(format!("{base}.{member}")), pos };
                }
                Tok::LBracket => return Err(unsupported(self.pos(), "indexing")),
                Tok::Question => {
                    // ternary is handled by the caller; postfix `?` propagation is not
                    break;
                }
                _ => break,
            }
        }
        if let ExprKind::Var(n) = &e.kind {
            if n.starts_with("stdio.") {
                return Err(self.expected("'('"));
            }
        }
        Ok(e)
    }

    fn parse_primary(&mut self) -> R<Expr> {
        let pos = self.pos();
        let t = self.peek().clone();
        let kind = match t {
            Tok::Int(v) => {
                self.advance();
                ExprKind::Int(v)
            }
            Tok::Str(s) => {
                self.advance();
                ExprKind::Str(s)
            }
            Tok::LParen => {
                self.advance();
                let inner = if self.cond > 0 { self.parse_cond()? } else { self.parse_expr()? };
                if *self.peek() == Tok::Comma {
                    return Err(unsupported(self.pos(), "tuples"));
                }
                self.expect(Tok::RParen, "')'")?;
                return Ok(Expr { kind: inner.kind, pos });
            }
            Tok::Ident(s) => match s.as_str() {
                "true" => {
                    self.advance();
                    ExprKind::Bool(true)
                }
                "false" => {
                    self.advance();
                    ExprKind::Bool(false)
                }
                "null" => return Err(Diagnostic::new(pos, "E_NULL", "AttoLang has no null value; use Option<T>")),
                "new" | "spawn" | "await" | "self" | "super" | "match" | "select" | "readch" | "writech" | "obj" | "arr" | "typeof" => {
                    return Err(unsupported(pos, &format!("'{s}' expressions")))
                }
                w if is_reserved(w) => {
                    return Err(Diagnostic::new(
                        pos,
                        "E_SYNTAX",
                        format!("expected an expression, found keyword '{w}'"),
                    ))
                }
                _ => {
                    self.advance();
                    if *self.peek() == Tok::Lt && self.looks_generic() {
                        return Err(unsupported(self.pos(), "generic calls"));
                    }
                    ExprKind::Var(s)
                }
            },
            Tok::Pipe => return Err(unsupported(pos, "lambdas")),
            Tok::LBracket => return Err(unsupported(pos, "sequence literals")),
            Tok::LBrace => return Err(unsupported(pos, "record literals")),
            _ => return Err(self.expected("an expression")),
        };
        Ok(Expr { kind, pos })
    }

    /// `f<int>(..)` heuristic: `<` Type `>` `(`.
    fn looks_generic(&self) -> bool {
        matches!(self.peek_at(1), Tok::Ident(_)) && *self.peek_at(2) == Tok::Gt && *self.peek_at(3) == Tok::LParen
    }
}

fn incr(name: String, pos: Pos, op: BinOp) -> Stmt {
    let var = Expr {
        kind: ExprKind::Var(name.clone()),
        pos,
    };
    let one = Expr {
        kind: ExprKind::Int(Some(1)),
        pos,
    };
    Stmt::Assign {
        name,
        value: Expr {
            kind: ExprKind::Binary(op, Box::new(var), Box::new(one)),
            pos,
        },
        pos,
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => format!("'{s}'"),
        Tok::Int(_) => "an integer".into(),
        Tok::Str(_) => "a string".into(),
        Tok::Eof => "end of input".into(),
        other => {
            let s = match other {
                Tok::LParen => "(",
                Tok::RParen => ")",
                Tok::LBrace => "{",
                Tok::RBrace => "}",
                Tok::LBracket => "[",
                Tok::RBracket => "]",
                Tok::Semi => ";",
                Tok::Comma => ",",
                Tok::Colon => ":",
                Tok::ColonColon => "::",
                Tok::Arrow => "->",
                Tok::Dot => ".",
                Tok::Assign => "=",
                Tok::EqEq => "==",
                Tok::NotEq => "!=",
                Tok::Lt => "<",
                Tok::Le => "<=",
                Tok::Gt => ">",
                Tok::Ge => ">=",
                Tok::Plus => "+",
                Tok::PlusPlus => "++",
                Tok::Minus => "-",
                Tok::MinusMinus => "--",
                Tok::Star => "*",
                Tok::Slash => "/",
                Tok::Bang => "!",
                Tok::Question => "?",
                Tok::Pipe => "|",
                Tok::PipeGt => "|>",
                Tok::Amp => "&",
                Tok::At => "@",
                _ => "?",
            };
            format!("'{s}'")
        }
    }
}
