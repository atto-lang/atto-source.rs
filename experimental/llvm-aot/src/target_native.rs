//! Executable, bounded target-syntax linker for real Atto native actors.
 //! Every method body is checked again by the production AST parser and typed
 //! checker. This is NOT an interpreter or string substitution.
 //! Critical callbacks are lowered only for a deliberately tiny, statically
 //! checked scalar form; unsupported bodies reject before native codegen.
use atto_compiler::ast::{Method, Program, Type};
use atto_compiler::check;
use atto_compiler::diag::Pos;
use atto_compiler::lexer::{Tok, Token};
use atto_compiler::parser::Parser;
use atto_compiler::{target_syntax, target_typed};

fn word(tok: &Token, name: &str) -> bool {
    matches!(&tok.tok, Tok::Ident(v) if v == name)
}
fn is_word(tokens: &[Token], i: usize, name: &str) -> bool {
    tokens.get(i).is_some_and(|t| word(t, name))
}
fn t(tk: Tok, pos: Pos) -> Token { Token { tok: tk, pos } }
fn error(code: &str, message: impl AsRef<str>) -> String {
    format!("{code}: {}", message.as_ref())
}
/// Confirm that the independently reparsed executable method has exactly the
/// signature validated by the nominal target header. Never infer an ABI from
/// a matching name and arity alone.
fn scalar_named_type(name: &str) -> Option<Type> {
    match name {
        "int" => Some(Type::Int),
        "bool" | "boolean" => Some(Type::Bool),
        "void" => Some(Type::Void),
        _ => None,
    }
}
fn locate_pos(tokens: &[Token], pos: Pos) -> Result<usize, String> {
    tokens.iter().position(|x| x.pos.line == pos.line && x.pos.col == pos.col)
        .ok_or_else(|| error("E_INTERNAL", "target header position lost"))
}
fn enclosing_paren(tokens: &[Token], start: usize, end: usize) -> Result<usize, String> {
    if !matches!(tokens.get(start).map(|x| &x.tok), Some(Tok::LParen)) {
        return Err(error("E_SYNTAX", "expected opening '('"));
    }
    let mut depth = 0i32;
    for j in start..end {
        match tokens[j].tok {
            Tok::LParen => depth += 1,
            Tok::RParen => {
                depth -= 1;
                if depth == 0 { return Ok(j); }
            }
            _ => {}
        }
    }
    Err(error("E_SYNTAX", "unclosed target receiver/callback"))
}
fn field_name(tokens: &[Token], at: usize, binder: &str) -> Option<String> {
    if !is_word(tokens, at, binder)
        || !matches!(tokens.get(at+1).map(|x| &x.tok), Some(Tok::Dot)) {
        return None;
    }
    match tokens.get(at+2).map(|x| &x.tok) {
        Some(Tok::Ident(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Single field read or single assignment under an already validated critical
/// callback. A receiver's guarded read/write is executed only on its own
/// mailbox worker. There are no exposed state references or suspension points.
fn lower_callback(tokens: &[Token], open: usize, end: usize,
    params: &[String]) -> Result<(Vec<Token>, usize), String> {
    let close = enclosing_paren(tokens, open, end)?;
    let k = open + 1;
    if !is_word(tokens, k, "critical")
        || !matches!(tokens.get(k+1).map(|x| &x.tok), Some(Tok::Minus))
        || !(is_word(tokens, k+2, "read") || is_word(tokens, k+2, "write"))
        || !matches!(tokens.get(k+3).map(|x| &x.tok), Some(Tok::Pipe))
        || !matches!(tokens.get(k+5).map(|x| &x.tok), Some(Tok::Pipe))
        || !matches!(tokens.get(k+6).map(|x| &x.tok), Some(Tok::Arrow)) {
        return Err(error("E_SEALED", "critical callback requires critical-read/write |s| -> body"));
    }
    let binder = match tokens.get(k+4).map(|x| &x.tok) {
        Some(Tok::Ident(name)) if name != "self" => name.as_str(),
        _ => return Err(error("E_SEALED", "invalid critical callback binder")),
    };
    let start = k+7;
    let pos = tokens[open].pos;
    if is_word(tokens, k+2, "read") {
        if close != start + 3 {
            return Err(error("E_UNSUPPORTED_CRITICAL",
                "only one scalar field read is currently native-lowered"));
        }
        let field = field_name(tokens, start, binder)
            .ok_or_else(|| error("E_SEALED", "critical-read requires s.field"))?;
        return Ok((vec![t(Tok::Ident("self".into()),pos),
                        t(Tok::Dot,pos),t(Tok::Ident(field),pos)],close+1));
    }

    // Exactly one assignment in a braced critical-write callback.
    if !matches!(tokens.get(start).map(|x| &x.tok), Some(Tok::LBrace))
        || !matches!(tokens.get(close-1).map(|x| &x.tok), Some(Tok::RBrace))
        || !matches!(tokens.get(close-2).map(|x| &x.tok), Some(Tok::Semi)) {
        return Err(error("E_UNSUPPORTED_CRITICAL",
            "critical-write must contain one braced scalar assignment"));
    }
    let left = start+1;
    let field = field_name(tokens, left, binder)
        .ok_or_else(|| error("E_SEALED", "critical-write needs s.field assignment"))?;
    if !matches!(tokens.get(left+3).map(|x| &x.tok), Some(Tok::Assign)) {
        return Err(error("E_SEALED", "critical-write requires plain assignment"));
    }
    let rhs = left+4;
    let rhs_end = close-2;
    if rhs >= rhs_end {
        return Err(error("E_UNSUPPORTED_CRITICAL", "empty critical-write RHS"));
    }
    let mut out = vec![t(Tok::Ident("self".into()),pos),t(Tok::Dot,pos),
                       t(Tok::Ident(field),pos),t(Tok::Assign,pos)];
    let mut i = rhs;
    while i < rhs_end {
        if let Some(member) = field_name(tokens,i,binder) {
            out.push(t(Tok::Ident("self".into()),tokens[i].pos));
            out.push(t(Tok::Dot,tokens[i].pos));
            out.push(t(Tok::Ident(member),tokens[i].pos));
            i+=3;
            continue;
        }
        match &tokens[i].tok {
            Tok::Int(Some(_)) | Tok::Plus | Tok::Minus | Tok::Star
                | Tok::LParen | Tok::RParen => {}
            Tok::Ident(name) if params.iter().any(|x| x==name) => {}
            _ => return Err(error("E_UNSUPPORTED_CRITICAL",
                "only pure integer expressions of s.field, params and literals are admitted")),
        }
        out.push(tokens[i].clone());
        i+=1;
    }
    out.push(t(Tok::Semi,pos));
    // Write callbacks are statements. Do not accidentally accept a callback
    // as an expression or silently create a stray second semicolon.
    if !matches!(tokens.get(close+1).map(|x| &x.tok), Some(Tok::Semi)) {
        return Err(error("E_UNSUPPORTED_CRITICAL",
            "critical-write must be a standalone statement"));
    }
    Ok((out,close+2))
}
fn lower_method_body(tokens: &[Token], body_start: usize, body_end: usize,
    params: &[String]) -> Result<Vec<Token>, String> {
    let mut out = Vec::new();
    let mut i = body_start;
    while i < body_end {
        if is_word(tokens,i,"self")
            && matches!(tokens.get(i+1).map(|x| &x.tok), Some(Tok::Dot))
            && is_word(tokens,i+2,"handle_state") {
            let open=i+3;
            let write = is_word(tokens,open+3,"write");
            if write && i > body_start
                && !matches!(tokens[i-1].tok,Tok::LBrace|Tok::Semi|Tok::RBrace) {
                return Err(error("E_UNSUPPORTED_CRITICAL",
                    "critical-write must be a standalone statement"));
            }
            let (expanded,next) = lower_callback(tokens,open,body_end,params)?;
            out.extend(expanded);
            i=next;
        } else {
            out.push(tokens[i].clone());
            i+=1;
        }
    }
    Ok(out)
}
fn parse_one_function(tokens: &[Token], start: usize, end: usize)
    -> Result<atto_compiler::ast::Func,String> {
    let mut from=None;
    for i in (0..start).rev() {
        if is_word(tokens,i,"fnc")
            && tokens[i+1..start].iter().all(|tok|
                !matches!(tok.tok,Tok::LBrace|Tok::RBrace|Tok::Semi)) {
            from=Some(if i>0 && is_word(tokens,i-1,"pub") {i-1}else{i});
            break;
        }
    }
    let from=from.ok_or_else(||error("E_INTERNAL","fnc header lost"))?;
    let mut subset=tokens[from..end].to_vec();
    subset.push(t(Tok::Eof,tokens[end-1].pos));
    let p=Parser::new(subset).parse_program().map_err(|e|e.to_string())?;
    if p.funcs.len()!=1 || !p.actors.is_empty() || !p.classes.is_empty() {
        return Err(error("E_SYNTAX","invalid native target top-level function"));
    }
    let func=p.funcs.into_iter().next().expect("exactly one function");
    // Parsed standalone function name indices are local to their temporary
    // parser. Do not allow unresolved Named/NamedRef indices to enter the
    // unified checked program without a verified type-name relocation table.
    if !matches!(func.ret,Type::Int|Type::Bool|Type::Void)
        || func.params.iter().any(|p|
            !matches!(p.ty,Type::Int|Type::Bool)) {
        return Err(error("E_UNSUPPORTED_NATIVE_TYPE",
            "target fnc signatures admit only int/bool parameters and int/bool/void result"));
    }
    Ok(func)
}
fn parse_one_receiver(tokens: &[Token], start: usize, end: usize,
    head: &target_syntax::ReceiverHeader) -> Result<Method,String> {
    let at=locate_pos(tokens,head.pos)?;
    let mtd=if is_word(tokens,at,"pub") {at+1} else {at};
    if !is_word(tokens,mtd,"mtd") || (is_word(tokens,at,"pub") != head.public) {
        return Err(error("E_INTERNAL","receiver header shifted"));
    }
    let close=enclosing_paren(tokens,mtd+1,start)?;
    let name_start=close+1;
    if !is_word(tokens,name_start,&head.name) {
        return Err(error("E_INTERNAL","receiver method name mismatch"));
    }
    let pos=head.pos;
    let mut subset=vec![t(Tok::Ident("define".into()),pos),
                        t(Tok::Ident("actor".into()),pos),
                        t(Tok::Ident(head.owner.clone()),pos),
                        t(Tok::Ident("as".into()),pos)];
    if head.public {subset.push(t(Tok::Ident("pub".into()),pos));}
    subset.extend_from_slice(&tokens[name_start..start]);
    let params=head.params.iter().map(|p|p.name.clone()).collect::<Vec<_>>();
    subset.extend(lower_method_body(tokens,start,end,&params)?);
    subset.push(t(Tok::Ident("end".into()),pos));
    subset.push(t(Tok::Eof,pos));
    let p=Parser::new(subset).parse_program().map_err(|e|e.to_string())?;
    if p.actors.len()!=1 || p.actors[0].methods.len()!=1 {
        return Err(error("E_INTERNAL","native target method did not reparse"));
    }
    let method=p.actors[0].methods[0].clone();
    if method.name != head.name || method.params.len()!=head.arity
        || method.ret != scalar_named_type(&head.return_type)
            .ok_or_else(|| error("E_UNSUPPORTED_NATIVE_TYPE",
                "receiver return is not an admitted primitive type"))?
        || method.params.iter().zip(&head.params).any(|(got, expected)|
            got.name != expected.name || Some(got.ty) !=
                scalar_named_type(&expected.type_name))
        || method.is_static {
        return Err(error("E_SIGNATURE",
            "target receiver name, parameters, types or return mismatch"));
    }
    Ok(method)
}
pub fn requires_target_link(tokens: &[Token]) -> bool {
    tokens.iter().any(|x|
        word(x,"declare") || word(x,"mtd") ||
        word(x,"namespace"))
}
pub fn lower(tokens: &[Token]) -> Result<Program,String> {
    let headers=target_syntax::parse_target_declarations(tokens)
        .map_err(|e|e.to_string())?;
    if !headers.imports.is_empty() {
        return Err(error("E_UNSUPPORTED_NATIVE_IMPORT",
            "target module imports require verified linker, not name-only imports"));
    }
    let module=target_typed::lower_target_signatures(&headers)
        .map_err(|e|e.to_string())?;
    // Only scalar methods and concrete actors are executable in this profile;
    // ActorRef parameters/returns must await ownership-aware call lowering.
    for m in &module.receivers {
        if !matches!(m.returns,target_typed::TargetType::Void|
            target_typed::TargetType::Int|target_typed::TargetType::Bool)
            || m.params.iter().any(|p|!matches!(p.ty,
                target_typed::TargetType::Int|target_typed::TargetType::Bool)) {
            return Err(error("E_UNSUPPORTED_NATIVE_MTD",
                "target receiver ABI admits only int/bool params and int/bool/void result"));
        }
    }
    for name in &headers.actors {
        if !headers.definitions.iter().any(|d|&d.name==name) {
            return Err(error("E_UNLINKED",
                format!("actor {name} is forward-declared but has no receive implementation")));
        }
    }
    if headers.actors.is_empty() || headers.receivers.is_empty() {
        return Err(error("E_UNSUPPORTED_NATIVE_MTD",
            "native target requires concrete actors and out-of-line receiver methods"));
    }
    let mut program=target_typed::build_actor_program(&headers,&module);
    check::check_target_actor_interface(&program).map_err(|e|e.to_string())?;
    // TargetHeaders reparses each concrete actor with a temporary parser.
    // Only primitive fields and payloads can be reused without relocating
    // its locally indexed Named types into the linked Program type table.
    for actor in &program.actors {
        // Actor state is sealed even if the legacy concrete parser accepts
        // a 'pub' field. Target-native syntax must not export state via an
        // accidental public field descriptor.
        if actor.fields.iter().any(|field| field.public) {
            return Err(error("E_SEALED",
                "target actor state cannot be declared public"));
        }
        if actor.fields.iter().any(|field|
            !matches!(field.ty,Type::Int|Type::Bool))
            || actor.receive.as_ref().is_some_and(|r|
                !matches!(r.payload,Type::Int|Type::Bool)) {
            return Err(error("E_UNSUPPORTED_NATIVE_TYPE",
                "target actor state and mail payloads must be int/bool"));
        }
    }
    // A target-profile actor's legacy definition can include a lifecycle
    // receive handler, but cannot carry mutable state access or inline
    // receiver methods. State is only lowered inside checked out-of-line mtd.
    for actor in &program.actors {
        if !actor.methods.is_empty() {
            return Err(error("E_SEALED",
                "target actor methods must be declared out-of-line with mtd"));
        }
    }
    // Forbid raw self.state outside the guarded target receiver callbacks.
    // This applies to receive handlers too (which are parsed from the
    // concrete actor). The callback transformer never changes these tokens.
    for i in 0..tokens.len().saturating_sub(3) {
        if is_word(tokens,i,"self")
            && matches!(tokens[i+1].tok,Tok::Dot)
            && matches!(tokens[i+2].tok,Tok::Ident(_))
            && !matches!(tokens[i+3].tok,Tok::LParen) {
            return Err(error("E_SEALED",
                "direct actor state access is forbidden; use critical-read/write"));
        }
    }
    for &(start,end) in &headers.function_body_spans {
        program.funcs.push(parse_one_function(tokens,start,end)?);
    }
    if headers.receiver_body_spans.len()!=headers.receivers.len() {
        return Err(error("E_INTERNAL","receiver span coverage mismatch"));
    }
    for &(index,start,end) in &headers.receiver_body_spans {
        let head=headers.receivers.get(index)
            .ok_or_else(||error("E_INTERNAL","receiver index mismatch"))?;
        let m=parse_one_receiver(tokens,start,end,head)?;
        let owner=program.actors.iter_mut().find(|actor|actor.name==head.owner)
            .ok_or_else(||error("E_UNLINKED","target receiver has no concrete actor"))?;
        owner.methods.push(m);
    }
    // Compile all target sources by the same production typed checker as
    // legacy actors. Never emit a partial interface or unchecked body.
    program.actor_interface=None;
    Ok(program)
}
