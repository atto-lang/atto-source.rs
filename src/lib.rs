//! Experimental AttoLang scalar frontend and typed i32 IR for browser Wasm.
//! This is a canonical frontend candidate only, NOT whole-language conformance.
use std::sync::Mutex;
const SOURCE_CAP: usize=4096;
const LOCALS_CAP: usize=64;
const CODE_CAP: usize=8192;
const WASM_CAP: usize=16384;
const DEPTH_CAP: u16=128;

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
enum K { Eof, Ident, Number, Fnc, Return, Const, Let, Mut, Int,
    Open, Close, Begin, End, Semi, Eq, Plus, Minus, Mul, Div, Arrow }
#[derive(Clone,Copy,Debug)]
struct Tok<'a> { k:K, text:&'a str, line:u32, col:u32, value:i32 }
#[derive(Clone,Debug,PartialEq,Eq)]
pub struct Diagnostic { pub line:u32, pub column:u32, pub message:&'static str }
fn err(line:u32,column:u32,message:&'static str)->Diagnostic { Diagnostic{line,column,message} }
fn alpha(b:u8)->bool {b.is_ascii_alphabetic() || b==b'_'}
fn alnum(b:u8)->bool {alpha(b)||b.is_ascii_digit()}

fn lex(source:&str)->Result<Vec<Tok<'_>>,Diagnostic>{
    let b=source.as_bytes();
    let (mut i,mut line,mut col)=(0usize,1u32,1u32);
    let mut out=Vec::new();
    while i<b.len(){
        let c=b[i];
        if matches!(c,b' '|b'\n'|b'\r'|b'\t'){
            if c==b'\n' {line+=1;col=1} else {col+=1}
            i+=1;continue
        }
        if c==b'/' && b.get(i+1)==Some(&b'/'){
            while i<b.len() && b[i]!=b'\n' {i+=1;col+=1}
            continue
        }
        let (start,sl,sc)=(i,line,col);
        let mut value=0i32;
        let k=if alpha(c){
            i+=1;while i<b.len() && alnum(b[i]) {i+=1}
            match &source[start..i] {
                "fnc"=>K::Fnc,"return"=>K::Return,"const"=>K::Const,
                "let"=>K::Let,"mut"=>K::Mut,"int"=>K::Int,_=>K::Ident
            }
        }else if c.is_ascii_digit(){
            i+=1;while i<b.len() && b[i].is_ascii_digit() {i+=1}
            let mut n=0u32;
            for digit in &b[start..i] {
                n=n.checked_mul(10).and_then(|n|n.checked_add((digit-b'0') as u32))
                    .ok_or(err(sl,sc,"E_INTEGER_RANGE: integer must fit signed i32"))?;
                if n>i32::MAX as u32{return Err(err(sl,sc,"E_INTEGER_RANGE: integer must fit signed i32"))}
            }
            value=n as i32;K::Number
        }else{
            i+=1;
            match c {
                b'('=>K::Open,b')'=>K::Close,b'{'=>K::Begin,b'}'=>K::End,
                b';'=>K::Semi,b'='=>K::Eq,b'+'=>K::Plus,
                b'*'=>K::Mul,b'/'=>K::Div,
                b'-' if b.get(i)==Some(&b'>')=>{i+=1;K::Arrow},
                b'-'=>K::Minus,
                _=>return Err(err(sl,sc,"E_TOKEN: unsupported token"))
            }
        };
        col+=(i-start) as u32;
        out.push(Tok{k,text:&source[start..i],line:sl,col:sc,value});
    }
    out.push(Tok{k:K::Eof,text:"",line,col,value:0});
    Ok(out)
}

/// Typechecked stack IR. Every operand is i32; unsupported types reject.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum I32Op { Const(i32), Get(u32), Set(u32), Add, Sub, Mul, DivS }
#[derive(Clone,Debug,PartialEq,Eq)]
pub struct TypedModule { pub locals:u32, pub ops:Vec<I32Op> }
#[derive(Clone,Debug)]
struct Local<'a>{ name:&'a str, mutable:bool, ready:bool }
struct Parser<'a>{ t:Vec<Tok<'a>>, at:usize, locals:Vec<Local<'a>>, ops:Vec<I32Op> }
impl<'a> Parser<'a>{
    fn now(&self)->Tok<'a>{self.t[self.at]}
    fn is(&self,k:K)->bool{self.now().k==k}
    fn bad(&self,s:&'static str)->Diagnostic{let t=self.now();err(t.line,t.col,s)}
    fn pop(&mut self)->Tok<'a>{let t=self.now();if t.k!=K::Eof{self.at+=1}t}
    fn need(&mut self,k:K,s:&'static str)->Result<Tok<'a>,Diagnostic>{
        if !self.is(k){return Err(self.bad(s))}
        Ok(self.pop())
    }
    fn local(&self,s:&str)->Option<usize>{self.locals.iter().position(|b|b.name==s)}
    fn op(&mut self,op:I32Op)->Result<(),Diagnostic>{
        if self.ops.len()>CODE_CAP/2{return Err(self.bad("E_OUTPUT_LIMIT: generated code too large"))}
        self.ops.push(op);Ok(())
    }
    fn atom(&mut self,depth:u16)->Result<(),Diagnostic>{
        if depth>DEPTH_CAP{return Err(self.bad("E_NESTING_LIMIT: expression too deep"))}
        match self.now().k{
            K::Number=>{let n=self.pop().value;self.op(I32Op::Const(n))}
            K::Ident=>{
                let s=self.pop().text;
                let id=self.local(s).ok_or(self.bad("E_UNBOUND: variable not declared"))?;
                if !self.locals[id].ready {return Err(self.bad("E_UNINITIALIZED: variable used before initialization"))}
                self.op(I32Op::Get(id as u32))
            }
            K::Open=>{
                self.pop();self.expr(0,depth+1)?;
                self.need(K::Close,"E_PAREN: expected ')'")?;Ok(())
            }
            K::Minus=>{
                self.pop();self.op(I32Op::Const(0))?;self.atom(depth+1)?;self.op(I32Op::Sub)
            }
            _=>Err(self.bad("E_EXPRESSION: expected integer, variable or parenthesized expression"))
        }
    }
    fn expr(&mut self,min:u8,depth:u16)->Result<(),Diagnostic>{
        self.atom(depth)?;
        loop {
            let (precedence,op)=match self.now().k{
                K::Plus=>(1,I32Op::Add),K::Minus=>(1,I32Op::Sub),
                K::Mul=>(2,I32Op::Mul),K::Div=>(2,I32Op::DivS),_=>break
            };
            if precedence<min {break}
            self.pop();self.expr(precedence+1,depth+1)?;self.op(op)?;
        }
        Ok(())
    }
    fn declaration(&mut self)->Result<(),Diagnostic>{
        let mut mutable=false;
        match self.now().k {
            K::Let|K::Mut=>{mutable=true;self.pop();},
            K::Const=>{self.pop();},_=>{}
        }
        if self.is(K::Int){self.pop();}
        let t=self.need(K::Ident,"E_DECL: expected variable identifier")?;
        if self.local(t.text).is_some(){return Err(self.bad("E_DUPLICATE: duplicate variable"))}
        if self.locals.len()>=LOCALS_CAP{return Err(self.bad("E_LOCALS_LIMIT: too many variables"))}
        let id=self.locals.len();
        self.locals.push(Local{name:t.text,mutable,ready:false});
        self.need(K::Eq,"E_DECL: expected '='")?;
        self.expr(0,0)?;
        self.need(K::Semi,"E_SEMICOLON: expected ';'")?;
        self.op(I32Op::Set(id as u32))?;
        self.locals[id].ready=true;Ok(())
    }
    fn assignment(&mut self)->Result<(),Diagnostic>{
        let t=self.pop();
        let id=self.local(t.text).ok_or(err(t.line,t.col,"E_UNBOUND: variable not declared"))?;
        if !self.locals[id].mutable{return Err(err(t.line,t.col,"E_IMMUTABLE: rebinding requires let or mut"))}
        self.need(K::Eq,"E_ASSIGN: expected '='")?;
        self.expr(0,0)?;
        self.need(K::Semi,"E_SEMICOLON: expected ';'")?;
        self.op(I32Op::Set(id as u32))
    }
    fn parse(mut self)->Result<TypedModule,Diagnostic>{
        self.need(K::Fnc,"E_PROGRAM: expected fnc")?;
        let name=self.need(K::Ident,"E_PROGRAM: expected main")?;
        if name.text!="main"{return Err(self.bad("E_PROGRAM: only main supported"))}
        self.need(K::Open,"E_PROGRAM: expected '('")?;
        self.need(K::Close,"E_PROGRAM: expected ')'")?;
        self.need(K::Arrow,"E_PROGRAM: expected '->'")?;
        self.need(K::Int,"E_PROGRAM: expected int return type")?;
        self.need(K::Begin,"E_PROGRAM: expected '{'")?;
        while !self.is(K::Return){
            if self.is(K::End) || self.is(K::Eof){return Err(self.bad("E_RETURN: main must end with return expression"))}
            match self.now().k{
                K::Int|K::Const|K::Let|K::Mut=>self.declaration()?,
                K::Ident=>self.assignment()?,
                _=>return Err(self.bad("E_STATEMENT: expected declaration, assignment or return"))
            }
        }
        self.pop();self.expr(0,0)?;
        self.need(K::Semi,"E_SEMICOLON: expected ';'")?;
        if !self.is(K::End) && !self.is(K::Eof){return Err(self.bad("E_RETURN: return must be the final statement"))}
        self.need(K::End,"E_ENTRY: expected '}'")?;
        self.need(K::Eof,"E_TRAILING: trailing tokens after main")?;
        Ok(TypedModule{locals:self.locals.len() as u32,ops:self.ops})
    }
}
pub fn frontend(source:&str)->Result<TypedModule,Diagnostic>{
    if source.len()>SOURCE_CAP{return Err(err(1,1,"E_SOURCE_LIMIT: source exceeds 4096 UTF-8 bytes"))}
    if source.as_bytes().contains(&0){return Err(err(1,1,"E_NUL: source contains NUL"))}
    Parser{t:lex(source)?,at:0,locals:Vec::new(),ops:Vec::new()}.parse()
}
fn uleb(b:&mut Vec<u8>,mut n:u32){
    loop{let mut v=(n&127) as u8;n>>=7;if n!=0{v|=128}b.push(v);if n==0{break}}
}
fn sleb(b:&mut Vec<u8>,mut n:i32){
    loop{
        let mut v=(n&127) as u8;n>>=7;
        let done=(n==0 && v&64==0)||(n==-1 && v&64!=0);
        if !done{v|=128}
        b.push(v);if done{break}
    }
}
fn section(out:&mut Vec<u8>,id:u8,body:&[u8]){
    out.push(id);uleb(out,body.len() as u32);out.extend_from_slice(body)
}
pub fn encode_wasm(ir:&TypedModule)->Result<Vec<u8>,Diagnostic>{
    if ir.locals>LOCALS_CAP as u32 || ir.ops.len()>CODE_CAP/2{
        return Err(err(1,1,"E_OUTPUT_LIMIT: generated code too large"))
    }
    let mut code=Vec::new();
    uleb(&mut code,if ir.locals==0{0}else{1});
    if ir.locals!=0{uleb(&mut code,ir.locals);code.push(0x7f)}
    for op in &ir.ops {
        match *op {
            I32Op::Const(n)=>{code.push(0x41);sleb(&mut code,n)},
            I32Op::Get(n)=>{code.push(0x20);uleb(&mut code,n)},
            I32Op::Set(n)=>{code.push(0x21);uleb(&mut code,n)},
            I32Op::Add=>code.push(0x6a),I32Op::Sub=>code.push(0x6b),
            I32Op::Mul=>code.push(0x6c),I32Op::DivS=>code.push(0x6d)
        }
    }
    code.push(0x0b);
    if code.len()>CODE_CAP{return Err(err(1,1,"E_OUTPUT_LIMIT: generated code too large"))}
    let mut payload=vec![1];uleb(&mut payload,code.len() as u32);payload.extend_from_slice(&code);
    let mut wasm=vec![0,97,115,109,1,0,0,0];
    section(&mut wasm,1,&[1,0x60,0,1,0x7f]);
    section(&mut wasm,3,&[1,0]);
    section(&mut wasm,7,&[1,4,b'm',b'a',b'i',b'n',0,0]);
    section(&mut wasm,10,&payload);
    if wasm.len()>WASM_CAP{return Err(err(1,1,"E_OUTPUT_LIMIT: module too large"))}
    Ok(wasm)
}
pub fn compile_source(source:&str)->Result<Vec<u8>,Diagnostic>{encode_wasm(&frontend(source)?)}

// Browser-facing C ABI. Instance must be single-threaded and only one caller may
// access the input buffer; output pointers remain valid until the next compile.
static mut INPUT:[u8;SOURCE_CAP+1]=[0;SOURCE_CAP+1];
struct ResultState{bytes:Vec<u8>,error:Vec<u8>,line:u32,col:u32}
static RESULT:Mutex<ResultState>=Mutex::new(ResultState{bytes:Vec::new(),error:Vec::new(),line:1,col:1});
#[no_mangle]pub extern "C" fn input_ptr()->u32{std::ptr::addr_of_mut!(INPUT) as *mut u8 as usize as u32}
#[no_mangle]pub extern "C" fn input_capacity()->u32{SOURCE_CAP as u32}
#[no_mangle]pub extern "C" fn result_ptr()->u32{RESULT.lock().unwrap().bytes.as_ptr() as usize as u32}
#[no_mangle]pub extern "C" fn result_length()->u32{RESULT.lock().unwrap().bytes.len() as u32}
#[no_mangle]pub extern "C" fn error_ptr()->u32{RESULT.lock().unwrap().error.as_ptr() as usize as u32}
#[no_mangle]pub extern "C" fn error_line_number()->u32{RESULT.lock().unwrap().line}
#[no_mangle]pub extern "C" fn error_column_number()->u32{RESULT.lock().unwrap().col}
#[no_mangle]pub extern "C" fn compile(length:i32)->i32{
    let result=if length<0 || length as usize>SOURCE_CAP{
        Err(err(1,1,"E_SOURCE_LIMIT: source exceeds 4096 UTF-8 bytes"))
    }else{
        let bytes=unsafe{std::slice::from_raw_parts(std::ptr::addr_of!(INPUT) as *const u8,length as usize)};
        std::str::from_utf8(bytes).map_err(|_|err(1,1,"E_UTF8: invalid UTF-8")).and_then(compile_source)
    };
    let mut state=RESULT.lock().unwrap();
    state.bytes.clear();state.error.clear();state.line=1;state.col=1;
    match result{
        Ok(bytes)=>{state.bytes=bytes;0},
        Err(d)=>{
            state.error.extend_from_slice(d.message.as_bytes());
            state.error.push(0);state.line=d.line;state.col=d.column;1
        }
    }
}
#[cfg(test)]
mod tests{
    use super::*;
    #[test]fn typed_ir(){let ir=frontend("fnc main() -> int { let int x=7; x=x+1; return x*6; }").unwrap();assert_eq!(ir.locals,1);}
    #[test]fn actual_wasm(){let bytes=compile_source("fnc main() -> int { const int x=6; int y=7; return x*y; }").unwrap();assert_eq!(&bytes[0..8],b"\0asm\x01\0\0\0");}
    #[test]fn immutability(){assert!(compile_source("fnc main() -> int {int x=1;x=2;return x;}").unwrap_err().message.contains("E_IMMUTABLE"));}
    #[test]fn initializer_guard(){assert!(compile_source("fnc main() -> int {int x=x+1;return x;}").unwrap_err().message.contains("E_UNINITIALIZED"));}
    #[test]fn different_programs(){assert_ne!(compile_source("fnc main() -> int { return 1; }"),compile_source("fnc main() -> int { return 2; }"));}
    #[test]fn deep_nesting(){let bad=format!("fnc main() -> int {{return {}1{};}}","(".repeat(140),")".repeat(140));assert!(compile_source(&bad).unwrap_err().message.contains("E_NESTING_LIMIT"));}
}
