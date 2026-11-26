//! Native Rust source compiler emits actual guest Wasm; Node is the engine,
//! not a source interpreter. Exact shared corpus blob 193249523cf18ef725f4900c30f3925c5bf8ce4b.
use std::{fs, path::PathBuf, process::Command, time::{SystemTime,UNIX_EPOCH}};
struct Temp(PathBuf);
impl Drop for Temp {fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
#[test]
fn actual_wasm_guests(){
    let dir=Temp(std::env::temp_dir().join(format!("atto-rust-tuple-{}-{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos())));
    fs::create_dir(&dir.0).unwrap();
    let positive=[
        ("flat-three", "pub fnc main(): int { return (5, 3, 5) |> |(a, b, c)| -> a + b + c; }\n", 13),
        ("order-weighted", "pub fnc main(): int { return (2, 7, 11) |> |(a, b, c)| -> 2*a + 3*b + 4*c; }\n", 69),
        ("discard-middle", "pub fnc main(): int { return (5, 99, 8) |> |(a, _, c)| -> a + c; }\n", 13),
        ("discard-first", "pub fnc main(): int { return (99, 8) |> |(_, b)| -> b + 3; }\n", 11),
        ("discard-last", "pub fnc main(): int { return (6, 99) |> |(a, _)| -> a + 5; }\n", 11),
        ("discard-all", "pub fnc main(): int { return (5, 8) |> |(_, _)| -> 19; }\n", 19),
        ("singleton", "pub fnc main(): int { return (9,) |> |(a,)| -> a; }\n", 9),
        ("empty", "pub fnc main(): int { return () |> |()| -> 7; }\n", 7),
        ("typed-leaves", "pub fnc main(): int { return (5, 8) |> |(a: int, b: int)| -> a + b; }\n", 13),
        ("parenthesized-stage", "pub fnc main(): int { return ((5, 8) |> |(a, b)| -> a + b) + 1; }\n", 14),
        ("nested-scalar-stage", "pub fnc main(): int { return (5, 8) |> |(a, b)| -> ((a, 2) |> |(x, y)| -> x*y) + b; }\n", 18),
        ("signed-values", "pub fnc main(): int { return (-5, 8, 12) |> |(a, b, c)| -> a + b + c; }\n", 15),
        ("maximum-arity", "pub fnc main(): int { return (1, 2, 3, 4, 5, 6, 7, 8) |> |(a,b,c,d,e,f,g,h)| -> a+b+c+d+e+f+g+h; }\n", 36),
        ("repeat","pub fnc main(): int { return (5,8) |> |(a,_)| -> a+a; }\n",10),
    ];
    // Keep execution required, never skip if the engine is unavailable.
    let engine=dir.0.join("guest.cjs");
    fs::write(&engine,r#"const fs=require('node:fs');const assert=require('node:assert/strict');
const m=new WebAssembly.Module(fs.readFileSync(process.argv[2]));
const imports={};for(const i of WebAssembly.Module.imports(m)){assert.equal(i.kind,'function');(imports[i.module]??={})[i.name]=()=>{throw Error('unexpected host call '+i.name)};}
const g=new WebAssembly.Instance(m,imports);assert.equal(typeof g.exports.main,'function');
if(process.argv[3]==='trap'){assert.throws(()=>g.exports.main(),WebAssembly.RuntimeError);}else{assert.equal(g.exports.main(),BigInt(process.argv[3]));}"#).unwrap();
    for (name,source,want) in positive {
        let bytes=atto_compiler::compile(source).unwrap_or_else(|d|panic!("{name}: {d}"));
        let wasm=dir.0.join(format!("{name}.wasm"));fs::write(&wasm,bytes).unwrap();
        let output=Command::new("node").arg(&engine).arg(&wasm).arg(want.to_string()).output().unwrap();
        assert!(output.status.success()&&output.stdout.is_empty()&&output.stderr.is_empty(),"{name}: {output:?}");
    }
    for (name,expr) in [("overflow-first","(9223372036854775807+1,8) |> |(_,b)| -> b"),("overflow-last","(5,9223372036854775807+1) |> |(a,_)| -> a")] {
        let source=format!("pub fnc main(): int {{ return {expr}; }}\n");
        let bytes=atto_compiler::compile(&source).unwrap_or_else(|d|panic!("{name}: {d}"));
        let wasm=dir.0.join(format!("{name}.wasm"));fs::write(&wasm,bytes).unwrap();
        let output=Command::new("node").arg(&engine).arg(&wasm).arg("trap").output().unwrap();
        assert!(output.status.success(),"{name}: {output:?}");
    }
    for expr in ["(1,2) |> |(a,a)| -> a","(1,2) |> |(a,b,c)| -> a","(1,2) |> |(a,_)| -> _","(1,) |> |(a)| -> a","(1,2) |> |(_:int,b)| -> b","(true,2) |> |(a,b)| -> b","(1,2) |> |(a,b)| -> missing","((1,2),3) |> |(a,b)| -> b"] {
        let source=format!("pub fnc main(): int {{ return {expr}; }}");
        let d=atto_compiler::compile(&source).unwrap_err();
        assert_eq!(d.code,"E_TUPLE_PATTERN","{expr}: {d}");
    }
    println!("RUST_TUPLE_NATIVE_PASS 13 shared + 1 positive + 2 overflow + 8 rejection");
}
