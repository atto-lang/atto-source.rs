//! Typed browser IR to ESM. Shares parser/checker and linear-memory layout with
//! the Wasm emitter. Source identifiers never become JavaScript identifiers.
use crate::ast::{BinOp, Type};
use crate::check::{Builtin, PrintPiece, TExpr, TProgram, TStmt, TE};

fn load(ty: Type, address: String) -> String {
    format!("v.{}({},true)", if ty == Type::Int {"getBigInt64"} else {"getInt32"}, address)
}
fn store(ty: Type, address: String, value: String) -> String {
    format!("v.{}({},{},true)", if ty == Type::Int {"setBigInt64"} else {"setInt32"}, address, value)
}
struct Emit<'a> { p: &'a TProgram, state: u32, next: usize, loops: Vec<usize>, void_main: bool }
impl Emit<'_> {
    fn expr(&mut self, e: &TE) -> String {
        match &e.e {
            TExpr::I64(n) => format!("({n}n)"),
            TExpr::Bool(b) => if *b {"1"} else {"0"}.into(),
            TExpr::Local(i) => format!("l{i}"),
            TExpr::Neg(a) => format!("checked(-({}))",self.expr(a)),
            TExpr::Not(a) => format!("Number(!({}))",self.expr(a)),
            TExpr::Bin(op,a,b) => {
                let a=self.expr(a); let b=self.expr(b);
                let op=match op {
                    BinOp::Add=>"+",BinOp::Sub=>"-",BinOp::Mul=>"*",
                    BinOp::Eq=>"===",BinOp::Ne=>"!==",BinOp::Lt=>"<",BinOp::Le=>"<=",
                    BinOp::Gt=>">",BinOp::Ge=>">=",BinOp::And=>"&",BinOp::Or=>"|",
                    BinOp::Div=>unreachable!("division rejected by shared checker"),
                };
                let value=format!("(({a}){op}({b}))");
                if matches!(&e.e,TExpr::Bin(BinOp::Add|BinOp::Sub|BinOp::Mul,_,_)) {format!("checked({value})")}
                else {format!("Number({value})")}
            }
            TExpr::Ternary(c,a,b) => format!("(({})?({}):({}))",self.expr(c),self.expr(a),self.expr(b)),
            TExpr::Call(i,args) => format!("f{i}({})",args.iter().map(|a|self.expr(a)).collect::<Vec<_>>().join(",")),
            TExpr::Builtin(op,a,b) => format!("(({}){}({}))",self.expr(a),match op {Builtin::And=>"&",Builtin::Or=>"|",Builtin::Xor=>"^"},self.expr(b)),
            TExpr::Print{value,newline} => format!("({}.print_{}({}){})", "atto", if value.ty==Type::Int {"i64"}else{"bool"},self.expr(value),if *newline {",atto.print_newline()"}else{""}),
            TExpr::PrintStr{offset,len,newline} => format!("(atto.print_str({offset},{len}){})",if *newline {",atto.print_newline()"}else{""}),
            TExpr::EnumVal(i)=>i.to_string(),
            TExpr::FieldGet{obj,offset}=>{let obj=self.expr(obj);load(e.ty,format!("({obj})+{offset}"))},
            TExpr::GlobalGet(i)=>load(e.ty,(self.state+i*8).to_string()),
            TExpr::Alloc(n)=>format!("alloc({n})"),
            TExpr::Spawn(i)=>format!("atto.actor_spawn({i})"),
            TExpr::Send{target,value}=>format!("atto.actor_send({},{}{})",self.expr(target),if value.ty==Type::Bool {"BigInt("}else{""},format!("{}{}",self.expr(value),if value.ty==Type::Bool {")"}else{""})),
            TExpr::PrintObj{obj,tmp,pieces,newline}=>{
                let mut parts=vec![format!("l{tmp}={}",self.expr(obj))];
                for piece in pieces {parts.push(match piece {
                    PrintPiece::Str(offset,len)=>format!("atto.print_str({offset},{len})"),
                    PrintPiece::Field(offset,ty)=>format!("atto.print_{}({})",if *ty==Type::Int {"i64"}else{"bool"},load(*ty,format!("l{tmp}+{offset}"))),
                });}
                if *newline {parts.push("atto.print_newline()".into());}
                format!("({})",parts.join(","))
            }
        }
    }
    fn stmts(&mut self, ss: &[TStmt]) -> String {
        ss.iter().map(|s|self.stmt(s)).collect()
    }
    fn stmt(&mut self, s: &TStmt) -> String {
        match s {
            TStmt::Set(i,e)=>format!("l{i}={};\n",self.expr(e)),
            TStmt::FieldSet{obj,offset,ty,value}=>{let obj=self.expr(obj);let value=self.expr(value);format!("{};\n",store(*ty,format!("({obj})+{offset}"),value))},
            TStmt::GlobalSet(i,e)=>{let value=self.expr(e);format!("{};\n",store(self.p.globals[*i as usize],(self.state+i*8).to_string(),value))},
            TStmt::Expr(e)=>format!("{};\n",self.expr(e)),
            TStmt::Return(e)=>format!("return {};\n",e.as_ref().map(|e|self.expr(e)).unwrap_or_else(||if self.void_main {"0n"}else{"undefined"}.into())),
            TStmt::If(arms,other)=>{
                let mut s=String::new();
                for (i,(c,body)) in arms.iter().enumerate(){s+=&format!("{}if({}){{\n{}}}",if i==0 {""}else{"else "},self.expr(c),self.stmts(body));}
                if let Some(body)=other {s+=&format!("else{{\n{}}}",self.stmts(body));}
                s+"\n"
            }
            TStmt::Loop{cond,body,update}=>{
                let id=self.next;self.next+=1;self.loops.push(id);
                let cond=cond.as_ref().map(|c|self.expr(c)).unwrap_or("1".into());
                let body=self.stmts(body);let update=self.stmts(update);self.loops.pop();
                // continue exits the body block, then runs the for-update.
                format!("loop{id}:while({cond}){{body{id}:{{\n{body}}}\n{update}}}\n")
            }
            TStmt::Break=>format!("break loop{};\n",self.loops.last().unwrap()),
            TStmt::Continue=>format!("break body{};\n",self.loops.last().unwrap()),
            TStmt::Block(body)=>format!("{{\n{}}}\n",self.stmts(body)),
        }
    }
}
fn quote(s: &str) -> String {
    let mut out=String::from("\"");
    for c in s.chars() {match c {'"'=>out.push_str("\\\""),'\\'=>out.push_str("\\\\"),c if c<' '||c=='\u{2028}'||c=='\u{2029}'=>out+=&format!("\\u{:04x}",c as u32),c=>out.push(c)}}
    out+'"'.to_string().as_str()
}
pub fn emit(p: &TProgram) -> String {
    let state=(p.data.len() as u32+7)&!7;
    let heap=state+p.globals.len() as u32*8;
    let has_memory=!p.data.is_empty()||p.heap||!p.actors.is_empty();
    let pages=if p.heap {crate::wasm::HEAP_PAGES}else{heap.div_ceil(65536).max(1)};
    let mut out=String::from("// Generated by oreslang-source.rs; typed browser IR, checked signed i64.\nexport function instantiate(atto={}){\nconst panic_code={value:0};\nfunction panic(code){panic_code.value=code;throw new Error('E_PANIC:'+code)}\nfunction checked(n){if(n<-(1n<<63n)||n>(1n<<63n)-1n)panic(1);return n}\n");
    if has_memory {
        out+=&format!("const memory=new WebAssembly.Memory({{initial:{pages},maximum:{pages}}});const v=new DataView(memory.buffer);\n");
        if !p.data.is_empty(){out+=&format!("new Uint8Array(memory.buffer).set([{}]);\n",p.data.iter().map(|b|b.to_string()).collect::<Vec<_>>().join(","));}
    }
    if p.heap {out+=&format!("let heap={heap};function alloc(n){{if(heap+n>memory.buffer.byteLength)panic(3);const old=heap;heap+=n;return old}}\n");}
    let mut e=Emit{p,state,next:0,loops:vec![],void_main:false};
    for (i,f) in p.funcs.iter().enumerate(){
        e.void_main=i as u32==p.main&&f.ret==Type::Void;
        out+=&format!("function f{i}({}){{\n",(0..f.params.len()).map(|j|format!("l{j}")).collect::<Vec<_>>().join(","));
        for (j,t) in f.locals.iter().enumerate(){out+=&format!("let l{}={};\n",j+f.params.len(),if *t==Type::Int {"0n"}else{"0"});}
        out+=&e.stmts(&f.body);
        if e.void_main {out+="return 0n;\n";}
        out+="}\n";
    }
    if !p.actors.is_empty(){
        e.void_main=false;out+="function actor_init(type){switch(type){\n";
        for (i,a) in p.actors.iter().enumerate(){out+=&format!("case {i}:\n");for (g,v) in &a.inits {out+=&e.stmt(&TStmt::GlobalSet(*g,v.clone()));}out+="return;\n";}
        out+="default:throw new Error('E_ACTOR: unknown actor type')}}\nfunction actor_receive(type,value,sequence){switch(type){\n";
        for (i,a) in p.actors.iter().enumerate(){if let Some(f)=a.receive {out+=&format!("case {i}:return f{f}(value,sequence);\n");}}
        out+="default:throw new Error('E_ACTOR: no receive handler')}}\n";
    }
    out+=&format!("return {{exports:{{main:f{},panic_code{}{} }}}};\n}}\n",p.main,if has_memory {",memory"}else{""},if !p.actors.is_empty() {",actor_init,actor_receive"}else{""});
    out+=&format!("export const actorNames=Object.freeze([{}]);\n",p.actors.iter().map(|a|quote(&a.name)).collect::<Vec<_>>().join(","));
    out
}
