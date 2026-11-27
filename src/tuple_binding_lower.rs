//! Conservative executable scalar tuple lowering for the Rust/browser compiler.
//!
//! Converts closed `Tuple<int|bool|float|string,...>` into private generated
//! class storage and converts destructuring into exactly-once root evaluation
//! plus independent Copy scalar projections. Object/loan tuples, unproven
//! constexpr sources, dynamic tuple shapes and nested patterns fail closed.
//!
//! This is a bounded ABI subset; it is not general Tuple or actor semantics.
use std::collections::BTreeMap;
use crate::diag::{Diagnostic, Pos};
use crate::lexer::{lex, Tok, Token};
use crate::tuple_binding_admission::{self, BindingMode};
use crate::tuple_ownership_ir::{ElementType, Scalar, TupleType};

type R<T> = Result<T, Diagnostic>;
const PREFIX: &str = "__atto_tuple_private_";
const LIMIT: usize = 16;
const MAX_SHAPES: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Shape(Vec<String>);

fn word(t: &Token, s: &str) -> bool {
    matches!(&t.tok, Tok::Ident(w) if w == s)
}
fn id(t: &Token) -> Option<&str> {
    if let Tok::Ident(s) = &t.tok { Some(s) } else { None }
}
fn error(pos: Pos, code: &'static str, message: &str) -> Diagnostic {
    Diagnostic::new(pos, code, message)
}
fn eofless(mut v: Vec<Token>) -> Vec<Token> {
    if matches!(v.last().map(|t| &t.tok), Some(Tok::Eof)) { v.pop(); }
    v
}
fn synthetic(code: &str, pos: Pos) -> R<Vec<Token>> {
    let mut v = eofless(lex(code)?);
    for t in &mut v { t.pos = pos; }
    Ok(v)
}
fn matching(t: &[Token], i: usize, left: Tok, right: Tok) -> Option<usize> {
    if t.get(i)?.tok != left { return None; }
    let mut depth = 0usize;
    for (j, tok) in t.iter().enumerate().skip(i) {
        if tok.tok == left { depth += 1; }
        if tok.tok == right {
            depth -= 1;
            if depth == 0 { return Some(j); }
        }
    }
    None
}
fn primitive(s: &str) -> Option<&'static str> {
    match s {
        "int" => Some("int"),
        "bool" => Some("bool"),
        "float" | "f64" => Some("float"),
        "string" | "String" => Some("string"),
        _ => None,
    }
}
fn ctor_name(i: usize) -> String { format!("{PREFIX}{i}") }
fn shape_for(parts: Vec<String>, shapes: &mut Vec<Shape>, pos: Pos) -> R<String> {
    // Reuse the same typed shape gate as the independently checked element
    // ownership planner. The current guest ABI still admits Copy scalars
    // *only*; a nominal object can never be silently treated as a scalar.
    let typed = parts.iter().map(|name| match name.as_str() {
        "int" => Ok(ElementType::Copy(Scalar::Int)),
        "bool" => Ok(ElementType::Copy(Scalar::Bool)),
        "float" => Ok(ElementType::Copy(Scalar::Float)),
        "string" => Ok(ElementType::Copy(Scalar::String)),
        _ => Err(error(pos, "E_UNSUPPORTED_TYPE",
            "scalar tuple codegen does not admit owned or unresolved element types")),
    }).collect::<R<Vec<_>>>()?;
    let _shape = TupleType::new(typed, pos)?;
    let target = Shape(parts);
    if let Some(idx) = shapes.iter().position(|x| *x == target) {
        Ok(ctor_name(idx))
    } else {
        if shapes.len() >= MAX_SHAPES {
            return Err(error(pos, "E_SOURCE_LIMIT", "too many distinct scalar tuple shapes (64 maximum)"));
        }
        let idx = shapes.len();
        shapes.push(target);
        Ok(ctor_name(idx))
    }
}
fn default_value(ty: &str) -> &'static str {
    match ty {
        "int" => "0", "bool" => "false", "float" => "0.0",
        "string" => "\"\"", _ => unreachable!(),
    }
}
fn create_class(name: &str, shape: &Shape) -> String {
    let mut out = format!("define class {name} as\n");
    for (i, ty) in shape.0.iter().enumerate() {
        // Class fields now default to val/final. Tuple storage is populated
        // exactly once by this generated constructor, so its write authority
        // must be explicit, as in shape_lower::synthesized. Users still cannot
        // mutate tuple-root fields outside constructor or via mutable loans.
        out.push_str(&format!("pub let {ty} e{i} = {};\n", default_value(ty)));
    }
    out.push_str("constructor(");
    for (i, ty) in shape.0.iter().enumerate() {
        if i > 0 { out.push_str(", "); }
        out.push_str(&format!("{ty} p{i}"));
    }
    out.push_str(") {\n");
    for i in 0..shape.0.len() {
        out.push_str(&format!("self.e{i} = p{i};\n"));
    }
    out.push_str("}\nend\n");
    out
}

