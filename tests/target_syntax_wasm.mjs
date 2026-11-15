// Verify the actual shipped Rust-to-Wasm compiler ABI fails closed for
// syntax-only namespace, declared-actor and out-of-line receiver headers.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';

const file = process.argv[2] || 'target/wasm32-unknown-unknown/release/atto_compiler.wasm';
const compiler = new WebAssembly.Module(await readFile(file));
assert.deepEqual(WebAssembly.Module.imports(compiler), []);
const {exports: api} = new WebAssembly.Instance(compiler, {});
assert.equal(api.abi_version(), 2);
const encoder = new TextEncoder();
const decoder = new TextDecoder('utf-8', {fatal:true});

function compile(source) {
  const bytes = encoder.encode(source);
  assert.ok(bytes.length <= api.input_capacity());
  new Uint8Array(api.memory.buffer,api.input_ptr(),bytes.length).set(bytes);
  const rc = api.compile(bytes.length);
  if (rc === 0) {
    const output = new Uint8Array(api.memory.buffer,api.result_ptr(),api.result_length()).slice();
    assert.deepEqual([...output.subarray(0,8)],[0,97,115,109,1,0,0,0]);
    return {ok:true,bytes:output};
  }
  assert.equal(rc,1);
  assert.equal(api.result_length(),0,'never emit partial code on an error');
  const mem=new Uint8Array(api.memory.buffer);
  let end=api.error_ptr();
  while (mem[end]) end++;
  const text=decoder.decode(mem.subarray(api.error_ptr(),end));
  return {ok:false,code:text.split(':')[0],text,line:api.error_line_number(),col:api.error_column_number()};
}
function rejects(source,code) {
  const out=compile(source);
  assert.equal(out.ok,false,source);
  assert.equal(out.code,code,source+' -> '+out.text);
  assert.ok(out.line>=1 && out.col>=1,source);
}
rejects(`use namespace workers;
declare actor Foo1;
declare actor Foo2;
mtd (Foo1 self) foo(): void {}
mtd (Foo2 self) foo(): void {}
pub fnc main(): int { return 0; }`,'E_UNSUPPORTED');
rejects(`pub fnc helper(): int { return 1; }
declare actor Foo;
mtd (mut self: Foo) set(int n): void {}`,'E_UNSUPPORTED');
rejects('use namespace workers; pub fnc main(): int {return 1;}','E_UNSUPPORTED');
rejects('declare actor Foo; declare actor Foo;','E_DUPLICATE');
rejects('declare actor Foo; mtd (Other self) foo(): void {}','E_UNBOUND');
rejects('declare actor Foo; mtd (Foo self) foo(): void {} mtd (self: Foo) foo(): void {}','E_DUPLICATE');
rejects('declare actor Foo; mtd (Foo self) foo(int): void {}','E_SYNTAX');
rejects('declare actor Foo; use namespace workers;','E_SYNTAX');
assert.equal(compile('pub fnc main(): int {return 42;}').ok,true);
assert.equal(compile('pub fnc main(): int {stdio.println("declare actor Foo; mtd (Foo self) foo()"); return 42;}').ok,true);

