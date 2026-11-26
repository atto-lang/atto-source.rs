//! Ready-only closed-world counted selection. Guest-side FIFO and arbitration,
//! no host imports or compile-time evaluation of the guest result. Bounds are
//! deliberately small; dynamic external producers and general actors reject.
use crate::{Diagnostic,Pos,lexer::{lex,Tok}};
use std::collections::{BTreeMap,BTreeSet};
type Code=Vec<u8>;
type R<T>=Result<T,Diagnostic>;
fn bad(s:impl Into<String>)->Diagnostic{Diagnostic::new(Pos{line:1,col:1},"E_COUNTED_READY_PROFILE",s)}
fn u(c:&mut Code,mut n:u32){loop{let b=(n&127) as u8;n>>=7;c.push(if n==0{b}else{b|128});if n==0{break;}}}
fn s(c:&mut Code,mut n:i64){loop{let b=(n&127)as u8;n>>=7;let end=(n==0&&b&64==0)||(n == -1&&b&64!=0);c.push(if end{b}else{b|128});if end{break;}}}
fn num(n:i64)->Code{let mut c=vec![0x42];s(&mut c,n);c}
fn get(i:u32)->Code{let mut c=vec![0x20];u(&mut c,i);c}
fn set(c:&mut Code,i:u32){c.push(0x21);u(c,i);}
fn assign(c:&mut Code,i:u32,v:Code){c.extend(v);set(c,i);}
fn op(mut a:Code,b:Code,opcode:u8)->Code{a.extend(b);a.push(opcode);a}
fn call(mut a:Code,fn_id:u8)->Code{a.extend([0x10,fn_id]);a}
fn trap(c:&mut Code,n:u8){c.extend([0x41,n,0x24,0,0]);}
fn guard(c:&mut Code,cond:Code,n:u8){c.extend(cond);c.extend([4,0x40]);trap(c,n);c.push(0x0b);}
fn addr(base:u32,index:Code)->Code{let mut c=op(num(base as i64),op(index,num(8),0x7e),0x7c);c.push(0xa7);c}
fn load(base:u32,index:Code)->Code{let mut c=addr(base,index);c.extend([0x29,3,0]);c}
fn store(c:&mut Code,base:u32,index:Code,value:Code){c.extend(addr(base,index));c.extend(value);c.extend([0x37,3,0]);}
#[derive(Clone,Debug)]
struct Channel{base:u32,cap:i64,head:u32,len:u32}
#[derive(Clone,Debug)]
struct Array{base:u32,len:u32}
#[derive(Clone,Debug)]
struct Arm{name:String,projection:Code}
#[derive(Clone,Debug)]
struct Batch{array:Array,arms:Vec<Arm>,fair:bool,project:u32}
#[derive(Clone,Debug)]
enum Kind{Int(u32),Channel(Channel),Future(Code),Array(Array),Pending(Batch)}
#[derive(Clone,Debug)]
struct Slot{kind:Kind,moved:bool,mutable:bool}
enum Value{Int(Code),Bool(Code),Array(Array,Code)}
impl Value{
 fn int(self)->R<Code>{if let Self::Int(c)=self{Ok(c)}else{Err(bad("expected int"))}}
 fn boolean(self)->R<Code>{if let Self::Bool(c)=self{Ok(c)}else{Err(bad("expected bool"))}}
}
struct Parser{tokens:Vec<String>,pos:usize,slots:BTreeMap<String,Slot>,locals:u32,memory:u32,budget:u32,depth:u32,sites:u32,pending:Option<String>,pure:bool}
impl Parser{
 fn new(source:&str)->R<Self>{
  if source.len()>crate::SOURCE_CAP||source.bytes().any(|b|b>=127||(b<32&&!matches!(b,b'\n'|b'\r'|b'\t'))){return Err(bad("bounded ASCII source required"))}
  let ts=lex(source)?;let mut tokens=vec![];let mut i=0;
  while i<ts.len(){
   let t=&ts[i];let text=match &t.tok{
    Tok::Ident(n)=>n.clone(),Tok::Int(Some(n))=>n.to_string(),
    Tok::LParen=>"(".into(),Tok::RParen=>")".into(),Tok::LBrace=>"{".into(),Tok::RBrace=>"}".into(),Tok::LBracket=>"[".into(),Tok::RBracket=>"]".into(),
    Tok::Semi=>";".into(),Tok::Comma=>",".into(),Tok::Colon=>":".into(),Tok::Arrow=>"->".into(),Tok::Dot=>".".into(),Tok::Assign=>"=".into(),
    Tok::EqEq=>"==".into(),Tok::NotEq=>"!=".into(),Tok::Lt=>"<".into(),Tok::Le=>"<=".into(),Tok::Gt=>">".into(),Tok::Ge=>">=".into(),Tok::Plus=>"+".into(),Tok::Minus=>"-".into(),Tok::Star=>"*".into(),Tok::Eof=>"<eof>".into(),
    Tok::Amp|Tok::Pipe if ts.get(i+1).is_some_and(|next|next.tok==t.tok&&next.pos.line==t.pos.line&&next.pos.col==t.pos.col+1)=>{i+=1;if t.tok==Tok::Amp{"&&".into()}else{"||".into()}},
    _=>return Err(bad("unsupported token in ready-only profile")),
   };tokens.push(text);i+=1;
  }
  Ok(Self{tokens,pos:0,slots:BTreeMap::new(),locals:0,memory:0,budget:4096,depth:0,sites:0,pending:None,pure:false})
 }
 fn peek(&self)->&str{self.tokens.get(self.pos).map(String::as_str).unwrap_or("<eof>")}
 fn eat(&mut self,t:&str)->bool{if self.peek()==t{self.pos+=1;true}else{false}}
 fn need(&mut self,t:&str)->R<()>{if self.eat(t){Ok(())}else{Err(bad(format!("expected {t}")))}}
 fn name(&mut self)->R<String>{
  let n=self.peek();if n.is_empty()||!n.as_bytes()[0].is_ascii_alphabetic()&&n.as_bytes()[0]!=b'_'||!n.bytes().all(|b|b.is_ascii_alphanumeric()||b==b'_')||"pub suspends fnc main int bool val let const Channel Future Array new writech readch select nb await when fair first random default over end return if then fi true false".split_whitespace().any(|w|w==n){return Err(bad("lexical name required"))}
  let n=n.to_owned();self.pos+=1;Ok(n)
 }
 fn literal(&mut self)->R<i64>{let n=self.peek().parse().map_err(|_|bad("integer literal required"))?;self.pos+=1;Ok(n)}
 fn small(&mut self,max:i64)->R<i64>{let n=self.literal()?;if !(0..=max).contains(&n){Err(bad("literal limit"))}else{Ok(n)}}
 fn local(&mut self)->R<u32>{if self.locals>=2048{return Err(bad("local budget"))}let i=self.locals;self.locals+=1;Ok(i)}
 fn bytes(&mut self,n:u32)->R<u32>{if self.memory+n>65536{return Err(bad("fixed guest memory budget"))}let p=self.memory;self.memory+=n;Ok(p)}
 fn slot(&self,n:&str)->R<Slot>{self.slots.get(n).filter(|s|!s.moved).cloned().ok_or_else(||bad("unknown or moved lexical binding"))}
 fn ty(&mut self)->R<&'static str>{
  if self.eat("int"){return Ok("int")}
  if self.eat("Channel"){self.need("<")?;self.need("int")?;self.need(">")?;return Ok("channel")}
  if self.eat("Future"){self.need("<")?;if self.eat("Array"){self.need("<")?;self.need("int")?;self.need(">")?;self.need(">")?;return Ok("pending")}self.need("int")?;self.need(">")?;return Ok("future")}
  if self.eat("Array"){let bracket=self.eat("[");if !bracket{self.need("<")?}self.need("int")?;self.need(if bracket{"]"}else{">"})?;return Ok("array")}
  Err(bad("unsupported lexical type"))
 }
 fn prec(t:&str)->u8{match t{"||"=>1,"&&"=>2,"=="|"!="=>3,"<"|">"|"<="|">="=>4,"+"|"-"=>5,"*"=>6,_=>0}}
 fn expr(&mut self,min:u8,binder:Option<(&str,u32)>)->R<Value>{
  if self.budget==0||self.depth>=64{return Err(bad("expression budget"))}self.budget-=1;self.depth+=1;
  let mut lhs=self.primary(binder)?;
  loop{let p=Self::prec(self.peek());if p==0||p<min{break}let tok=self.peek().to_owned();self.pos+=1;let rhs=self.expr(p+1,binder)?;
   lhs=match tok.as_str(){
    "+"|"-"|"*"=>{let mut c=lhs.int()?;c.extend(rhs.int()?);Value::Int(call(c,if tok=="+"{0}else if tok=="-"{1}else{2}))},
    "&&"|"||"=>{let mut c=lhs.boolean()?;c.extend([4,0x7f]);if tok=="&&"{c.extend(rhs.boolean()?);c.extend([5,0x41,0]);}else{c.extend([0x41,1,5]);c.extend(rhs.boolean()?);}c.push(0x0b);Value::Bool(c)},
    _=>Value::Bool(op(lhs.int()?,rhs.int()?,match tok.as_str(){"=="=>0x51,"!="=>0x52,"<"=>0x53,">"=>0x55,"<="=>0x57,">="=>0x59,_=>return Err(bad("operator"))}))
   };
  }self.depth-=1;Ok(lhs)
 }
 fn primary(&mut self,binder:Option<(&str,u32)>)->R<Value>{
  if self.eat("("){let v=self.expr(0,binder)?;self.need(")")?;return Ok(v)}
  if self.eat("-"){return Ok(Value::Int(call(self.expr(7,binder)?.int()?,3)))}
  if self.peek().as_bytes().first().is_some_and(u8::is_ascii_digit){return Ok(Value::Int(num(self.literal()?)))}
  if let Some((n,project))=binder{let got=self.name()?;if n=="@count"{if let Kind::Int(i)=self.slot(&got)?.kind{return Ok(Value::Int(get(i)))}}else if got==n{return Ok(Value::Int(get(project)))}return Err(bad("pure projection/count binding required"))}
  if self.eat("await"){
   if self.pure{return Err(bad("effect in condition"))}
   let legacy=self.eat("nb");
   if self.eat("select"){let (b,mut code)=self.batch()?;code.extend(self.drive(&b)?);return Ok(Value::Array(b.array,code))}
   if legacy{return Err(bad("nb requires select"))}
   let n=self.name()?;let slot=self.slot(&n)?;
   match slot.kind{
    Kind::Pending(b) if self.pending.as_deref()==Some(&n)=>{self.pending=None;self.slots.get_mut(&n).unwrap().moved=true;let code=self.drive(&b)?;Ok(Value::Array(b.array,code))},
    Kind::Future(code) if self.pending.is_none()=>{self.slots.get_mut(&n).unwrap().moved=true;Ok(Value::Int(code))},
    _=>Err(bad("expected live awaited Future")),
   }
  }else if self.eat("readch"){
   if self.pure||self.pending.is_some(){return Err(bad("channel access before await or in condition"))}
   let n=self.name()?;if let Kind::Channel(ch)=self.slot(&n)?.kind{Ok(Value::Int(self.read(&ch)?))}else{Err(bad("expected Channel"))}
  }else{
   let n=self.name()?;match self.slot(&n)?.kind{
    Kind::Int(i)=>Ok(Value::Int(get(i))),
    Kind::Array(a)=>{if self.eat("["){let idx=self.small(15)?;self.need("]")?;let mut c=vec![];guard(&mut c,op(num(idx),get(a.len),0x59),10);c.extend(load(a.base,num(idx)));Ok(Value::Int(c))}else{self.need(".")?;self.need("size")?;Ok(Value::Int(get(a.len)))}},
    _=>Err(bad("capability cannot escape scalar expression")),
   }
  }
 }
 fn read(&mut self,ch:&Channel)->R<Code>{
  let tmp=self.local()?;let mut c=vec![];guard(&mut c,op(get(ch.len),num(0),0x51),9);
  assign(&mut c,tmp,load(ch.base,get(ch.head)));assign(&mut c,ch.head,op(op(get(ch.head),num(1),0x7c),num(ch.cap),0x82));
  assign(&mut c,ch.len,op(get(ch.len),num(1),0x7d));c.extend(get(tmp));Ok(c)
 }
 fn batch(&mut self)->R<(Batch,Code)>{
  if self.pending.is_some(){return Err(bad("await outstanding batch first"))}
  if self.eat("<"){self.need("int")?;self.need(">")?}
  self.need("(")?;let begin=self.pos;let count=self.expr(0,Some(("@count",0)))?.int()?;let end=self.pos;self.need(")")?;
  let literal=if end==begin+1{self.tokens[begin].parse::<i64>().ok()}else{None};
  if literal.is_some_and(|n|n>16)||(end==begin+2&&self.tokens[begin]=="-"&&self.tokens[begin+1].parse::<i64>().is_ok()){return Err(bad("SelectCountInvalid"))}
  let fair=self.eat("fair");if !fair{self.eat("first");}let over=self.eat("over");if !over{self.need("{")?}
  let project=self.local()?;let mut arms=vec![];let mut futures=BTreeSet::new();
  while self.eat("when"){
   if arms.len()==8{return Err(bad("arm quota"))}
   let channel=self.eat("readch");if !channel{self.need("await")?}
   let n=self.name()?;match self.slot(&n)?.kind{Kind::Channel(_) if channel=>{},Kind::Future(_) if !channel=>{futures.insert(n.clone());},_=>return Err(bad("wrong lexical source type"))}
   self.need(":")?;self.need("int")?;let binder=self.name()?;self.need("->")?;self.need("{")?;self.need("return")?;
   let projection=self.expr(0,Some((&binder,project)))?.int()?;self.need(";")?;self.need("}")?;arms.push(Arm{name:n,projection});
  }
  if arms.is_empty(){return Err(bad("at least one arm required"))}self.need(if over{"end"}else{"}"})?;self.sites+=1;
  if literal!=Some(0){for n in futures{self.slots.get_mut(&n).unwrap().moved=true;}}
  let a=Array{base:self.bytes(16*8)?,len:self.local()?};let mut c=vec![];assign(&mut c,a.len,count);
  guard(&mut c,op(op(get(a.len),num(0),0x53),op(get(a.len),num(16),0x55),0x72),8);
  Ok((Batch{array:a,arms,fair,project},c))
 }
 fn drive(&mut self,b:&Batch)->R<Code>{
  // No source plan is read before the count check or for runtime zero.
  let (round,start,scan,winner,candidate)=(self.local()?,self.local()?,self.local()?,self.local()?,self.local()?);
  let mut live=BTreeMap::new();
  for arm in &b.arms{if matches!(self.slots[&arm.name].kind,Kind::Future(_))&&!live.contains_key(&arm.name){live.insert(arm.name.clone(),self.local()?);}}
  let mut c=vec![];assign(&mut c,round,num(0));assign(&mut c,start,num(0));
  for id in live.values(){assign(&mut c,*id,num(1));}
  c.extend([2,0x40,3,0x40]); // block, round loop
  c.extend(op(get(round),get(b.array.len),0x59));c.extend([0x0d,1]);
  assign(&mut c,scan,num(0));assign(&mut c,winner,num(-1));
  c.extend([2,0x40,3,0x40]); // scan loop
  c.extend(op(op(get(scan),num(b.arms.len() as i64),0x59),op(get(winner),num(0),0x59),0x72));c.extend([0x0d,1]);
  assign(&mut c,candidate,op(op(get(start),get(scan),0x7c),num(b.arms.len() as i64),0x82));
  for (i,arm) in b.arms.iter().enumerate(){
   let kind=self.slots[&arm.name].kind.clone();
   let available=match &kind{Kind::Channel(ch)=>op(get(ch.len),num(0),0x55),Kind::Future(_)=>op(get(live[&arm.name]),num(0),0x55),_=>return Err(bad("invalid arm plan"))};
   c.extend(op(op(get(candidate),num(i as i64),0x51),available,0x71));c.extend([4,0x40]);
   match kind{Kind::Channel(ch)=>{let value=self.read(&ch)?;assign(&mut c,b.project,value);},Kind::Future(value)=>{assign(&mut c,b.project,value);assign(&mut c,live[&arm.name],num(0));},_=>return Err(bad("invalid arm"))}
   store(&mut c,b.array.base,get(round),arm.projection.clone());assign(&mut c,winner,num(i as i64));c.push(0x0b);
  }
  assign(&mut c,scan,op(get(scan),num(1),0x7c));c.extend([0x0c,0,0x0b,0x0b]); // scan
  guard(&mut c,op(get(winner),num(0),0x53),9);
  if b.fair{assign(&mut c,start,op(op(get(winner),num(1),0x7c),num(b.arms.len() as i64),0x82));}
  assign(&mut c,round,op(get(round),num(1),0x7c));c.extend([0x0c,0,0x0b,0x0b]);Ok(c)
 }
 fn bind(&mut self,mutable:bool,out:&mut Code)->R<()>{
  let ty=self.ty()?;let n=self.name()?;if self.slots.contains_key(&n)||self.slots.len()>=128||(mutable&&ty!="int"){return Err(bad("invalid lexical binding"))}self.need("=")?;
  let kind=match ty{
   "channel"=>{if self.pending.is_some(){return Err(bad("await batch first"))}self.need("Channel")?;self.need(".")?;self.need("new")?;self.need("<")?;self.need("int")?;self.need(">")?;self.need("(")?;let cap=self.small(64)?;self.need(")")?;if cap==0{return Err(bad("zero capacity"))}let ch=Channel{base:self.bytes(cap as u32*8)?,cap,head:self.local()?,len:self.local()?};assign(out,ch.head,num(0));assign(out,ch.len,num(0));Kind::Channel(ch)},
   "future"=>{if self.pending.is_some(){return Err(bad("await batch first"))}self.need("select")?;self.need("<")?;self.need("int")?;self.need(">")?;let over=self.eat("over");if !over{self.need("{")?}self.need("default")?;self.need(":")?;self.need("->")?;self.need("{")?;self.need("return")?;let v=self.expr(0,Some(("$no_binding",0)))?.int()?;self.need(";")?;self.need("}")?;self.need(if over{"end"}else{"}"})?;Kind::Future(v)},
   "pending"=>{self.eat("nb");self.need("select")?;let (b,code)=self.batch()?;out.extend(code);self.pending=Some(n.clone());Kind::Pending(b)},
   "int"=>{let v=self.expr(0,None)?.int()?;let i=self.local()?;assign(out,i,v);Kind::Int(i)},
   "array"=>{if let Value::Array(a,c)=self.expr(0,None)?{out.extend(c);Kind::Array(a)}else{return Err(bad("expected counted array"))}},
   _=>return Err(bad("type")),
  };self.need(";")?;self.slots.insert(n,Slot{kind,moved:false,mutable});Ok(())
 }
 fn parse(mut self)->R<Vec<u8>>{
  self.eat("pub");self.need("suspends")?;self.need("fnc")?;self.need("main")?;self.need("(")?;self.need(")")?;self.need(":")?;self.need("int")?;self.need("{")?;
  let mut out=vec![];let mut statements=0;
  while self.peek()!="return"{
   statements+=1;if statements>256{return Err(bad("statement quota"))}
   if self.eat("val"){self.bind(false,&mut out)?}
   else if self.eat("let"){self.bind(true,&mut out)?}
   else if self.eat("writech"){
    if self.pending.is_some(){return Err(bad("await batch first"))}let n=self.name()?;let ch=match self.slot(&n)?.kind{Kind::Channel(c)=>c,_=>return Err(bad("expected Channel"))};self.need(",")?;let v=self.expr(0,None)?.int()?;self.need(";")?;
    let tmp=self.local()?;assign(&mut out,tmp,v);guard(&mut out,op(get(ch.len),num(ch.cap),0x59),9);
    store(&mut out,ch.base,op(op(get(ch.head),get(ch.len),0x7c),num(ch.cap),0x82),get(tmp));assign(&mut out,ch.len,op(get(ch.len),num(1),0x7c));
   }else if self.eat("if"){
    if self.pending.is_some(){return Err(bad("await batch first"))}self.pure=true;let cond=self.expr(0,None)?.boolean()?;self.pure=false;
    let before=self.slots.clone();self.need("then")?;self.need("return")?;let v=self.expr(0,None)?.int()?;self.need(";")?;self.need("fi")?;self.slots=before;out.extend(cond);out.extend([4,0x40]);out.extend(v);out.extend([0x0f,0x0b]);
   }else if self.tokens.get(self.pos+1).is_some_and(|t|t=="="){
    let n=self.name()?;let s=self.slot(&n)?;if !s.mutable{return Err(bad("immutable binding"))}let i=if let Kind::Int(i)=s.kind{i}else{return Err(bad("scalar assignment"))};self.need("=")?;let v=self.expr(0,None)?.int()?;self.need(";")?;assign(&mut out,i,v);
   }else{return Err(bad("unsupported statement"))}
  }
  self.need("return")?;out.extend(self.expr(0,None)?.int()?);out.push(0x0f);self.need(";")?;self.need("}")?;if self.peek()!="<eof>"||self.sites==0||self.pending.is_some(){return Err(bad("one closed-world counted main without escaped batch required"))}
  let bytes=crate::wasm::emit_counted_ready(self.locals,out);if bytes.len()>crate::wasm::MAX_OUTPUT{return Err(bad("output budget"))}Ok(bytes)
 }
}
pub(crate) fn compile(source:&str)->R<Vec<u8>>{Parser::new(source)?.parse()}
