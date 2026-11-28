use atto_compiler::compile;
use std::io::Write;
use std::process::{Command,Stdio};
#[test]
#[ignore = "requires Node.js; run explicitly in exact-source CI"]
fn real_wasm_arrow_guests() {
    for body in [
        "let b = 21; let a <- b; return a+b;",
        "let b = 21; let int a <- b; return a+b;",
        "let a = 0; let b = 21; set a <- b; return a+b;",
        "let a = 0; set a <- 42; return a;",
        "let a = 0; set a <- 21+21; return a;",
        "let a = 21; let a <- a; return a+a;",
        "let a = 21; let a = a+21; return a;",
    ] {
        let bytes=compile(&format!("pub fnc main(): int {{ {body} }}")).expect(body);
        let mut child=Command::new("node").args(["-e",r#"
          const fs=require('node:fs');
          const bytes=fs.readFileSync(0);
          const m=new WebAssembly.Module(bytes);
          if(WebAssembly.Module.imports(m).length) throw Error('unexpected import');
          const i=new WebAssembly.Instance(m,{});
          if(i.exports.main()!==42n) throw Error('incorrect guest result');
          if(i.exports.panic_code && i.exports.panic_code.value!==0) throw Error('guest panic');
          console.log('ARROW_WASM_GUEST_PASS');
        "#]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(&bytes).unwrap();
        let out=child.wait_with_output().unwrap();
        assert!(out.status.success(),"{body}: {}",String::from_utf8_lossy(&out.stderr));
        assert_eq!(out.stdout,b"ARROW_WASM_GUEST_PASS\n");
        println!("ARROW_WASM_GUEST_PASS {body}");
    }
}
