# Integrating failure rows with unwind and Result (draft compatibility gate)

This branch is based on the verified #304 compiler and reconciles #305/#313;
it **does not** merge the unrelated full lowerings in #275 or #296.

## Required unified effect transfer

Track independent channels for ordinary guest throw, escalated raise, and fatal
panic, with nominal Error type sets per channel. Add control-flow exits and
handler-stack state to this summary before combining the runtime branches.
A handler may *discharge* a channel only if the complete operation is within
its protected scope and its transport/cleanup semantics are actually lowered.

- `trap e` evaluates `e` exactly once and discharges only ordinary throw.
  Error payload transport follows #275's **Result** contract, replacing the
  current browser `Option` placeholder. `raise` and `panic` remain untouched.
- `try ... catch` discharges only the matching ordinary throw types; a catch
  handler's outgoing effects and all `finally` effects remain in scope.
- `defer`/`recover` registration arguments execute at registration. Callback
  bodies execute during unwind; they cannot emit escaping throw/raise or break
  cleanup obligations. `recover` is not generally assumed to erase a caller's
  outstanding declared exception without actual handler precedence.
- `?` returns its error/None through a typed Result/Option pipeline and
  must not be interpreted as throw, raise, panic, or a runtime trap.
- `await`/cancellation/actor CONTROL must remain separate effect channels,
  with explicit cleanup and critical-state lease restrictions.

## Review gates for the #275/#296 bridge

1. Reconcile AST and typed-IR variants, including `Try`, `Raise`,
   `Defer`, `Recover`, `Trap`, `Result` and reachability, without
   replacing the newer parser/target scanner wholesale.
2. Use one monotone effect-closure solver shared by callable signatures,
   cleanup handlers, indirect calls and expression argument evaluation.
3. Add guest fixtures: nested catch/trap, throw during argument evaluation,
   recover precedence, recursive calls, definite return, fatal panic leakage,
   and cancellation/actor supervisor paths (fail-closed until executable).
4. Re-run **exact Git-tree SHA** Rust/wasm32/guest conformance. Do not count
   Java differential or other backends as passed when not executed.

The current browser target intentionally supports only built-in Error in
nominal rows. Full subtype checking and actor/runtime unwind parity remain
separate work, not assertions of this branch.
