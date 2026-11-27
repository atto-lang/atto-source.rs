// Exact source from #331 093aedf606acdcae6f4d7e5f7d5ccdf8578732d9 and shared conformance 2be1230ee6bad2017fc0dfab8777c01643a265f1.
// Run explicitly; this is a parity candidate, not an assertion of existing support.
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {mkdtemp,writeFile,readFile,rm} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
const attoc=process.env.ATTOC || fileURLToPath(new URL('../target/debug/attoc',import.meta.url));
const cases=[
  {
    "name": "checked-unwrap-option-none",
    "source": "// expect-exit: 42\n// expect-stdout: called Option::unwrap() on a None value\nfnc missing(): int throws(Error) {\n  val Option<int> x = None;\n  return x.unwrap();\n}\npub fnc main(): int {\n  val Result<int,Error> r = trap! missing();\n  match r over\n    on Ok(v) -> { return 1; }\n    on Err(e) -> { stdio.println(e.message); return 42; }\n  end\n}\n",
    "blob": "296db8692562dc5d03abb4fc2ece7ece7199b0f7",
    "stdout": "called Option::unwrap() on a None value\n"
  },
  {
    "name": "checked-unwrap-option-some",
    "source": "// expect-exit: 42\npub fnc main(): int {\n  val Option<int> x = Some(42);\n  val Result<int,Error> r = trap! x.unwrap();\n  match r over\n    on Ok(v) -> { return v; }\n    on Err(e) -> { return 1; }\n  end\n}\n",
    "blob": "1941b78ae55a48ced77ff7c7b1b4ca68b63c25f1",
    "stdout": ""
  },
  {
    "name": "checked-unwrap-result-error-cleanup",
    "source": "// expect-exit: 42\n// expect-stdout: cleanup\n// expect-stdout: called Result::unwrap() on an Err value\nfnc rejected(Error original): int throws(Error) {\n  defer || -> { stdio.println(\"cleanup\"); };\n  val Result<int,Error> r = Err(original);\n  return r.unwrap();\n}\npub fnc main(): int {\n  val Error original = Error(\"original\");\n  val Result<int,Error> r = trap! rejected(original);\n  match r over\n    on Ok(v) -> { return 1; }\n    on Err(e) -> {\n      if e.is_same_instance(original) then return 2; fi\n      stdio.println(e.message);\n      return 42;\n    }\n  end\n}\n",
    "blob": "8e261174e4dfec3c35db5bc30d37ffcec661f71e",
    "stdout": "cleanup\ncalled Result::unwrap() on an Err value\n"
  },
  {
    "name": "checked-unwrap-result-ok",
    "source": "// expect-exit: 42\nfnc accepted(): Result<int,Error> { return Ok(42); }\npub fnc main(): int {\n  val Result<int,Error> r = trap! accepted().unwrap();\n  match r over\n    on Ok(v) -> { return v; }\n    on Err(e) -> { return 1; }\n  end\n}\n",
    "blob": "9556e38f355bccd5a5b4c9305f2b706d1559113f",
    "stdout": ""
  },
  {
    "name": "checked-unwrap-result-text",
    "source": "// expect-exit: 42\n// expect-stdout: called Result::unwrap() on an Err value\nfnc rejected(): Result<int,String> { return Err(\"original\"); }\npub fnc main(): int {\n  val Result<int,Error> r = trap! rejected().unwrap();\n  match r over\n    on Ok(v) -> { return 1; }\n    on Err(e) -> { stdio.println(e.message); return 42; }\n  end\n}\n",
    "blob": "046095da5962f024fe0b9cd7d09710e26e5e394c",
    "stdout": "called Result::unwrap() on an Err value\n"
  },
  {
    "name": "v6-checked-unwrap",
    "source": "fnc optional(bool present): Option<int> {\n  if present do return Some(42); fi\n  return None;\n}\nfnc outcome(bool present): Result<int, String> {\n  if present do return Ok(42); fi\n  return Err(\"original\");\n}\npub fnc main(): int {\n  val Result<int, Error> a = trap! optional(false).unwrap();\n  val Result<int, Error> b = trap! outcome(false).unwrap();\n  val Result<int, Error> c = trap! optional(true).unwrap();\n  val Result<int, Error> d = trap outcome(true).unwrap();\n  val Result<Option<int>, Error> e = trap! optional(false);\n  val Result<int, Error> f = trap! (trap! optional(false).unwrap());\n  guard a.is_err(), b.is_err(), c.is_ok(), d.is_ok(), e.is_ok(), f.is_err() { return 1; }\n  guard c.unwrap() eq 42, d.unwrap() eq 42, e.unwrap().is_none() { return 2; }\n  return 42;\n}\n",
    "blob": "90c6166041963d1f466bc51f105c66a0a7be15ac",
    "stdout": "",
    "panic": false
  },
  {
    "name": "v6-inline-task-failure",
    "source": "pub fnc main(): int {\n  let int count = 0;\n  val Option<int> missing = None;\n  val Result<int, Error> failure = trap! sync task<int> {\n    count = count + 2;\n    defer stdio.stdout.write(\"D\");\n    val int value = missing.unwrap();\n    count = count + 100;\n    return value;\n  };\n  guard failure.is_err(), count eq 2 { return 1; }\n  return count + 40;\n}\n",
    "blob": "446624a894788834550ac7a13a7a76ea5d06444c",
    "stdout": "D",
    "panic": false
  },
  {
    "name": "v6-short-circuit",
    "source": "fnc rhs(): bool {\n  stdio.stdout.write(\"R\");\n  val Option<bool> missing = None;\n  return missing.unwrap();\n}\nfnc conjunction(bool run): bool { return run && rhs(); }\nfnc disjunction(bool skip): bool { return skip || rhs(); }\npub fnc main(): int {\n  val Result<bool, Error> a = trap! conjunction(false);\n  val Result<bool, Error> b = trap! disjunction(true);\n  val Result<bool, Error> c = trap! conjunction(true);\n  val Result<bool, Error> d = trap! disjunction(false);\n  guard a.is_ok(), b.is_ok(), c.is_err(), d.is_err() { return 1; }\n  guard !a.unwrap(), b.unwrap() { return 2; }\n  return 42;\n}\n",
    "blob": "d24442cf7daf1dd86876b418f0cb9e793cbb23c2",
    "stdout": "RR",
    "panic": false
  },
  {
    "name": "v6-terminal-expect",
    "source": "pub fnc main(): int {\n  val Option<int> missing = None;\n  val Result<int, Error> ignored = trap! missing.expect(\"explicit panic control\");\n  return 101;\n}\n",
    "blob": "864e6b34fe46de9c260a7e537f3f008caa7c8b79",
    "stdout": "",
    "panic": true
  },
  {
    "name": "v6-piped-match-none",
    "source": "declare type Bounded = int where self >= 1 and self <= 42;\n\npub fnc main() -> int {\n    val Option<int> x = None;\n\n    return trap! x.unwrap() |> match<int>  over\n        on Ok(n): int -> n,\n        on Err(): int -> 42,\n    end;\n}\n",
    "blob": "fb1653c04f6556e130ec0dc51f8a0fd933e34580",
    "stdout": "",
    "panic": false
  },
  {
    "name": "v6-piped-match-some",
    "source": "declare type Bounded = int where self >= 1 and self <= 42;\n\npub fnc main() -> int {\n    val Option<int> x = Some(42);\n\n    return trap! x.unwrap() |> match<int>  over\n        on Ok(n): int -> n,\n        on Err(): int -> 7,\n    end;\n}\n",
    "blob": "77b3f981f031aaaef072f1c68b66960825cf232c",
    "stdout": "",
    "panic": false
  },
  {
    "name": "v6-piped-match-dynamic",
    "source": "declare type Bounded = int where self >= 1 and self <= 42;\n\nfnc get_value(bool present): Option<int> {\n    return present ? Some(7) : None;\n}\n\nfnc choose(bool present): int {\n    val Option<int> x = get_value(present);\n    return trap! x.unwrap() |> match<int> over\n        on Ok(n): int -> n,\n        on Err(): int -> 42,\n    end;\n}\n\npub fnc main(): int {\n    guard choose(true) eq 7 { return 1; }\n    guard choose(false) eq 42 { return 2; }\n    return 42;\n}\n",
    "blob": "99cf8ea1bf165b622c65400244a684ae0153fcb9",
    "stdout": "",
    "panic": false
  },
  {
    "name": "v6-piped-match-arm-throw",
    "source": "fnc downstream_failure(): int throws(Error) {\n    throw! new Error(\"downstream\");\n}\n\nfnc choose(Option<int> x): int throws(Error) {\n    return trap! x.unwrap() |> match<int> over\n        on Ok(n): int -> n,\n        on Err(): int -> downstream_failure(),\n    end;\n}\n\npub fnc main(): int {\n    guard choose(Some(7)) eq 7 { return 1; }\n    val caught = trap! choose(None);\n    guard caught.is_err() { return 2; }\n    return 42;\n}\n",
    "blob": "411801dc7eb4f6e3065911012ba21f3ffc8227c4",
    "stdout": "",
    "panic": false
  },
  {
    "name": "v6-piped-match-error-binding",
    "source": "fnc handle_error(Error e): int throws() {\n    return 42;\n}\n\nfnc choose(Option<int> x): int throws() {\n    return trap! x.unwrap() |> match<int> over\n        on Ok(n): int -> n,\n        on Err(e): int -> handle_error(e),\n    end;\n}\n\npub fnc main(): int throws() {\n    guard choose(Some(7)) eq 7 { return 1; }\n    guard choose(None) eq 42 { return 2; }\n    return 42;\n}\n",
    "blob": "13ee1d7cd4b83e062c4f8b454d2b5b213cc24a8f",
    "stdout": "",
    "panic": false
  }
];
// Exact new corpus from conformance #703; historic v6 pins untouched.
cases.push(...[
  {
    "name": "canonical-stdout-v1-stdio-direct.atto",
    "source": "pub fnc main(): int {\n  stdio.out.write(\"A\");\n  stdio.out.write(\"B\");\n  return 42;\n}\n",
    "blob": "fcb8c1b56322b1b068007513cd60791504de5918",
    "stdout": "AB",
    "panic": false
  },
  {
    "name": "canonical-stdout-v1-process-direct.atto",
    "source": "pub fnc main(): int {\n  process.stdout.write(\"A\");\n  process.stdout.write(\"B\");\n  return 42;\n}\n",
    "blob": "0d4a07d1c1ccf6c9f1083816a3740091a8467641",
    "stdout": "AB",
    "panic": false
  },
  {
    "name": "canonical-stdout-v1-mixed-defer.atto",
    "source": "pub fnc main(): int {\n  defer stdio.out.write(\"A\");\n  defer process.stdout.write(\"B\");\n  stdio.out.write(\"C\");\n  return 42;\n}\n",
    "blob": "7d1868164d3263bb8158c590a7ad0c36e069bb56",
    "stdout": "CBA",
    "panic": false
  },
  {
    "name": "canonical-stdout-v1-mixed-trapped-task.atto",
    "source": "pub fnc main(): int {\n  let int count = 0;\n  val Option<int> missing = None;\n  val Result<int, Error> failure = trap! sync task<int> {\n    count = count + 2;\n    defer stdio.out.write(\"D\");\n    defer process.stdout.write(\"E\");\n    val int value = missing.unwrap();\n    count = count + 100;\n    return value;\n  };\n  guard failure.is_err(), count eq 2 { return 1; }\n  return count + 40;\n}\n",
    "blob": "5b122c0396240cee7d0be125f952655560b1e78d",
    "stdout": "ED",
    "panic": false
  }
]);
function runGuest(bytes) {
  assert.ok(WebAssembly.validate(bytes), 'invalid guest');
  const module = new WebAssembly.Module(bytes);
  for (const imp of WebAssembly.Module.imports(module)) {
    assert.equal(imp.module, 'atto');
    assert.ok(['print_i64', 'print_bool', 'print_str', 'print_newline', 'print_f64'].includes(imp.name),
      'unexpected host import ' + imp.name);
  }
  let memory;
  let stdout = '';
  const dec = new TextDecoder();
  const instance = new WebAssembly.Instance(module, { atto: {
    print_i64: v => { stdout += String(v); },
    print_bool: v => { stdout += v ? 'true' : 'false'; },
    print_f64: v => { stdout += String(v); },
    print_str: (p, n) => { stdout += dec.decode(new Uint8Array(memory.buffer, p, n)); },
    print_newline: () => { stdout += '\n'; },
  } });
  memory = instance.exports.memory;
  try {
    return { value: instance.exports.main(), stdout, panic: instance.exports.panic_code.value };
  } catch (error) {
    return { error, stdout, panic: instance.exports.panic_code.value };
  }
}