rejects('declare actor Foo; mtd (Foo self) foo(): int { return self.count; }','E_SEALED');
rejects('declare actor Foo; mtd (self: Foo) foo(): void { self.handle_state(critical-write |s| -> {s.count = 1;}); }','E_MUTABILITY');
rejects('declare actor Foo; mtd (mut self: Foo) foo(): void { self.handle_state(critical-read |s| -> {s.count = 1;}); }','E_READONLY');
rejects('declare actor Foo; mtd (mut self: Foo) foo(): void { self.handle_state(critical-read |s| -> {return s;}); }','E_ESCAPE');
rejects('declare actor Foo; mtd (mut self: Foo) foo(): void { self.handle_state(critical-read |s| -> await something()); }','E_BORROW');
rejects('declare actor Foo; mtd (Foo self) foo(): void {} pub fnc main(): int {val a = spawn Foo(); a.foo();return 0;}','E_ISOLATION');
rejects('declare actor Foo; mtd (mut self: Foo) foo(): void {self.handle_state(critical-write |s| -> {s.count = 1;});}','E_UNSUPPORTED');
rejects('declare actor Worker; mtd (Worker self) bad(Mystery value): void {}','E_UNKNOWN_TYPE');
rejects('declare actor Worker; mtd (Worker self) bad(Worker value): void {}','E_TYPE');
rejects('declare actor Worker; mtd (Worker self) bad(ActorRef<Missing> value): void {}','E_UNBOUND');
rejects('declare actor Worker; mtd (Worker self) bad(void value): void {}','E_TYPE');
rejects('declare actor Worker; mtd (Worker self) bad(int a,bool a): void {}','E_DUPLICATE');
rejects('declare actor Worker; mtd (Worker self) bad(): Mystery {}','E_UNKNOWN_TYPE');
rejects('declare actor Worker; mtd (Worker self) good(ActorRef<Worker> value): void {}','E_UNSUPPORTED');
rejects(`use namespace workers;
declare actor Foo;
define actor Foo as receive(ActorMail<int> mail): void { return; } end
mtd (Foo self) ping(): int { return 1; }`,'E_UNSUPPORTED');
rejects('use namespace workers; declare actor Foo; define actor Bar as end','E_UNBOUND');
rejects('use namespace workers; declare actor Foo; define actor Foo as end define actor Foo as end','E_DUPLICATE');
rejects('use namespace workers; declare actor Foo; define actor Foo as foo(): int {return 1;} end mtd (Foo self) foo(): int {return 2;}','E_DUPLICATE');
rejects('use namespace workers; declare actor Foo; define actor Foo as foo(): int {return 1;} end pub mtd (Foo self) foo(): bool {return true;}','E_SIGNATURE');

// Harden the executable browser compiler ABI preflight: malformed nested calls,
// receiver escapes and readonly mutations fail closed with no guest artifact.
rejects('declare actor Foo; mtd (Foo self) foo(int a,int b): void {} mtd (Foo self) run(): void { self.foo(self.secret,1); }','E_SEALED');
rejects('declare actor Foo; mtd (Foo self) foo(int a,int b): void {} mtd (Foo self) run(): void { self.foo(self,1); }','E_ESCAPE');
rejects('declare actor Foo; mtd (Foo self) foo(int a,int b): void {} mtd (Foo self) run(): void { self.foo(1,]); }','E_SYNTAX');
rejects('declare actor Foo; mtd (Foo self) foo(int a,int b): void {} mtd (Foo self) run(): void { self.foo(self.handle_state(critical-write |s| -> { s.n = 1; }),1); }','E_MUTABILITY');
rejects('declare actor Foo; mtd (mut self: Foo) run(): void { self.handle_state(critical-read |s| -> { ++s.n; }); }','E_READONLY');
rejects('declare actor Foo; mtd (Foo self) run(): void { self.handle_state(critical-read |s| -> { sink(self); }); }','E_BORROW');


// Concrete state layouts must be checked in the same Wasm compiler ABI.
const concreteState = 'declare actor Counter; define actor Counter as let int count = 0; val int limit = 12; let string label = "hello"; end';
rejects(concreteState+' mtd (Counter self) run(): int { return self.handle_state(critical-read |s| -> s.missing); }','E_UNBOUND');
rejects(concreteState+' mtd (mut self: Counter) run(): void { self.handle_state(critical-write |s| -> { s.limit = 2; }); }','E_READONLY');
rejects(concreteState+' mtd (mut self: Counter) run(): int { return self.handle_state(critical-read |s| -> s.label); }','E_BORROW');
rejects(concreteState+' mtd (Counter self) run(): int { return self.handle_state(critical-read |s| -> s.count); }','E_UNSUPPORTED');


