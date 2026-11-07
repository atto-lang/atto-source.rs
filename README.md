# atto-source.rs: AttoLang browser compiler

A Rust compiler from `.atto` source to WebAssembly. It is built for `wasm32-unknown-unknown`, so the [atto-playground](https://github.com/atto-lang/atto-playground) compiles and runs programs entirely in the browser. No server sees user source.

It compiles a **subset** of AttoLang. Everything outside the subset is rejected with a diagnostic, and a rejected program never produces an artifact. The compiler does not approximate unsupported features.

## Where the syntax comes from

The compiler does not define any syntax of its own:

- **Syntax** follows `docs/grammar.ebnf` and `Parser.java` in `oreslang-source.java`. That includes binding forms, the `const mut` / `let mut` referent qualifier, statement terminators with implicit newline termination, condition `,` / `|`, and arity overloading.
- **`int`** is a signed 64-bit integer with checked arithmetic. Overflow is a panic, never a silent wrap. This matches the LLVM backend's trap and spec chapter 17 (TARGET, decided).
- **Spec** decisions come from the AttoLang spec chapters 01, 16 and 17.

## Admitted subset

| Area | Admitted |
| --- | --- |
| Declarations | `[pub] fnc` / `routine` with `: T` or `-> T`, brace body; parameters `int a`, `int mut a`, `a: int`; arity overloads |
| Top-level declarations | `define class Name as ... end` admits **immutable `int`/`bool` literal-default fields** and an explicit `new Name()` zero-argument constructor that allocates a fresh bounded guest-private instance; typed field reads, nominal class parameters/returns. `define enum` admits fieldless nominal variants. One stateless actor `receive(int)->int` handler and typed `spawn Worker()` are admitted only by the actor-aware private browser host; empty actors and struct/module/interface/trait/supervisor bodies remain inert. No language-level `send()->Result`, supervisor or actor heap semantics are claimed. |
| Entry point | zero-argument `main` returning `int` or `void` |
| Types | `int` (i64), `bool` / `boolean`, `void` (return only) |
| Bindings | `val` / `const` / `let`, with `name: T`, `T name` or inferred; `const mut` / `let mut` |
| Statements | assignment, `return`, `if … then … elif/elseif/else if … else … fi` and the brace form closed by `fi`, `while … do … done` / `while … { }`, `loop { }`, C-style `for (…; …; i++)`, `break`, `continue`, `block { }` |
| Expressions | `+ - *`, unary `- + !`, `< <= > >=`, `eq neq == != !eq`, ternary `? :`, `,` (AND) and `\|` (OR) inside conditions, calls, `And` / `Or` / `Xor` |
| Output | `stdio.print(x)` / `stdio.println(x)` for `int`, `bool` and string literals |

The following are rejected, with the code shown:

- Removed `%`, `^` and `^^` give `E_UNSUPPORTED` with no artifact emitted.
- Condition `,` / `|` evaluate **both** boolean operands left-to-right (eager), including observable side effects.
- `int / int` gives `E_UNSUPPORTED`. Its result type is undecided (spec S-25): Java yields `float` and LLVM rejects it.
- `|` outside a condition gives `E_UNSUPPORTED`. Java treats it as bitwise, and the grammar has no bitwise operators.
- Strings as values, floats and sized integers give `E_UNSUPPORTED_TYPE`.
- Bare `Point()` constructor calls give `E_NEW_REQUIRED` (no guest output). Class methods, mutable fields, custom constructors, nonliteral defaults, inheritance and general actor state/methods or struct/module/interface/trait/supervisor members are **not** executable in the browser; only the separately documented stateless actor `receive(int)->int` slice is admitted. Those constructs fail closed. Nominal class values cannot be implicitly converted to scalars and class equality is not yet admitted.
- `define macro` and other unsupported `define` forms, `match` / `switch`, imports, lambdas, `Option` / `Result`, `for … of`, `try` and `defer` give fail-closed diagnostics.

### Known divergences from the Java reference

These are recorded in fixtures and checked by the differential run:

- Binding the result of a void call. Java accepts it. Here it is `E_TYPE`, because a void call has no value.
- Integer overflow. Java wraps silently. Here it panics, following spec 17 and the LLVM backend.


### Top-level `define` staging boundary

This branch implements **nominal fieldless enum values and a constrained nominal class subset**, not the full declaration system. The compiler resolves types and variants (including forward type references), enforces enum identity at assignments, calls, returns, ternaries and equality, then emits actual `i32` guest values in Rust-native and WebAssembly-hosted compilation. Default class construction allocates a fresh, 8-byte-aligned object in a **single fixed 64-KiB guest-private Wasm memory**, with checked bump allocation and panic code 3 on exhaustion. Each immutable `int`/`bool` field occupies an 8-byte slot; field offsets, types and initializer literals are compiler-validated. This is NOT actor heap isolation or a general-purpose allocator. Class methods, writable fields, custom constructors, enum payloads, actor spawning, and other unsupported constructs fail closed with **no** Wasm artifact.

The compiler Wasm shipped at the public `/play/` URL is separately hash-pinned: this source-only PR does not update that artifact or deploy any syntax changes. Promotion requires exact-head Rust+wasm32 guest tests, independently reviewed compiler bytes and provenance receipts, source/playground mirror parity, and the site's protected publication gate.

### Executable nominal enum example

```atto
define enum Status as
  Pending
  Ready
end

fnc ready(Status status): bool { return status eq Status.Ready; }
pub fnc main(): int {
  val current: Status = Status.Ready;
  if ready(current) then return 42; else return 0; fi
}
```

`Status.Ready` is not implicitly converted to `int`, `bool`, or another enum. The in-browser Rust compiler and native CLI share this implementation; the existing **public** playground Wasm artifact is unchanged until the protected release process completes.

### Executable immutable-class example

Class allocation requires the explicit `new` keyword. `Point()` is rejected with `E_NEW_REQUIRED`; only `new Point()` allocates. Ordinary `fnc` calls remain `f()`, without `new`.

```atto
define class Point as
  int x: 40
  int y: 2
  bool valid: true
end

fnc total(Point p): int { return p.x + p.y; }
pub fnc main(): int {
  val point = new Point();  // explicit fresh guest-heap allocation
  if point.valid then return total(point); else return 0; fi
}
```

The bounded class allocator uses the same guest `memory` export as literal output. It introduces **no new imports**, but adds a reserved panic code `3` for class heap exhaustion; consumers must display the updated diagnostic when promoting compiler artifacts. This narrow class implementation is a compiler-backed feature slice, not proof of canonical class lifetime/reclamation or actor-heap parity.

## Guest contract `atto.browser.guest.v1`

- **Exports:** `main: () -> i64` (a `void` main returns 0) and `panic_code: mut i32` (1 = checked integer overflow; code 2 reserved/unreachable; 3 = fixed class heap exhausted). `memory` is exported when string literals are printed **or class instances are allocated**. Stage-1 actor handlers are additionally exported as `atto_actor_receive_<nominal-id>(i64)->i64`, for a separate browser worker guest instance.
- **Imports:** only those used, from module `atto`: `print_i64(i64)`, `print_bool(i32)`, `print_str(i32, i32)`, `print_newline()`, and for a checked executable actor spawn only, `actor_spawn(i32 actor_type)->i32 actor_handle`. The `actor_spawn` capability is implemented **only in the private browser test host**, not in the published playground. No other host capabilities are admitted.
- **Determinism:** output is a pure function of the source. The native and wasm32 hosts emit byte-identical modules.

## Compiler ABI v2 (wasm32 build)

The compiler has no imports and memory capped at 64 MiB. It exports:

- `abi_version()`, which returns 2
- `memory`, `input_ptr()` and `input_capacity()` (64 KiB)
- `compile(len)`, which returns 0 on success and 1 on a diagnostic
- `result_ptr()` and `result_length()`, which are 0 on error
- `error_ptr()`, a NUL-terminated `E_CODE: message`
- `error_line_number()` and `error_column_number()`

## Resource limits

Every limit fails closed with a diagnostic; none of them can trap the compiler.

| Limit | Value | Code |
|---|---|---|
| Source size | 64 KiB of UTF-8 | `E_SOURCE_LIMIT` |
| Tokens | 32,768 | `E_SOURCE_LIMIT` |
| Recursive nesting (parens, blocks, unary operators, ternaries, calls) | 128 levels | `E_NESTING` |
| Syntax-tree height, counting each link of an operator chain and each `elif` arm | 256 | `E_NESTING` |
| Functions / parameters / locals per function | 1,024 / 32 / 1,024 | `E_SOURCE_LIMIT` |
| One string literal / all string data | 4 KiB / 32 KiB | `E_SOURCE_LIMIT` |
| Generated module | 1 MiB | `E_OUTPUT_LIMIT` |

The tree-height budget exists because `a + b + c + ...` is parsed in a loop
but produces a left-leaning tree that the checker and emitter walk
recursively; without it a 30 KB source exhausted the stack. At 256 levels the
worst case needs about 1.7 MiB of stack in a debug build and about 220 KiB in
a release build (measured with `examples/stackprobe.rs`), so `compile()` is
safe on a default 2 MiB thread.

## Build and test

```sh
rustup toolchain install 1.85.1 --profile minimal --target wasm32-unknown-unknown
cargo test --locked
cargo build --locked --release --bin attoc
cargo rustc --locked --release --lib --target wasm32-unknown-unknown --crate-type cdylib -- -C link-arg=--max-memory=67108864
ATTOC=target/release/attoc node tests/conformance.mjs
# Optional Java reference differential (local only; the reference is private):
ATTO_JAVA=/path/to/oreslang-java-launcher ATTOC=target/release/attoc node tests/conformance.mjs
```

The fixture corpus is in `tests/fixtures/`:

- `pass/`: expected `main()` result and stdout
- `fail/`: expected diagnostic code, plus a `// java: rejects|accepts` claim
- `panic/`: expected panic code

`tests/conformance.mjs` checks the following:

- It compiles every fixture with the wasm32 compiler and executes every guest.
- It compiles 200 generated programs.
- It feeds the compiler 2,000 random inputs.
- It checks the nesting, tree-height and source limits, at and just over each budget.
- It feeds the compiler adversarial shapes (15,000-link chains, 30,000 unary operators, unclosed nesting) and requires a diagnostic, never a trap.
- It mutates every positive fixture at the token level (60 mutants each; set `ATTO_FUZZ=<n>` for more) and requires every accepted mutant to be valid Wasm and every rejection to carry a code and position.
- It checks recovery after an error.
- With `ATTOC` set, it requires byte-identical native output.
- With `ATTO_JAVA` set, it requires the Java reference to agree on exit status, stdout and acceptance for each fixture.


## Private browser actor stage-1 proposal (PR #18)

A **restricted stateless actor** may be declared with a single
`receive(msg: int) -> int { ... }` body (optional `fnc` prefix), then
spawned as `val actor: Worker = spawn Worker();`. Its result is a nominal
actor handle, never an integer, pointer or class reference. A standalone
browser host must supply `atto.actor_spawn(i32)->i32` and implement an
actor worker for that declared handler. The guest exports
`atto_actor_receive_<nominal-id>(i64)->i64` for the actor worker to
invoke with its own guest Wasm instance, per-worker memory and JS
execution stack. The real-browser test harness uses explicit host
message injection to deliver canonical signed i64 values, as
language-level `actor.send(msg) -> Result` is **not admitted yet**.

For example, source compilation is allowed (with the actor-aware host):

```atto
define actor Worker as
  receive(msg: int) -> int { return msg + 1; }
end
pub fnc main(): int {
  val worker: Worker = spawn Worker();
  return 0;
}
```

This is **not** a complete actor runtime. The following remain fail-closed:
actor state, multiple handler arms, constructors, actor-side stdio or
nested spawning, member calls such as `actor.send(...)`, and
`rt take`/borrowing/ownership transfers. The browser host rejects
objects, class pointers, noncanonical or out-of-range messages.
The stage-1 harness bounds actors per guest run (four) and its ready
and request handshakes (two seconds each), serializes replies and
terminates workers. It does **not** implement canonical language
`Result`/completion, supervisor CONTROL, actor arena lifetime,
independent guaranteed guest stacks, or cross-tenant enforced security.

The test host's actor import is **not** part of the current published
playground's grant list. No release of actor-capable compiler bytes or
public site changes is authorized without the source, playground and
Pages release gates.

## Actor mailbox admission: restricted Result<void, SendError> (private only)

On the actor-runtime development branch, `worker.send(int)` returns a
**typed admission result**, currently only `Result<void, SendError>`.
`Ok(())` means the bounded host mailbox accepted the message *synchronously*
during guest execution. `Err(Full)` means capacity 2 was reached;
`Err(Closed)` means the mailbox is closed. Neither `Ok` nor `close()`
confirms handler completion. `worker.close()` returns `bool` (true only
on the first close) and stops further sends while preserving admitted FIFO
messages for delivery.

The limited `Result<void, SendError>` supports `is_ok()`, `is_err()`,
`is_full()` and `is_closed()`. It is not the general generic `Result`
runtime and does not yet support `match`, `?`, `map_err`, or transport
of class pointers, handles or borrowed values. The current browser actor
test host runs admitted queues after the synchronous guest `main()` returns,
in separate worker/guest instances; it does not establish canonical
production scheduler fairness or `get_done()` semantics.

The new guest imports `atto.actor_send(i32,i64)->i32` (status 0=Ok,
1=Full, 2=Closed) and `atto.actor_close(i32)->i32` (bool), both prohibited
inside `receive` until capability and supervision checks exist. These
imports are *not* granted by the public playground or previously shipped
compiler. Compilation rejects all non-int send payloads and unsupported
generic Result types with no guest code.

## Private browser-worker lifecycle proof (host only)

The actor lifecycle test host, not the language compiler, owns an explicit
per-actor state machine: `STARTING -> READY -> DRAINING -> FINISHED/CLOSED`,
with exceptional terminal states `FAILED` and `TERMINATED`. The actor
must acknowledge a `shutdown` request after processing all prior work
before the host records `FINISHED` or `CLOSED`. A forced worker termination,
unexpected failure, timeout, or guest-main panic is never mislabelled as
a normal completion.

The private harness returns `actorCompletions` (one terminal record per
actor, `handle`, `actorId`, `state`, `reason`, `admitted`,
`delivered`, `externalDelivered`, `dropped`), using an idempotent
terminal transition. This is a **host evidence envelope**, not an AttoLang
`get_done()` future, promise, acknowledgement of each message, or
canonical supervisor priority CONTROL channel. No guest `await` is
introduced. The existing finite guest-run scope implicitly shuts down
open actors after draining; explicit `close()` prevents further sends,
preserves already admitted messages, and yields `CLOSED` only after the
child's actual shutdown acknowledgement.

Pure lifecycle state-machine conformance and real Chromium child-worker
tests cover normal drain, idle actor termination, explicit close, worker
panic, host-forced termination, guest-main panic, initialization failure, failed shutdown ACK, invalid
transfers, resource cleanup and exactly-once terminal evidence. **No claim is made
for a real concurrent-producer scheduler, OS-enforced tenant isolation,
actor arena reclamation, separate guaranteed guest stacks, backend parity,
or language-level completion/supervision yet.**
