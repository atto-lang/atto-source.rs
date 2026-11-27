use atto_compiler::compile;
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn keyword_conditionals_and_existing_ternaries_execute() {
    for source in [
        "pub fnc main(): int { return if true then 42 else 7 fi; }",
        "pub fnc main(): int { return if false then 7 else 42 fi; }",
        "pub fnc main(): int { return if false then 7 elif true then 42 else 0 fi; }",
        "pub fnc main(): int { return if true then if false then 7 else 42 fi else 0 fi; }",
        "pub fnc main(): int { return true ? if false then 7 else 42 fi : 0; }",
        "pub fnc main(): int { return true ? 42 : 7; }",
        "pub fnc main(): int { return false ? { return 7; } : { return 42; }; }",
        "pub fnc main(): int { val int x = true ? { return 40; } : { return 7; }; return x + 2; }",
        "pub fnc main(): int { do if true then return 42; else return 7; fi }",
        "pub fnc main(): int { do if false then return 7; elif true then return 42; else return 0; fi }",
        "pub fnc main(): int { do if false then return 7; fi return 42; }",
        "pub fnc main(): int { if false do return 7; else return 42; fi }",
        "pub fnc main(): int { if true { return 42; } else { return 7; } }",
        "pub fnc main(): int { if false { return 7; } else { return 42; } }",
        "pub fnc main(): int { if false { return 7; } return 42; }",
        "pub fnc main(): int { if false { return 7; } fi return 42; }",
        "pub fnc main(): int { do if true then if false { return 7; } fi return 42; }",
        "pub fnc main(): int { do if false then if true { return 7; } else return 42; fi return 0; }",
        "pub fnc main(): int { if false do if true { return 7; } elif true do return 42; else return 0; fi return 0; }",
        "pub fnc main(): int { do if true then if false { return 7; } elif true { return 42; } fi return 0; }",
        "pub fnc main(): int { val int x = 9223372036854775807; return if false then x + 1 else 42 fi; }",
    ] {
        let wasm = compile(source).unwrap_or_else(|err| panic!("{source}: {err:?}"));
        let mut child = Command::new("node")
            .args(["-e", "const fs=require('node:fs');const assert=require('node:assert/strict');(async()=>{const bytes=fs.readFileSync(0);assert.ok(WebAssembly.validate(bytes));const {instance}=await WebAssembly.instantiate(bytes,{atto:{}});assert.equal(instance.exports.main(),42n);if(instance.exports.panic_code)assert.equal(instance.exports.panic_code.value,0);})().catch(e=>{console.error(e);process.exitCode=1;});"])
            .stdin(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("Node required");
        child.stdin.take().unwrap().write_all(&wasm).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
    }
}

#[test]
fn keyword_conditionals_reject_invalid_inputs() {
    for source in [
        "pub fnc main(): int { return if true then 42 fi; }",
        "pub fnc main(): int { return if 1 then 42 else 7 fi; }",
        "pub fnc main(): int { return if true then 42 else false fi; }",
        "pub fnc main(): int { return if true then 42 else unknown_name fi; }",
        "pub fnc main(): int { do if 1 then return 42; else return 7; fi }",
        "pub fnc main(): int { do if true do return 42; fi }",
        "pub fnc main(): int { do if true then return 42; elif false do return 7; fi }",
        "pub fnc main(): int { if true do return 42; elif false then return 7; fi }",
        "pub fnc main(): int { return if true then 42 else 7; }",
    ] {
        assert!(compile(source).is_err(), "must reject: {source}");
    }
}