/// Resolve only explicitly scalar Tuple<T,...> types. No fake object
/// mutability or actor-sendable tuple representation is inferred.
fn rewrite_types(tokens: &mut Vec<Token>, shapes: &mut Vec<Shape>) -> R<()> {
    let mut i = 0usize;
    while i + 1 < tokens.len() {
        if !word(&tokens[i], "Tuple") || tokens[i + 1].tok != Tok::Lt {
            i += 1;
            continue;
        }
        // 'Tuple < expression' must remain an ordinary comparison when
        // Tuple is a local identifier. Rewrite generic types only in a
        // type-introducing context; never classify expressions by spelling.
        let type_context = if i == 0 { false } else {
            match &tokens[i - 1].tok {
                Tok::Colon | Tok::Arrow => true,
                Tok::Ident(w) if matches!(w.as_str(),
                    "let" | "val" | "const" | "pub" | "private" |
                    "final" | "mut" | "static") => true,
                Tok::LParen | Tok::Comma => {
                    // Callable argument lists also use these punctuation
                    // tokens. Require an actual generic close followed by
                    // an identifier binder, as in '(Tuple<int> value)'.
                    let mut j = i + 2;
                    while j < tokens.len()
                        && tokens[j].tok != Tok::Gt
                        && tokens[j].tok != Tok::RParen
                        && tokens[j].tok != Tok::Semi
                    { j += 1; }
                    tokens.get(j).is_some_and(|t| t.tok == Tok::Gt)
                        && tokens.get(j + 1).is_some_and(|t|
                            matches!(&t.tok, Tok::Ident(w) if w != "then"))
                }
                _ => false,
            }
        };
        if !type_context {
            i += 1;
            continue;
        }
        let start = i;
        i += 2;
        let mut ts = Vec::new();
        if tokens.get(i).is_some_and(|x| x.tok != Tok::Gt) {
            loop {
                let t = tokens.get(i).and_then(id)
                    .and_then(primitive).ok_or_else(|| error(tokens[start].pos,
                        "E_UNSUPPORTED_TYPE", "only flat Tuple<int|bool|float|string,...> is executable"))?;
                ts.push(t.to_string());
                i += 1;
                if tokens.get(i).is_some_and(|x| x.tok == Tok::Comma) {
                    i += 1;
                    continue;
                }
                break;
            }
        }
        if !tokens.get(i).is_some_and(|x| x.tok == Tok::Gt) {
            return Err(error(tokens[start].pos, "E_SYNTAX", "expected '>' after tuple element types"));
        }
        let name = shape_for(ts, shapes, tokens[start].pos)?;
        let pos = tokens[start].pos;
        tokens.splice(start..i+1, [Token { tok: Tok::Ident(name), pos }]);
        i = start + 1;
    }
    Ok(())
}

