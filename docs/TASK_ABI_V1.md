# Guest task ABI `atto.task.v1` (susp / suspends / await / task)

Status: implemented in the Rust/Wasm compiler; executed in browsers by the
playground's OresVM control loop (`atto-lang.github.io/play/src/ores-*.js`).
It mirrors the Java reference `OresScheduler` turn contract
(`Done` / `Await` / `Cooperate`, scheduler-owned continuation placement).

## Source subset

| Form | Meaning |
| --- | --- |
| `susp fnc f(..): T` / `suspends fnc ...` | Suspending callable. `T` is `int`, `bool` or `void`. `susp` and `suspends` are one effect; writing both is `E_DUPLICATE_EFFECT`. |
| `pub susp fnc main(): int` | Required when any susp callable exists. The module then exports the task ABI instead of `main`. |
| `await f(args)` | Run the susp callable `f` first (like a call that may suspend), resume with its result. |
| `val t = task { ... }` | Start a task now; `t: Future<int>`. The body must return `int`. |
| `do task { ... };` | Start a detached task (no Future, no value). |
| `await t` | Park until the `Future<int>` settles. |
| `await sleeper(100ms)` | Park on a host timer. Duration literals `Nms` / `Ns` are only admitted here; an `int` millisecond expression also works. `0ms` yields. |

`await` is legal only as a statement-level operand inside a top-level susp
callable: `val x = await e;`, `x = await e;`, `await e;`, `return await e;`.
Nested awaits (`await f(await g())`, `stdio.println(await f())`) are hoisted
into temporaries in left-to-right order. Awaits behind `&&`/`||` right
operands, ternary arms, `elif` conditions, `for` headers or match guards are
rejected rather than evaluated eagerly. `while (await c)` is rewritten to a
loop that re-awaits `c` before every iteration.

Task bodies capture enclosing **immutable** bindings by value (`val`,
`const`, non-`mut` parameters). Capturing or assigning a `let` binding is
`E_TASK_CAPTURE`: a task never aliases a caller's mutable local.

Not lowered yet (positioned diagnostics, no artifact): suspending lambdas,
susp methods / actor receive handlers, `try`/`defer`/`recover` in susp
callables, await inside generator `for-in` bodies, `Future<T>` other than
`int`, select/channels/cancellation.

## Lowering

The ordinary checker types a susp body; `await` becomes `TStmt::Suspend`.
The task planner (`src/check/task_plan.rs`) flattens only statements that
must be resumable (they suspend, return, or `break`/`continue` out of
themselves) into PC-addressed steps; everything else stays an opaque `Run`
segment emitted by the ordinary statement emitter. Each susp callable becomes
`resume_k() -> i32`; all parameters and locals live in frame slots and are
reloaded on every resume, so nothing depends on a discarded Wasm stack.
Back-edges of flattened loops spend a per-turn fuel budget (4096) and then
`COOPERATE`, so a long suspending loop cannot monopolize the tenant.

## Guest surface

Custom section `atto.task.v1` = one byte `01`. Exports (no new imports):

| Export | Signature | Meaning |
| --- | --- | --- |
| `atto_task_main` | `() -> i64` | Frame handle of the root task (main). |
| `atto_task_resume` | `(i64 handle, i64 wake) -> i32` | Run one turn; `wake` is the value the previous suspension resolved to. |
| `atto_task_request` | `(i64 handle) -> i64` | Payload of the last suspension (child frame handle, Future token or milliseconds). |
| `atto_task_result` | `(i64 handle) -> i64` | Result of a DONE task (`bool` widened, `void` = 0). |
| `atto_task_close` | `(i64 handle)` | Recycle a LIVE/DONE frame. |

Step status from `atto_task_resume`:

| Status | Name | Request | Host action |
| --- | --- | --- | --- |
| 0 | DONE | — | Read result, close frame, wake awaiters. |
| 1 | CALL | child frame | Run child before other ready work; resume caller with its result. |
| 2 | AWAIT | Future token | Park until settled; await cycles are `E_DEADLOCK`. |
| 3 | SPAWN | child frame | Register child (FIFO), resume caller first with a new Future token. |
| 4 | DETACH | child frame | As SPAWN, resume caller with 0. |
| 5 | SLEEP | milliseconds | Park on a reactor timer (0..60000 ms). |
| 6 | COOPERATE | — | Requeue behind already-ready work. |

Frames are fixed 552-byte, 8-aligned, generation-fenced arena blocks
(handle = `generation << 32 | offset`); every export revalidates the
handle and state, so a stale, forged or recycled handle traps with
`panic_code = 7` (invalid task handle/state). Future tokens are host-issued;
the guest never sees another task's frame handle.

Programs without susp callables are byte-identical to the previous compiler.
