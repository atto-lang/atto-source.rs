# L3 borrow-conformance export (`borrow_export`, structured-cfg-v1)

This compiler keeps its own in-pass borrow checker (`src/check.rs`). The opt-in
exporter in `src/borrow_export.rs` records the ownership decisions that checker
makes, using the canonical event vocabulary of
[atto-borrow-checker.rs](https://github.com/atto-lang/atto-borrow-checker.rs).
The master checker then re-verifies each function. This is level L3 of its
conformance plan (`docs/CONFORMANCE_PLAN.md` in that repo).

```sh
cargo build --release --bin attoc
target/release/attoc --check prog.atto \
  --borrow-report prog.borrow.jsonl --compiler-git-sha "$(git rev-parse HEAD)" [--case BCV1-010]
# in atto-borrow-checker.rs:
atto-borrow-conform cross-check prog.borrow.jsonl
```

Without `--borrow-report`, nothing is recorded and behaviour is unchanged.
Exit codes and diagnostics are identical with or without the flag.

## What is exported

Only **top-level `fnc` bodies made of bindings, expression statements,
blocks, `return` and structured control flow** (`if`/`elif`/`else`, `while`,
`loop`, `for`, `break`, `continue`) are exported. The allowed expressions are literals,
names, unary and binary operators, calls, `new`, field reads and ownership
operators. Everything else is reported as `mirUnavailable` with a reason:

- `match` statements, ternaries and generator `for-in` loops;
- method calls (no receiver effect summary yet);
- closures;
- reassignment and field assignment;
- unwind, state and generator statements;
- actors and `self`;
- reborrows;
- class, actor and constructor bodies.

The exporter never emits a partial graph silently.

| Source | Canonical events |
|---|---|
| parameter / `val x = new C()` / scalar binding | `Init` (scalars have `copy: true`) |
| name read | `Read`; through a `borrow! mut` binder it is `WriteThrough` |
| `take! x`, or a class local in an argument, initializer or return | `Move` (retargeted into the binding when the initializer is exactly `x` / `take! x`) |
| `copy!! x` (flat class, AOT-authenticated) | `DeepCopy{graphProven:true}` |
| `val b = borrow! [mut] x` | `Borrow`; `EndBorrow` when the binder's scope ends |
| transferring a loan binder, or `borrow!` outside a binding | the loan **escapes**: it is never closed, so canonical reports `E_LOAN_ESCAPE` |
| `drop! x` | `Read`, `Drop` |
| `return` / end of body | `EndBorrow` for each live lexical loan, then `Return` |

## Branches and loops (structured-cfg-v1)

Control flow is exported as real CFG blocks; block 0 is the entry. The
master checker judges joins and cycles with its **hybrid** verdict: V0, then
`cyclic_v1` past V0's join/cycle limits (atto-borrow-checker.rs
`src/hybrid_v1.rs`).

- **`if` / `elif` / `else`:**
  - each condition is evaluated in a test block;
  - the true edge goes to the arm, the false edge to the next condition or
    `else`;
  - arms that fall through jump to a join block;
  - if every arm returns, there is no join and later code is dead.
- **`while` / `for`:** a header block evaluates the condition, then branches
  to the body or the exit. The body jumps back to the header, or to the update
  block for `for`.
- **`loop`:** same, but there is no exit unless a `break` jumps out.
- **`break` / `continue`:** jump to the exit / the header (or update),
  releasing the loop body's still-open locals on that edge.

Live loans are tracked per path. Each arm or loop body starts from the loans
live before it, and they are restored after it.

**Loop iterations must start fresh**, because canonical re-initialization
requires a definitely-moved place:

- at every back-edge, `break` and `continue`, loop-local loan binders get
  `EndBorrow` and loop-local owners that are still owned get `Drop`;
- a loop-local owner that is consumed on *some* paths only (MAY-moved,
  not MUST-moved) cannot be released exactly, so the function fails closed;
- inside loops, a move into an unbound temporary (a call argument) is
  followed by `Drop` of that temporary, because the callee owns it.

The trace keeps its own MUST-moved set per path (intersected at joins), so
an owner consumed on every path needs no release.

**Rejected prefixes inside loops:**

- A rejection in a loop *header* (e.g. `take!` in a `while` condition)
  keeps the repetition: the header re-runs or exits, so canonical sees the
  second-iteration use.
- A rejection deeper in an open loop body fails closed, because a truncated
  single-iteration witness would mislead.

Ids:

- owner = local slot + 1;
- loan = binder slot + 1;
- temporaries are numbered from 1000001.

Each event is recorded **before** the check that may reject it, so a rejected
function exports its prefix up to and including the failing op, followed by
loan closes and `Return`. Those records are marked `prefix-at-local-error`.

**Limits, stated in every record's `note`:**

- `expectedInstructionIds` is the event stream itself, not an independent
  typed-IR census.
- `send` is always `false`, because this compiler does not attest Send.

An L3 `AGREE` means this compiler's ownership decision matches the canonical
model **for the exported graph**. It does not prove the graph is complete or
that codegen honours it.

## structured-cfg-v1 cross-check (2026-11-28, hybrid verdict)

These counts are from `tests/fixtures/{pass,fail}` on main `56ba0cf3`: 1046
records.

| Class | Count |
|---|---|
| AGREE | 412 (7 decided by `cyclic_v1`) |
| BACKEND_PERMISSIVE | **0** |
| CODE_MISMATCH / ADAPTER_DEFECT | 0 |
| BACKEND_STRICTER | 2, the same compiler limits as before (`fail/168`, `fail/170`) |
| NON_OWNERSHIP_REJECT | 323 |
| NOT_EXPORTED | 309 |

**Where NOT_EXPORTED comes from:**

| Reason | Count |
|---|---|
| class, actor and constructor bodies | 73 |
| `match` | 56 |
| reassignment | 50 |
| method calls | 31 |
| generator `for-in` | 18 |
| named Boolean conditionals | 15 |
| actor/state access | 15 |

V0 already accepts joins whose arms leave identical ownership states, so most
`if`s need no `cyclic_v1`. The repeated-condition rejects (`fail/177`) agree
only through the loop-header prefix encoding.

## First cross-check (2026-11-21, straight-line-v1)

These counts are from `tests/fixtures/{pass,fail}`: 847 records over 515
programs.

| Class | Count | Meaning |
|---|---|---|
| AGREE | 235 | same verdict; the reject code conforms |
| BACKEND_PERMISSIVE | **0** | no unsound accept found |
| CODE_MISMATCH | 0 | |
| BACKEND_STRICTER | 2 | real compiler limits: mutable scalar loans (`fail/168`) and borrowing a temporary (`fail/170`); both are legal in the canonical model |
| NON_OWNERSHIP_REJECT | 273 | syntax, type or unsupported-feature rejects |
| NOT_EXPORTED | 337 | outside straight-line-v1; branches (159) and loops (45) dominate |

The first run also found five exporter mapping defects, all fixed and covered
by `tests/borrow_export_l3.rs`. Escaping loan binders and expression-level
borrows had been exported as moves or omitted.

BCV1 (`atto-platform-conformance`, 40 fixtures) gives 10 AGREE, 31
non-ownership rejects and 1 not-exported record.

**BCV1-003 is an L2 finding, not an L3 one.** This compiler (and the canonical
model, given its export) treats `inspect(original)` as a **move**. The spec's
implicit-argument-borrow-v1 says it should be a borrow. L3 shows the export is
faithful; L2 shows main does not yet implement implicit argument borrowing.