/// Split an already balanced parenthesized tuple, respecting nested calls.
/// Items remain token slices so guest expression evaluation is not duplicated.
fn tuple_items(t: &[Token]) -> Option<Vec<&[Token]>> {
    if t.first()?.tok != Tok::LParen || t.last()?.tok != Tok::RParen { return None; }
    let mut start = 1usize;
    let mut items = Vec::new();
    let mut closers = Vec::new();
    let mut saw_comma = false;
    for i in 1..t.len()-1 {
        match t[i].tok {
            Tok::LParen => closers.push(Tok::RParen),
            Tok::LBracket => closers.push(Tok::RBracket),
            Tok::LBrace => closers.push(Tok::RBrace),
            Tok::RParen | Tok::RBracket | Tok::RBrace => {
                if closers.pop().as_ref() != Some(&t[i].tok) { return None; }
            }
            Tok::Comma if closers.is_empty() => {
                saw_comma = true;
                if i == start { return None; }
                items.push(&t[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if !closers.is_empty() { return None; }
    if start < t.len()-1 { items.push(&t[start..t.len()-1]); }
    if !saw_comma && !items.is_empty() { return None; }
    // Identifier-equals is a Record field, not a positional tuple item.
    // Leave records and mixed items to the production parser's admission gate.
    // Equality (==) and nested calls are deliberately not field markers.
    if items.iter().any(|item| {
        matches!(item.first().map(|x| &x.tok), Some(Tok::Ident(_)))
            && item.get(1).is_some_and(|x| x.tok == Tok::Assign)
    }) { return None; }
    if items.len() > LIMIT { return None; }
    Some(items)
}
fn scalar_literal(item: &[Token]) -> Option<&'static str> {
    match item {
        [Token { tok: Tok::Int(_), .. }] => Some("int"),
        [Token { tok: Tok::Float(_), .. }] => Some("float"),
        [Token { tok: Tok::Str(_), .. }] => Some("string"),
        [Token { tok: Tok::Ident(s), .. }] if s == "true" || s == "false" => Some("bool"),
        [Token { tok: Tok::Minus, .. }, Token { tok: Tok::Int(_), .. }] => Some("int"),
        [Token { tok: Tok::Minus, .. }, Token { tok: Tok::Float(_), .. }] => Some("float"),
        _ => None,
    }
}
// Preserve the old finite-float admission invariant independently of token
// reconstruction. Keeping source tokens must not widen accepted literals.
fn validate_expression_tokens(tokens: &[Token]) -> R<()> {
    for t in tokens {
        if let Tok::Float(bits) = t.tok {
            if !f64::from_bits(bits).is_finite() {
                return Err(error(t.pos, "E_FLOAT_LITERAL",
                    "non-finite scalar tuple literal is not admitted"));
            }
        }
    }
    Ok(())
}
fn constant_tuple(rhs: &[Token]) -> bool {
    tuple_items(rhs).is_some_and(|items| items.iter().all(|x| scalar_literal(x).is_some()))
}
/// Gather explicit top-level scalar tuple-return signatures, so a tuple
/// binder consuming `fixtures()` cannot accidentally project an arbitrary
/// class merely because it has similarly named fields.
/// Count only source-level call/parameter commas; nested generic/function
/// delimiters do not accidentally change overload resolution.
fn argument_count(tokens: &[Token], type_parameters: bool) -> usize {
    if tokens.is_empty() { return 0; }
    let mut depth = 0usize;
    let mut count = 1usize;
    for t in tokens {
        match t.tok {
            Tok::LParen | Tok::LBracket | Tok::LBrace => depth += 1,
            Tok::RParen | Tok::RBracket | Tok::RBrace if depth > 0 => depth -= 1,
            // In formal parameter TYPE annotations, angles delimit
            // generic arguments. In actual CALL expressions, < and >
            // are comparisons, not grouping. Counting '<' as nesting
            // would silently misidentify a tuple-returning overload.
            Tok::Lt if type_parameters => depth += 1,
            Tok::Gt if type_parameters && depth > 0 => depth -= 1,
            Tok::Comma if depth == 0 => count += 1,
            _ => {}
        }
    }
    count
}

fn tuple_functions(tokens: &[Token], shapes: &[Shape]) -> BTreeMap<(String, usize), usize> {
    let mut result = BTreeMap::new();
    for i in 0..tokens.len().saturating_sub(3) {
        if !word(&tokens[i], "fnc") { continue; }
        let Some(name) = id(&tokens[i+1]) else { continue; };
        if tokens[i+2].tok != Tok::LParen { continue; }
        let Some(end) = matching(tokens, i+2, Tok::LParen, Tok::RParen) else { continue; };
        if tokens.get(end+1).is_none_or(|t| t.tok != Tok::Colon && t.tok != Tok::Arrow) {
            continue;
        }
        let Some(ret) = tokens.get(end+2).and_then(id) else { continue; };
        if let Some(n) = shapes.iter().enumerate().find_map(|(j,_)|
            (ctor_name(j) == ret).then_some(j))
        {
            result.insert((name.to_string(), argument_count(&tokens[i+3..end], true)), n);
        }
    }
    result
}

fn tuple_call(rhs: &[Token], funcs: &BTreeMap<(String, usize), usize>) -> Option<usize> {
    if rhs.len() < 3 || rhs[1].tok != Tok::LParen { return None; }
    let name = id(&rhs[0])?;
    if matching(rhs, 1, Tok::LParen, Tok::RParen) != Some(rhs.len()-1) { return None; }
    funcs.get(&(name.to_string(), argument_count(&rhs[2..rhs.len()-1], false))).copied()
}

/// A tuple initializer must either be an explicit tuple constructor or a
/// typed function return. This disallows arbitrary non-Tuple objects.
fn checked_rhs(rhs: &[Token], functions: &BTreeMap<(String, usize), usize>, pos: Pos) -> R<()> {
    if tuple_items(rhs).is_some() || tuple_call(rhs, functions).is_some() {
        return Ok(());
    }
    Err(error(pos, "E_UNSUPPORTED",
        "tuple destructuring requires a scalar tuple literal or a function returning Tuple<...>"))
}

fn lower_bindings(tokens: &mut Vec<Token>, functions: &BTreeMap<(String, usize), usize>, shapes: &[Shape]) -> R<()> {
    let bindings = tuple_binding_admission::inspect(tokens)?;
    for (i, b) in bindings.iter().enumerate().rev() {
        let mut end = b.rhs_start_index;
        let mut depth = 0usize;
        while end < tokens.len() {
            match tokens[end].tok {
                Tok::LParen | Tok::LBrace | Tok::LBracket => depth += 1,
                Tok::RParen | Tok::RBrace | Tok::RBracket if depth > 0 => depth -= 1,
                Tok::Semi if depth == 0 => break,
                _ => {}
            }
            end += 1;
        }
        if tokens.get(end).is_none_or(|t| t.tok != Tok::Semi) {
            return Err(error(b.pos, "E_UNSUPPORTED",
                "scalar tuple lowering requires an explicit ';' terminator"));
        }
        let rhs = &tokens[b.rhs_start_index..end];
        checked_rhs(rhs, functions, b.pos)?;
        validate_expression_tokens(rhs)?;
        let actual_arity = if let Some(items) = tuple_items(rhs) {
            items.len()
        } else {
            let index = tuple_call(rhs, functions).ok_or_else(|| error(b.pos,
                "E_UNSUPPORTED", "tuple-returning callable overload is not statically known"))?;
            shapes[index].0.len()
        };
        if actual_arity != b.members.len() {
            return Err(error(b.pos, "E_ARITY",
                "tuple destructuring binder count must equal initializer tuple arity"));
        }
        if b.members.iter().any(|m| m.name.is_some()
            && m.mode == BindingMode::Const && m.mutable_referent)
        {
            return Err(error(b.pos, "E_CONST_MUT_TARGET",
                "const mut on a Copy-scalar tuple element is not meaningful"));
        }
        let want_const = b.alias.as_ref().is_some_and(|a| a.mode == BindingMode::Const)
            || b.members.iter().any(|m| m.name.is_some() && m.mode == BindingMode::Const);
        if want_const && !constant_tuple(rhs) {
            return Err(error(b.pos, "E_CONST",
                "local tuple const requires a compile-time scalar tuple literal; a runtime fixtures() call is not const"));
        }
        let constant_elements = if constant_tuple(rhs) { tuple_items(rhs) } else { None };
        let owned = format!("{PREFIX}owner_{i}");
        let alias = b.alias.as_ref().map_or(owned.as_str(), |a| a.name.as_str());
        let root_mutable = b.alias.as_ref().is_some_and(|a| a.mode == BindingMode::Let);
        // Keep original RHS tokens and positions. The expression parser uses
        // source adjacency to distinguish contiguous shifts and ownership
        // operators; printing and re-lexing with spaces destroys that information.
        let mut generated = synthetic(
            &format!("{} {alias} =", if root_mutable { "let" } else { "val" }), b.pos)?;
        generated.extend_from_slice(rhs);
        generated.push(Token { tok: Tok::Semi, pos: b.pos });
        for (k, member) in b.members.iter().enumerate() {
            if let Some(name) = &member.name {
                let ty = member.annotation.as_ref()
                    .map_or(String::new(), |t| format!(": {t}"));
                match member.mode {
                    BindingMode::Let | BindingMode::Val => {
                        let kind = if member.mode == BindingMode::Let { "let" } else { "val" };
                        generated.extend(synthetic(&format!("{kind} {name}{ty} = {alias}.e{k};"), b.pos)?);
                    }
                    BindingMode::Const => {
                        let part = constant_elements.as_ref()
                            .and_then(|items| items.get(k))
                            .ok_or_else(|| error(b.pos, "E_CONST",
                                "tuple const element requires a constexpr literal"))?;
                        generated.extend(synthetic(&format!("const {name}{ty} ="), b.pos)?);
                        generated.extend_from_slice(part);
                        generated.push(Token { tok: Tok::Semi, pos: b.pos });
                    }
                }
            }
        }
        tokens.splice(b.start_index..=end, generated);
    }
    Ok(())
}

fn rewrite_literal_tuples(tokens: &mut Vec<Token>, shapes: &mut Vec<Shape>) -> R<()> {
    let mut i = 0usize;
    while i < tokens.len() {
        if tokens[i].tok != Tok::LParen
            || !(i > 0 && (word(&tokens[i-1], "return") || tokens[i-1].tok == Tok::Assign))
        {
            i += 1;
            continue;
        }
        let Some(close) = matching(tokens, i, Tok::LParen, Tok::RParen) else {
            return Err(error(tokens[i].pos, "E_SYNTAX", "unclosed tuple expression"));
        };
        let Some(items) = tuple_items(&tokens[i..=close]) else {
            i += 1;
            continue;
        };
        validate_expression_tokens(&tokens[i..=close])?;
        let detected: Option<Vec<String>> = items.iter().map(|x| scalar_literal(x).map(str::to_owned)).collect();
        let shape_name = if let Some(parts) = detected {
            shape_for(parts, shapes, tokens[i].pos)?
        } else {
            let candidates: Vec<usize> = shapes.iter().enumerate().filter_map(|(idx, s)|
                (s.0.len() == items.len()).then_some(idx)).collect();
            if candidates.len() != 1 {
                return Err(error(tokens[i].pos, "E_UNSUPPORTED_TYPE",
                    "nonliteral scalar tuple expression requires exactly one known Tuple shape of this arity"));
            }
            ctor_name(candidates[0])
        };
        // Preserve expression tokens, decoded literal values and original
        // positions. Only the tuple delimiters and constructor prefix change.
        let pos = tokens[i].pos;
        let mut replacement = synthetic(&format!("new {shape_name}("), pos)?;
        for (index, item) in items.iter().enumerate() {
            if index != 0 { replacement.push(Token { tok: Tok::Comma, pos }); }
            replacement.extend_from_slice(item);
        }
        replacement.push(Token { tok: Tok::RParen, pos: tokens[close].pos });
        let next = i + replacement.len();
        tokens.splice(i..=close, replacement);
        i = next;
    }
    Ok(())
}


// This lane lowers a closed, flat scalar tuple pattern to a private typed
// function. Call arguments are evaluated by the real checker/emitter in
// source order, including discards. No expression substitution or evaluation
// by this pass; no allocation, runtime lambda or general aggregate ABI.
struct PatternLower<'a> {
    tokens: &'a [Token],
    close: Vec<Option<usize>>,
    helpers: Vec<Token>,
    count: usize,
}
impl PatternLower<'_> {
    fn tok(&self, i: usize) -> Option<&Tok> { self.tokens.get(i).map(|t| &t.tok) }
    fn candidate(&self, i: usize) -> bool {
        self.tok(i) == Some(&Tok::LParen) &&
            self.close.get(i).copied().flatten().is_some_and(|j|
                self.tok(j+1) == Some(&Tok::PipeGt) &&
                self.tok(j+2) == Some(&Tok::Pipe) &&
                self.tok(j+3) == Some(&Tok::LParen))
    }
    fn diag(&self, i: usize, message: &str) -> Diagnostic {
        error(self.tokens.get(i).map_or(Pos::default(), |t| t.pos),
            "E_TUPLE_PATTERN", message)
    }
    fn scalar(&mut self, i: &mut usize, bp: u8, depth: usize,
        scope: Option<&std::collections::BTreeSet<String>>) -> R<Vec<Token>> {
        if depth > 48 { return Err(self.diag(*i, "scalar pattern depth exceeds 48")); }
        let at = *i;
        let mut out = if self.candidate(at) {
            self.pattern(i, depth+1, scope)?
        } else {
            match self.tok(at).cloned() {
                Some(Tok::Int(_)) => { *i += 1; vec![self.tokens[at].clone()] }
                Some(Tok::Ident(name)) if name != "_" &&
                    !matches!(name.as_str(), "true" | "false" | "self" | "arguments") &&
                    scope.is_none_or(|s| s.contains(&name)) => {
                    *i += 1; vec![self.tokens[at].clone()]
                }
                Some(Tok::Minus) => {
                    *i += 1;
                    let mut v=vec![self.tokens[at].clone()];
                    v.extend(self.scalar(i, 3, depth+1, scope)?);
                    v
                }
                Some(Tok::LParen) => {
                    *i += 1;
                    let mut v=vec![self.tokens[at].clone()];
                    v.extend(self.scalar(i, 0, depth+1, scope)?);
                    if self.tok(*i)!=Some(&Tok::RParen) {
                        return Err(self.diag(*i,"expected scalar group close"));
                    }
                    v.push(self.tokens[*i].clone()); *i+=1; v
                }
                _ => return Err(self.diag(at,"expected closed int arithmetic over bound leaves")),
            }
        };
        loop {
            let power=match self.tok(*i) {
                Some(Tok::Plus | Tok::Minus) => 1,
                Some(Tok::Star) => 2,
                _ => break,
            };
            if power < bp { break; }
            out.push(self.tokens[*i].clone()); *i+=1;
            out.extend(self.scalar(i,power+1,depth+1,scope)?);
            if out.len()>crate::lexer::MAX_TOKENS {
                return Err(self.diag(*i,"pattern token budget exceeded"));
            }
        }
        Ok(out)
    }
    fn pattern(&mut self, i: &mut usize, depth: usize,
        outer: Option<&std::collections::BTreeSet<String>>) -> R<Vec<Token>> {
        if depth > 48 { return Err(self.diag(*i,"pattern depth exceeds 48")); }
        let start=*i;
        let pos=self.tokens[start].pos;
        let close=self.close[start].ok_or_else(||self.diag(start,"unclosed tuple root"))?;
        // Preserve one-element tuple vs grouping, and reject record syntax.
        let items=tuple_items(&self.tokens[start..=close])
            .ok_or_else(||self.diag(start,"expected flat literal tuple root"))?;
        let arity=items.len();
        if arity>8 { return Err(self.diag(start,"maximum tuple pattern arity is 8")); }
        *i+=1;
        let mut args=Vec::new();
        while *i<close {
            args.push(self.scalar(i,0,depth+1,outer)?);
            if *i==close { break; }
            if self.tok(*i)!=Some(&Tok::Comma) {
                return Err(self.diag(*i,"expected tuple element separator"));
            }
            *i+=1;
        }
        if *i!=close || args.len()!=arity { return Err(self.diag(*i,"invalid tuple root")); }
        *i=close+4;
        let pattern_start=close+3;
        let end=self.close[pattern_start].ok_or_else(||self.diag(pattern_start,"unclosed pattern"))?;
        let mut names=Vec::new();
        let mut scope=std::collections::BTreeSet::new();
        while *i<end {
            let Some(Tok::Ident(name))=self.tok(*i).cloned() else {
                return Err(self.diag(*i,"expected flat binder"));
            };
            if matches!(name.as_str(),"self"|"arguments"|"true"|"false") ||
                (name!="_" && !scope.insert(name.clone())) {
                return Err(self.diag(*i,"duplicate or reserved binder"));
            }
            *i+=1;
            if self.tok(*i)==Some(&Tok::Colon) {
                *i+=1;
                if name=="_" || self.tokens.get(*i).is_none_or(|t| !word(t,"int")) {
                    return Err(self.diag(*i,"only int leaf annotations; discard cannot be typed"));
                }
                *i+=1;
            }
            names.push(name);
            if names.len()>8 { return Err(self.diag(*i,"maximum tuple pattern arity is 8")); }
            if *i==end {
                if names.len()==1 {return Err(self.diag(*i,"singleton pattern needs trailing comma"));}
                break;
            }
            if self.tok(*i)!=Some(&Tok::Comma) {return Err(self.diag(*i,"expected binder separator"));}
            *i+=1;
        }
        if *i!=end || names.len()!=arity {return Err(self.diag(*i,"pattern arity mismatch"));}
        if self.tok(end+1)!=Some(&Tok::Pipe) || self.tok(end+2)!=Some(&Tok::Arrow) {
            return Err(self.diag(end,"expected '| ->' after unary pattern"));
        }
        *i=end+3;
        let body=self.scalar(i,0,depth+1,Some(&scope))?;
        // Do not silently truncate unsupported body syntax into outer syntax.
        if !matches!(self.tok(*i),Some(Tok::RParen|Tok::Comma|Tok::Semi|Tok::RBrace|Tok::PipeGt|Tok::Eof)) {
            return Err(self.diag(*i,"unsupported pattern body continuation"));
        }
        if self.count>=128 {return Err(self.diag(start,"maximum 128 pattern helpers"));}
        let name=format!("{PREFIX}pattern_{}",self.count);
        self.count+=1;
        let mut parameters=Vec::new();
        for (n,binder) in names.iter().enumerate() {
            let binder=if binder=="_" {format!("{PREFIX}discard_{n}")} else {binder.clone()};
            parameters.push(format!("int {binder}"));
        }
        self.helpers.extend(synthetic(&format!("fnc {name}({}): int {{ return",parameters.join(",")),pos)?);
        self.helpers.extend(body);
        self.helpers.extend(synthetic("; }",pos)?);
        let mut out=synthetic(&format!("{name}("),pos)?;
        for (n,arg) in args.into_iter().enumerate() {
            if n>0 {out.push(Token{tok:Tok::Comma,pos});}
            out.extend(arg);
        }
        out.push(Token{tok:Tok::RParen,pos});
        Ok(out)
    }
}
fn lower_callable_patterns(tokens: Vec<Token>) -> R<Vec<Token>> {
    if !tokens.windows(3).any(|w| w[0].tok==Tok::PipeGt && w[1].tok==Tok::Pipe && w[2].tok==Tok::LParen) {
        return Ok(tokens);
    }
    // Linear matching table, bounded by the existing input token cap.
    let mut close=vec![None;tokens.len()];
    let mut stack=Vec::new();
    for (i,t) in tokens.iter().enumerate() {
        if t.tok==Tok::LParen {stack.push(i);}
        else if t.tok==Tok::RParen {if let Some(j)=stack.pop(){close[j]=Some(i);}}
    }
    let mut pass=PatternLower{tokens:&tokens,close,helpers:Vec::new(),count:0};
    let mut out=Vec::new();let mut i=0;
    while i<tokens.len() {
        if pass.candidate(i) {
            out.extend(pass.pattern(&mut i,0,None)?);
        } else {
            out.push(tokens[i].clone());i+=1;
        }
    }
    if pass.count==0 {return Ok(tokens);}
    // Existing parser/typechecker must still accept the entire rewritten unit.
    // Helpers are private; source cannot name their reserved prefix.
    let eof=out.pop().ok_or_else(||pass.diag(0,"missing EOF"))?;
    if eof.tok!=Tok::Eof {return Err(pass.diag(0,"missing EOF"));}
    out.extend(pass.helpers);out.push(eof);
    if out.len()>crate::lexer::MAX_TOKENS {
        return Err(error(tokens[0].pos,"E_SOURCE_LIMIT","tuple pattern lowering exceeds token budget"));
    }
    Ok(out)
}

/// Return a fully ordinary compiler token stream for provably Copy-scalar
/// tuple shapes, or fail closed. Evaluation of a function-produced tuple
/// happens exactly once in the root local; scalar projections copy values.
pub fn lower(mut tokens: Vec<Token>) -> R<Vec<Token>> {
    if tokens.iter().any(|t| id(t).is_some_and(|s| s.starts_with(PREFIX))) {
        return Err(error(tokens.first().map_or(Pos::default(), |t| t.pos),
            "E_RESERVED", "internal tuple storage names are reserved"));
    }
    tokens = lower_callable_patterns(tokens)?;
    let mut shapes = Vec::new();
    rewrite_types(&mut tokens, &mut shapes)?;
    let functions = tuple_functions(&tokens, &shapes);
    lower_bindings(&mut tokens, &functions, &shapes)?;
    rewrite_literal_tuples(&mut tokens, &mut shapes)?;
    if shapes.is_empty() { return Ok(tokens); }
    let pos = tokens.first().map_or(Pos::default(), |t| t.pos);
    let mut source = String::new();
    for (i, shape) in shapes.iter().enumerate() {
        source.push_str(&create_class(&ctor_name(i), shape));
    }
    let mut prefix = synthetic(&source, pos)?;
    prefix.extend(tokens);
    if prefix.len() > crate::lexer::MAX_TOKENS {
        return Err(error(pos, "E_SOURCE_LIMIT", "tuple lowering exceeds token budget"));
    }
    Ok(prefix)
}

#[cfg(test)]
mod parenthesis_boundary_regression {
    use super::{eofless, tuple_items};
    use crate::lexer::lex;

    fn arity(source: &str) -> Option<usize> {
        let tokens = eofless(lex(source).expect("lex fixture"));
        tuple_items(&tokens).map(|items| items.len())
    }

    #[test]
    fn grouping_and_tuple_commas_are_depth_sensitive() {
        for (source, expected) in [
            ("()", Some(0)),
            ("(5)", None),
            ("((5))", None),
            ("(5,)", Some(1)),
            ("(5,6,true)", Some(3)),
            ("((5,6),true)", Some(2)),
            ("(f(1,2),3,)", Some(2)),
            ("(\"a,b\",5)", Some(2)),
            ("(a==5,6)", Some(2)),
        ] {
            assert_eq!(arity(source), expected, "{source}");
        }
    }

    #[test]
    fn records_and_malformed_delimiters_never_enter_tuple_lowering() {
        for source in [
            "(a=5)", "(a=5,b=6)", "(a=5,6)", "(5,b=6)",
            "(,5)", "(5,,6)", "(f([1),2],3)", "(f({1),2},3)",
        ] {
            assert_eq!(arity(source), None, "{source}");
        }
        assert_eq!(arity("((a=5),6)"), Some(2));
    }
}

#[cfg(test)]
mod preserved_expression_tokens {
    use super::lower;
    use crate::lexer::{lex, Tok};

    #[test]
    fn lowering_keeps_original_shift_adjacency_and_positions() {
        let source = "fnc fixtures(): Tuple<int,int> { return ((1 <<< 3), 34); } fnc main(): int { let (a,b) = fixtures(); return a+b; }";
        let input = lex(source).unwrap();
        let original: Vec<_> = input.iter().filter(|t| t.tok == Tok::Lt)
            .skip(1).map(|t| (t.pos.line, t.pos.col)).collect();
        assert_eq!(original.len(), 3);
        let result = lower(input).unwrap();
        let actual: Vec<_> = result.iter().filter(|t| t.tok == Tok::Lt)
            .map(|t| (t.pos.line, t.pos.col)).collect();
        assert_eq!(actual, original);
    }

    #[test]
    fn destructuring_keeps_string_and_float_tokens_exact() {
        let source = r#"fnc main(): int { let (a,b) = ("a,b", -0.0); return 42; }"#;
        let input = lex(source).unwrap();
        let original = input.iter().find(|t| matches!(&t.tok, Tok::Str(v) if v == "a,b")).unwrap().clone();
        let result = lower(input).unwrap();
        assert!(result.iter().any(|t| t.tok == original.tok &&
            t.pos.line == original.pos.line && t.pos.col == original.pos.col));
    }
}