// An untyped/unknown-effect call must not execute while an actor state lease
// is held; malformed nesting and reserved lifecycle methods fail closed too.
rejects('declare actor Foo; mtd (Foo self) receive(): void {}','E_SIGNATURE');
rejects('declare actor Foo; mtd (mut self: Foo) run(): void { self.handle_state(critical-write |s| -> { panic(Error(1)); }); }','E_BORROW');
rejects('declare actor Foo; mtd (mut self: Foo) run(): void { self.handle_state(critical-write |s| -> { s.count = risky(); }); }','E_BORROW');
rejects('declare actor Foo; mtd (mut self: Foo) run(): void { self.handle_state(critical-read |s| -> s.count?); }','E_BORROW');
rejects('declare actor Foo; mtd (mut self: Foo) run(): void { self.handle_state(critical-write |s| -> { s.count[0] = 1; }); }','E_UNSUPPORTED');
rejects('declare actor Foo; mtd (mut self: Foo) run(): void { self.handle_state(critical-read |s| -> { s.count + [1, 2); }); }','E_SYNTAX');
rejects('declare actor Foo; mtd (mut self: Foo) run(): void { self.handle_state(critical-write |s| -> { yield* stream; }); }','E_BORROW');


// Typed scalar actor-state assignment proofs run in the real Wasm compiler ABI.
const typedCounter = 'declare actor Counter; define actor Counter as let int count = 0; let bool enabled = false; end';
rejects(typedCounter+' mtd (mut self: Counter) tick(): void { self.handle_state(critical-write |s| -> { s.count = true; }); }','E_TYPE');
rejects(typedCounter+' mtd (mut self: Counter) tick(): void { self.handle_state(critical-write |s| -> { s.count = s.enabled; }); }','E_TYPE');
rejects(typedCounter+' mtd (mut self: Counter) tick(): void { self.handle_state(critical-write |s| -> { s.count = nowhere; }); }','E_UNBOUND');
rejects(typedCounter+' mtd (mut self: Counter) tick(): void { self.handle_state(critical-write |s| -> { ++s.enabled; }); }','E_TYPE');
rejects(typedCounter+' mtd (mut self: Counter) tick(): void { self.handle_state(critical-write |s| -> { s.count = new Widget(); }); }','E_BORROW');


// Boundary and effect regressions must be observable from the actual browser
// Wasm compiler ABI, with zero guest bytes published on rejection.
rejects(typedCounter+' mtd (mut self: Counter) tick(): void { self.handle_state(critical-write |s| -> { s.count = 9223372036854775808; }); }','E_INTEGER_RANGE');
rejects(typedCounter+' mtd (mut self: Counter) tick(): void { self.handle_state(critical-write |s| -> { s.enabled = true < false; }); }','E_TYPE');
rejects(typedCounter+' mtd (mut self: Counter) tick(): void { self.handle_state(critical-read |s| -> { val leaked = &s.count; }); }','E_BORROW');
rejects(typedCounter+' mtd (mut self: Counter) tick(): void { self.handle_state(critical-write |s| -> { val leaked = &s.count; }); }','E_BORROW');
rejects(typedCounter+' mtd (mut self: Counter) tick(): void { self.handle_state(critical-write |s| -> { s.count = -9223372036854775808; }); }','E_UNSUPPORTED');


// New typed receiver return facts must reject nominal/signature mismatches
// in the *actual* browser compiler ABI before publishing any guest Wasm.
rejects('declare actor A; mtd (A self) wrong(): int { return true; }','E_TYPE');
rejects('declare actor A; mtd (A self) wrong(): bool { return 3; }','E_TYPE');
rejects('declare actor A; mtd (A self) wrong(): void { return 3; }','E_TYPE');
rejects('declare actor A; mtd (A self) wrong(bool flag): int { return flag; }','E_TYPE');
rejects('declare actor A; mtd (A self) good(): bool { return true; } mtd (A self) wrong(): int {return self.good();}','E_TYPE');
rejects('declare actor A; mtd (A self) bad(): int { return 9223372036854775808; }','E_INTEGER_RANGE');
rejects('declare actor A; mtd (A self) valid(): int {return 10;}','E_UNSUPPORTED');


