use atto_compiler::{actor_bounded_checkpoint_v2::{Field,Kind,Schema,Value},lexer,parser::Parser};
fn field(n:&str,k:Kind)->Field {Field{name:n.into(),kind:k}}
fn schema()->Schema {Schema::new("Ledger",vec![field("count",Kind::Int),field("open",Kind::Bool),field("name",Kind::String),field("payload",Kind::Bytes)]).unwrap()}
fn golden()->Vec<u8>{let hex="4154544f53543032025b193c24a8de14db0400010000000000000080020103080000004100c3a9f09f9880040300000000ff8074c4cc420e041ac0";(0..hex.len()).step_by(2).map(|i|u8::from_str_radix(&hex[i..i+2],16).unwrap()).collect()}
fn rehash(b:&mut [u8]){let end=b.len()-8;let h=b[..end].iter().fold(0xcbf29ce484222325u64,|h,x|(h^u64::from(*x)).wrapping_mul(0x100000001b3));b[end..].copy_from_slice(&h.to_le_bytes());}
#[test]
fn golden_and_roundtrips(){
 let s=schema();assert_eq!(s.fingerprint(),0xdb14dea8243c195b);
 let v=vec![Value::Int(i64::MIN),Value::Bool(true),Value::String("A\0é😀".into()),Value::Bytes(vec![0,255,128])];
 assert_eq!(s.encode(&v).unwrap(),golden());assert_eq!(s.decode(&golden()).unwrap(),v);
 for i in -5000i64..=5000{let v=vec![Value::Int(i),Value::Bool(i%2==0),Value::String("a".repeat(((i+5000)%257) as usize)),Value::Bytes(vec![i as u8,0,255])];assert_eq!(s.decode(&s.encode(&v).unwrap()).unwrap(),v);}
}
#[test]
fn rejects_corruption_and_noncanonical_values(){
 let s=schema();let b=golden();
 for i in 0..b.len(){assert!(s.decode(&b[..i]).is_err());for bit in 0..8{let mut bad=b.clone();bad[i]^=1<<bit;assert!(s.decode(&bad).is_err());}}
 for (i,x) in [(8,1),(29,2),(37,0xc0)]{let mut bad=b.clone();bad[i]=x;rehash(&mut bad);assert!(s.decode(&bad).is_err());}
 let mut bad=b.clone();bad[31..35].fill(255);rehash(&mut bad);assert!(s.decode(&bad).is_err());
 let mut bad=b.clone();bad.insert(b.len()-8,0);rehash(&mut bad);assert!(s.decode(&bad).is_err());
 assert!(s.decode(&vec![0;65537]).is_err());
 assert!(s.encode(&[Value::Int(0),Value::Bool(false),Value::String("😀".repeat(4097)),Value::Bytes(vec![])]).is_err());
}
#[test]
fn exact_limits_and_owned_bytes(){
 let s=Schema::new("Large",["a","b","c","d"].iter().map(|n|field(n,Kind::Bytes)).collect()).unwrap();
 let mut v:Vec<Value>=[16384,16384,16384,16337].iter().map(|n|Value::Bytes(vec![255;*n])).collect();
 let b=s.encode(&v).unwrap();assert_eq!(b.len(),65536);assert_eq!(s.decode(&b).unwrap(),v);
 v[3]=Value::Bytes(vec![0;16338]);assert!(s.encode(&v).is_err());
 let many:Vec<Field>=(0..256).map(|i|field(&format!("f{i}"),Kind::Bool)).collect();
 let s=Schema::new("Many",many.clone()).unwrap();let values=vec![Value::Bool(true);256];assert_eq!(s.decode(&s.encode(&values).unwrap()).unwrap(),values);
 let mut extra=many;extra.push(field("extra",Kind::Bool));assert!(Schema::new("Many",extra).is_err());
 assert!(Schema::new("Bad",vec![field("a",Kind::Int),field("a",Kind::Bool)]).is_err());
 assert!(Schema::new("Bad",vec![field("bad name",Kind::Int)]).is_err());
}
#[test]
fn real_parser_schema_only_no_implicit_array_support(){
 let src="define durable actor Ledger as state count: int = 0; state open: bool = true; state name: string = \"abc\"; end";
 let p=Parser::new(lexer::lex(src).unwrap()).parse_program().unwrap();
 let actual=Schema::for_actor(&p.actors[0]).unwrap();
 let expected=Schema::new("Ledger",vec![field("count",Kind::Int),field("open",Kind::Bool),field("name",Kind::String)]).unwrap();
 assert_eq!(actual,expected);
 let p=Parser::new(lexer::lex("define durable actor A as state socket: Socket = 0; end").unwrap()).parse_program().unwrap();
 assert!(Schema::for_actor(&p.actors[0]).is_err());
}