const work=await mkdtemp(join(tmpdir(),'atto-unwrap-v6-'));
let failures=0;
try {
 for(const [i,c] of cases.entries()){
  try {
   const b=Buffer.from(c.source);
   assert.equal(createHash('sha1').update('blob '+b.length+'\0').update(b).digest('hex'),c.blob);
   const src=join(work,i+'.atto'), out=join(work,i+'.wasm');
   await writeFile(src,b);
   const built=spawnSync(attoc,[src,'-o',out],{encoding:'utf8',timeout:30000});
   if(built.status!==0){
    await assert.rejects(readFile(out),{code:'ENOENT'});
    throw Error('compiler did not admit guest: '+(built.stderr||built.error||built.signal));
   }
   const r=runGuest(await readFile(out));
   assert.equal(r.stdout,c.stdout);
   if(c.panic){
    assert.ok(r.error instanceof WebAssembly.RuntimeError,'expected terminal guest panic');
    assert.ok(Number(r.panic)>0,'terminal panic must be classified by guest');
   }else{
    assert.ok(!r.error,String(r.error));
    assert.equal(r.value,42n);
    assert.equal(r.panic,0);
   }
   console.log('GUEST_PASS|'+c.name);
  }catch(e){failures++;console.error('GUEST_FAIL|'+c.name+'|'+e.message);}
 }
 console.log('GUEST_TOTAL|'+cases.length+'|'+(cases.length-failures)+'|'+failures);
}finally{await rm(work,{recursive:true,force:true});}
assert.equal(failures,0,'parity gaps remain');
