//! Cross-backend Atto browser route v1 ABI contract.
//! The browser broker sends signed i64 little-endian bytes, then invokes
//! actor_receive(i32 type, i64 value, i64 sequence) on the selected guest.
use atto_compiler::compile;

fn leb(data: &[u8], at: &mut usize) -> u32 {
    let mut value = 0u32;
    for shift in (0..35).step_by(7) {
        let byte = *data.get(*at).expect("truncated compiler-produced Wasm");
        *at += 1;
        value |= ((byte & 127) as u32) << shift;
        if byte & 128 == 0 { return value }
    }
    panic!("invalid compiler-produced Wasm")
}
fn name<'a>(data: &'a [u8], at: &mut usize) -> &'a [u8] {
    let n = leb(data, at) as usize;
    let end = at.checked_add(n).expect("name length overflow");
    let text = data.get(*at..end).expect("truncated name");
    *at = end;
    text
}
fn section(wasm: &[u8], desired: u8) -> Option<&[u8]> {
    assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
    let mut at=8;
    while at<wasm.len() {
        let section_id=wasm[at];at+=1;
        let size=leb(wasm,&mut at) as usize;
        let end=at.checked_add(size).expect("section length overflow");
        let bytes=wasm.get(at..end).expect("truncated section");
        if section_id==desired { return Some(bytes) }
        at=end;
    }
    None
}
#[test]
fn compiler_actor_receiver_uses_route_v1_i32_i64_i64_signature() {
    let source = r#"
define actor Counter as
  let int total = 0;
  receive(ActorMail<int> mail): void {
    self.total = self.total + mail.value;
    return;
  }
end
pub fnc main(): int {
  val c = spawn Counter();
  c.send(40);
  return 0;
}
"#;
    let wasm=compile(source).expect("browser actor fixture compiles");
    let types_bytes=section(&wasm,1).expect("type section");
    let mut at=0;
    let count=leb(types_bytes,&mut at);
    let mut signatures=Vec::new();
    for _ in 0..count {
        assert_eq!(types_bytes[at],0x60);at+=1;
        let n=leb(types_bytes,&mut at) as usize;
        let params=types_bytes[at..at+n].to_vec();at+=n;
        let m=leb(types_bytes,&mut at) as usize;
        let returns=types_bytes[at..at+m].to_vec();at+=m;
        signatures.push((params,returns));
    }
    let imported=if let Some(bytes)=section(&wasm,2) {
        let mut pos=0;
        let n=leb(bytes,&mut pos);
        for _ in 0..n {
            name(bytes,&mut pos);name(bytes,&mut pos);
            assert_eq!(bytes[pos],0,"browser guest must import only functions");
            pos+=1;
            let _type_index=leb(bytes,&mut pos);
        }
        n
    }else{0};
    let funcs=section(&wasm,3).expect("function section");
    let mut pos=0;
    let n=leb(funcs,&mut pos);
    let func_type_ids:Vec<u32>=(0..n).map(|_|leb(funcs,&mut pos)).collect();
    let exports=section(&wasm,7).expect("export section");
    let mut pos=0;
    let n=leb(exports,&mut pos);
    let mut receiver=None;
    for _ in 0..n {
        let name=name(exports,&mut pos);
        let kind=exports[pos];pos+=1;
        let index=leb(exports,&mut pos);
        if name==b"actor_receive" {
            assert_eq!(kind,0,"actor_receive must be a function");
            receiver=Some(index);
        }
    }
    let index=receiver.expect("compiler must export actor_receive");
    let local=index.checked_sub(imported).expect("not a locally emitted receiver");
    let type_id=func_type_ids[local as usize] as usize;
    assert_eq!(signatures[type_id],(vec![0x7f,0x7e,0x7e],vec![]),
        "route v1 ABI actor_receive(i32,i64,i64)->void");
}
#[test]
fn browser_route_i64_wire_is_signed_eight_byte_little_endian() {
    for value in [i64::MIN,-1024,-1,0,1,42,1024,i64::MAX] {
        let bytes=value.to_le_bytes();
        assert_eq!(bytes.len(),8);
        assert_eq!(i64::from_le_bytes(bytes),value);
    }
    assert_eq!(42_i64.to_le_bytes(),[42,0,0,0,0,0,0,0]);
    assert_eq!((-1_i64).to_le_bytes(),[255;8]);
}