// Conservative structured CFG and nominal receiver call argument checks.
rejects('declare actor A; mtd (A self) choose(bool flag): int { if flag {return 1;} else {} }','E_MISSING_RETURN');
rejects('declare actor A; mtd (A self) choose(bool flag): int { if flag {return 1;} }','E_MISSING_RETURN');
rejects('declare actor A; mtd (A self) choose(int count): int { if count {return 1;} else {return 2;} }','E_TYPE');
rejects('declare actor A; mtd (A self) consume(int n): void {} mtd (A self) run(): void {self.consume(true);}','E_TYPE');
rejects('declare actor A; mtd (A self) consume(bool flag): void {} mtd (A self) run(int n): void {self.consume(n);}','E_TYPE');
rejects('declare actor A; declare actor B; mtd (A self) consume(ActorRef<A> a): void {} mtd (A self) run(ActorRef<B> b): void {self.consume(b);}','E_TYPE');
rejects('declare actor A; mtd (A self) choose(bool flag): int { if flag {return 1;} else {return 2;} }','E_UNSUPPORTED');
rejects('declare actor A; mtd (A self) consume(int n): void {} mtd (A self) run(int n): void {self.consume(n);}','E_UNSUPPORTED');


// Path-sensitive ownership preflight executes inside the actual Wasm compiler.
// Rejection must leave no emitted guest bytes (enforced by rejects()).
const movePrelude = 'declare actor M; mtd (M self) consume(ActorRef<M> peer): void {} mtd (M self) run(ActorRef<M> peer): void {';
rejects(movePrelude+' rt take peer; rt take peer; }','E_MOVE');
rejects(movePrelude+' rt take peer; self.consume(peer); }','E_MOVE');
rejects(movePrelude+' val a = rt borrow peer; rt take peer; }','E_BORROW');
rejects(movePrelude+' val a = rt borrow peer; await pending(); }','E_BORROW');
rejects(movePrelude+' val a = rt borrow peer; self.consume(a); }','E_ESCAPE');
rejects(movePrelude+' val a = rt borrow peer; return a; }','E_ESCAPE');
rejects(movePrelude+' let value = 1; val a = rt borrow mut value; val b = rt borrow value; }','E_BORROW');
rejects(movePrelude+' if true { rt take peer; } else {} self.consume(peer); }','E_MOVE');
rejects(movePrelude+' { val a = rt borrow peer; } rt take peer; }','E_UNSUPPORTED');


// Ownership preflight soundness regressions: shared-loan mutation and
// effectful conditions must not emit runnable actor code.
rejects(movePrelude+' val a = rt borrow peer; peer = spawn M(); }','E_BORROW');
rejects(movePrelude+' val a = rt borrow peer; peer += 1; }','E_BORROW');
rejects(movePrelude+' val a = rt borrow peer; peer++; }','E_BORROW');
rejects(movePrelude+' val a = rt borrow peer; ++peer; }','E_BORROW');
rejects(movePrelude+' if rt take peer { } else { } }','E_BORROW');
rejects(movePrelude+' if rt borrow peer { } else { } }','E_BORROW');
rejects(movePrelude+' if rt borrow mut peer { } else { } }','E_BORROW');
rejects(movePrelude+' if compute(peer) { } else { } }','E_UNSUPPORTED');
rejects(movePrelude+' { val a = rt borrow peer; val b = rt borrow peer; } rt take peer; }','E_UNSUPPORTED');

console.log('PASS exact browser Wasm ABI actor linking and fail-closed checks (89)');

