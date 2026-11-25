# RFC: Atto Verified Core

**Status: Draft language proposal and implementation plan.** P0 establishes the first native guarantees, P1 extends the proof language, and P2 adds relational and temporal reasoning. Each feature becomes a language target through `atto-spec` review. Implementation and parity claims require the evidence defined below. Every Atto snippet in this RFC uses proposed syntax.

## Objective

Make units, refinements, contracts, and actor protocols part of Atto's type checker. Make stronger claims, including termination, representation independence, and temporal properties, explicit obligations that the compiler can check. The same source semantics apply to independent compilers. Baseline Atto typing, ownership, actor isolation, and checked operations remain in effect in every build.

The programming-language foundations are a typed operational semantics, an ownership and effect judgment, a program logic for contracts, a logical relation for parametricity, and an actor trace model. These also constrain data-oriented layout changes: a layout transformation must preserve value semantics, borrow granularity, observable failures, and drop behavior. Start from Atto's existing [language](https://github.com/atto-lang/atto-spec/blob/88119cbf1aaaccb0c8657697902b38521c86f76b/spec/language.md), [ownership](https://github.com/atto-lang/atto-spec/blob/88119cbf1aaaccb0c8657697902b38521c86f76b/spec/04-ownership-and-memory.md), and [DOP and real-time chapters](https://github.com/atto-lang/atto-spec/blob/88119cbf1aaaccb0c8657697902b38521c86f76b/spec/10-dop-and-realtime.md).

## Integration and existing work

| Existing work | Disposition |
| --- | --- |
| [Spec #209](https://github.com/atto-lang/atto-spec/pull/209) and [Rust #523](https://github.com/atto-lang/atto-source.rs/pull/523) | Reconcile numeric semantics, dimension identity, and conversion rules in `atto-spec`. Adapt the Rust algebra to nominal source dimensions and checked value-dependent exact conversion. Treat its existing SI-vector grammar and constant-factor conversion as prototypes. |
| [Spec #204](https://github.com/atto-lang/atto-spec/pull/204), [Rust #514](https://github.com/atto-lang/atto-source.rs/pull/514), [Java #792](https://github.com/atto-lang/atto-source.java/pull/792) and [#1092](https://github.com/atto-lang/atto-source.java/pull/1092) | Reuse interval normalization, checked construction, and nominal identity work where the semantics agree. Keep refined aliases and opaque newtypes distinct. Java's migrated parser and proof substrate and Rust's restricted nominal path establish only their separately evidenced scopes. |
| [Common source #80](https://github.com/atto-lang/atto-common-source/pull/80) | Evaluate its checked-arithmetic and typed-IR fixtures as a reference oracle. Keep the formal judgments and observable semantics independent of its experimental IR serialization. |
| [Actor trace refinement v2](https://github.com/atto-lang/atto-spec/blob/88119cbf1aaaccb0c8657697902b38521c86f76b/formal/actor_trace_refinement_v2.md) and [platform conformance](https://github.com/atto-lang/atto-platform-conformance/blob/89f35186c0f41235bc5be25d1c4342af3ff5ec24/contracts/actor-trace-v2.json) | Extend the trace with protocol edge, generation, and outcome at actual guest commit points. Reuse mailbox, CONTROL, closure, and settlement invariants. |
| [LLVM #895](https://github.com/atto-lang/atto-source.llvm/pull/895) | Use the core trust and evidence model as the dependency for the separate `Pointer<T>`, separation logic, and machine-code validation proposal. |

The Rust, LLVM, and Java RFCs share this text. The formal semantics and accepted grammar belong in `atto-spec`. Versioned source fixtures and expected semantic observations belong in `atto-platform-conformance`. Each compiler owns its parser, checker, and lowering. The formatter and editor grammar consume the same language decisions.

### Source grammar and compatibility

Reuse `define`, module bodies, typed callable returns, existing effect syntax, and ownership operators such as `take!`, `borrow!`, and `share!`. Use `where` for refinement clauses at a type boundary and preserve existing generic constraints. Read `and units in` as a contextual clause separator. Use the established boolean operators inside predicates. Resolve `Time` and `Length / Time` as dimension expressions and `time.Millis` as a unit expression by symbol kind. A module name alone, such as `time`, never stands for a dimension.

A first source audit used the current main revisions of the [Rust parser](https://github.com/atto-lang/atto-source.rs/blob/a4c2497decbb3782a1a6f810e8078bbee094ccbb/src/parser.rs), [LLVM native parser](https://github.com/atto-lang/atto-source.llvm/blob/8ed25fd2ab19a21d9ec3291d42109d48229751af/src/main/cpp/oreslang/frontend/Parser.cpp), and [Java lexer](https://github.com/atto-lang/atto-source.java/blob/b02efee4c036557278284ca444655e19cd0adc64/src/main/java/dev/oreslang/parser/Lexer.java). Rust already parses `pure`. LLVM treats `pure` and `requires` contextually and currently rejects native `requires` at a real-time-analysis gate. Java's main lexer uses a reserved-word table, and its separate refinement branch already parses forms of `where`. Preserve the existing purity and real-time behavior. Resolve the scope and ordering of functional and real-time clauses in the grammar before enabling functional contracts on that path.

| Phase | Proposed vocabulary |
| --- | --- |
| P0 | `define dimension`, `define unit`, `define protocol`, `where`, `units in`, `requires`, `ensures`, `invariant`, `pure`, `assert`, `initial`, `terminal`, `on`, `transition` |
| P1 | `proof`, `ghost`, `lemma`, `spec`, `assume`, `trusted`, `decreases`, `forall`, `exists`, `old`, `reads`, `modifies` |
| P2 | `parametric`, `temporal`, `always`, `eventually` |

New spellings start contextual. `result` and `self` are scoped binders in postconditions and refinements respectively. Audit the LLVM Rust frontend if the supported command can route through it. Test identifiers, member names, comments, strings, named arguments, nested generics, `5<time.Millis>` versus comparisons, and a unit-suffixed negative literal. A missing implementation rejects recognized syntax before executable IR. Reserve new words only after a compatibility review and extend the [keyword taxonomy](https://github.com/atto-lang/atto-spec/blob/88119cbf1aaaccb0c8657697902b38521c86f76b/spec/23-keyword-taxonomy-and-highlighting.md). Induction, rewriting, and similar techniques can be proof-library operations.

## Formal core and compiler contract

Publish a versioned core with syntax and elaboration, a store and loan state, expression values and evaluation steps, actor traces, and observable outcomes. The initial judgment `Γ; regions ⊢ e : τ ! ε` assigns a type and effects to an elaborated expression. The model distinguishes normal return, typed failure, panic, cancellation, suspension, and divergence. Evaluation order, mutation, ownership transfer, cleanup, and calls across trust boundaries are part of the model.

The first model covers fixed-width checked scalars, conditionals, bounded refinements, calls, and local moves and borrows. Specify arithmetic with mathematical integers plus an explicit range precondition at each checked operation, or equivalent bitvectors with overflow predicates. Division checks zero and signed `MIN / -1` as separate conditions. Within the proposed sized-integer profile, successful division truncates toward zero. Establish this alongside [Spec #209](https://github.com/atto-lang/atto-spec/pull/209) before relying on it in a guest. Record the actual backend failure class when validating a dynamic check.

For the modeled fragment, prove preservation and progress up to the modeled outcomes, soundness of generated verification conditions, and noninterference of erased ghost/proof terms. For every claim, publish a theorem statement, assumptions, mechanized proof and pinned checker invocation. Use an executable reference evaluator and differential tests to check elaboration and lowering independently of tests for the proof engine. Decide the proof assistant and supported arithmetic lowering at the spec checkpoint. New constructs extend the formal core before a compiler enables them.

Compilation follows source → resolved typed HIR → ownership and effect checks → verification IR and obligations → executable MIR with proof-dependent operations guarded or justified → backend lowering. Verification consumes elaborated, kind-checked terms. It cannot introduce borrowing permissions, mutate executable state, or erase a check merely because a source annotation named a property. Code generation must use the same typed operations and semantics that generated the obligations.

## P0 — Units, refinements, contracts, and local protocol safety

### Dimensions, units, and conversions

A dimension is a nominal symbol and normalizes to a sorted vector of integer exponents for the first profile. A unit has a resolved symbol, a normalized dimension, and an exact positive rational scale. Each base dimension has one canonical unit. A unit declaration that aliases another unit retains a distinct name for diagnostics and exports. Equal normalized dimensions and scales are compatible in expressions. Unknown symbols, wrong kinds, cycles, invalid scales, and metadata support-limit overflows are stable diagnostics. Rational exponents and affine point/delta units require separate reviewed rules before admission.

A quantity carries its numeric representation, normalized unit, and any refinement in typed HIR. `units in time.Millis` fixes a concrete unit. `units in Time` constrains a dimension and requires a concrete scale to be available where needed. P0 admits concrete units first. An abstract dimension admits operations only with a resolved or explicitly quantified unit in the applicable phase. Existential or reflected runtime units require their own representation and are outside P0.

Add, subtract, compare, assign, and pass by value only with matching normalized dimensions and scales. Multiplication and division combine both vectors and exact scales. A result enters an unannotated scalar type only when its dimension vanishes and its scale is one. Seconds divided by milliseconds can produce a dimensionless quantity with a non-unit scale. Its normalization is an explicit conversion.

Explicit conversion uses the source/target ratio with cancellation before range checks. Integer exact conversion succeeds only when the particular value converts to an in-range integer. Thus 5,000 ms converts to 5 s, while 1,500 ms needs a checked inexact branch or a separately selected rounding operation. Constant failures are diagnostics. A dynamic exact conversion returns a typed error for either inexactness or range failure. Round only by an explicit policy and recheck destination refinements. Static unit metadata erases from internal execution, while typed module signatures retain a stable versioned unit identity. Validate numeric, unit, and refinement ingress through foreign, wire, mutable, or versioned boundaries before giving the caller a refined quantity. P0 can check `f64` unit algebra independently of floating-point value proofs, which require a specified model for rounding, NaN, infinity, and signed zero.

```atto
define dimension Time;
define dimension Length;

define pub module time as
    define pub unit Second: Time;
    define pub unit Millis: Time = Second / 1000;
    define pub unit Micros: Time = Second / 1000000;
end

define pub module length as
    define pub unit Meter: Length;
    define pub unit Kilometer: Length = 1000 * Meter;
end

type PositiveMillis = i32
    where self > 0
    and units in time.Millis;

type Distance = f64 where units in length.Meter;
type Seconds = f64 where units in time.Second;
type Velocity = f64 where units in length.Meter / time.Second;

val timeout: PositiveMillis = 500<time.Millis>;
fnc ratio(PositiveMillis a, PositiveMillis b) -> i32 pure {
    return a / b;
}
fnc speed(Distance distance, Seconds duration) -> Velocity {
    return distance / duration;
}
```

`ratio` yields a dimensionless integer. A positive signed divisor excludes both zero and `-1`. A plain integer divided by `PositiveMillis` yields inverse time. The type of `speed` establishes its physical dimension. A separate floating-point contract would establish a particular value or guarantee on failure.

### Refinements and contracts

An alias `{x : τ | P}` restricts values of its base type. It introduces no nominal identity. An opaque newtype uses a separately resolved nominal declaration and reviewed construction and extraction rules. P0 starts with pure decidable scalar predicates and interval or linear-integer reasoning. Multiplication of unknowns, floating-point value properties, arbitrary heap logic, and general recursion require later supported proof rules. A restricted dependent-style index such as `Array<T, N>` is checked at compile time with a well-kinded constant or a supported symbolic index. Use the const-generic shape work for this slice. Full dependent elimination is outside the initial design.

Introduce a refined value only through a static proof or an explicit checked constructor whose successful result carries the proposition. Apply the same rule at assignment, destructuring, calls, returns, imported interfaces, and deserialization. Reject unchecked strengthening or laundering through casts or untyped boundaries. Invalidate facts after mutations, mutable aliases, or unknown effects on the places they describe. Snapshot dependencies and frames make any fact that survives a call or suspension explicit. Base-type range predicates over `self` use its representation in the unit fixed by the declaration.

The caller establishes each precondition, the callee may use it at entry, and the callee establishes postconditions at each normal return. Publish contracts with resolved imports and versioned semantics. An override must accept at least the base precondition and guarantee at least the base postcondition, with compatible effects. A postcondition governs normal return unless a separately specified failure contract applies. A loop invariant must hold at entry and be inductive on its back edge. Only checked exit conditions and the invariant flow past the loop, and termination is a separate obligation. Unreachable code can use contradiction only when infeasibility is derived from sound facts. Any obligation conditional on an assumption retains its provenance.

```atto
fnc halve(PositiveMillis duration) -> PositiveMillis
    requires duration >= 2<time.Millis>
    ensures result > 0<time.Millis>
    ensures result <= duration
    pure
{
    return duration / 2;
}

fnc first<T, const N: usize>(Array<T, N> values) -> T
    requires N > 0
{
    // Illustrative ownership-aware extraction from a consumed array.
    return take! values[0];
}
```

The indexed example fixes the bounds obligation. Its move and drop behavior still needs the array ownership rules and backend admission. `pure` uses the existing effect discipline. A proof of normal termination also needs to exclude divergence and the relevant failure effects.

### Actor protocol safety

`define protocol` names a finite local state machine. Require one explicit initial state, resolved states with optional terminal markers, and at most one edge per source state and message tag in the first profile. Terminal states have no outgoing application edges. Message signatures include payload types and ownership modes. A typed handler is checked against the incoming edge and must select its destination on successful completion. It cannot send a transition by updating an untyped state field.

The handler below shows just the capture edge. A complete actor must cover every edge it implements, including explicit failure handling.

```atto
define protocol Payment as
    initial Pending;
    Pending -> Authorized on authorize;
    Pending -> Rejected on reject;
    Authorized -> Captured on capture;
    Authorized -> Cancelled on cancel;
    terminal Captured;
    terminal Cancelled;
    terminal Rejected;
end

define actor PaymentProcessor implements Payment as
    capture() -> void
    {
        transition Captured;
    }
    // Other edge handlers follow the same rule.
end

type Timeout = i32
    where self > 0
    and self <= 60_000
    and units in time.Millis;

define protocol Connection as
    initial Disconnected;
    Disconnected -> Connecting on connect(Timeout timeout);
    Connecting -> Connected on connected;
    Connecting -> Failed on fail;
    Connected -> Disconnected on disconnect;
    terminal Failed;
end
```

P0 starts with sealed, non-suspending transition handlers on a serial protocol lane, even when the actor runtime supports concurrency. Type-check state access with the existing critical-region and ownership semantics. Compile the lane to a linearized runtime operation that validates the message and actor generation, checks the edge against the current state, stages local updates, and publishes the destination and updated state atomically. Verify actor invariants at creation and before publication and keep the staged region free of externally visible effects. For example, a connection actor can maintain an elapsed-time invariant of `elapsed >= 0<time.Millis>` while consuming a bounded `Timeout` on the `connect` edge. A failure before publication leaves the original state intact or invokes the specified supervisor lifecycle event. A failure after publication follows the declared lifecycle without rolling back a committed edge. A restart creates a new incarnation, and old-generation messages cannot advance it. Specify message ownership and settlement for both admission rejection and dispatch failure so move-only payloads are returned or disposed once according to the existing send contract.

A statically typed endpoint may establish a source state using a linear capability bound to the incarnation and consumed by the operation. An ordinary actor handle cannot establish current protocol state by call order or an unenforced type tag. A late, stale, or out-of-order message is rejected at the runtime dispatch boundary without publishing a transition. Record transition commits alongside mailbox events and preserve observable ordering under races. Suspended handlers, concurrent state-dependent handlers, hot-upgrade migration, and cross-actor session completion each need additional rules before they can share this guarantee.

Track invariant establishment at initialization, local preservation across publication, and failure isolation as distinct obligations. The safety theorem states that every committed application transition is a legal edge with a valid payload and preserves the actor invariant. Proving deadlock freedom or eventual delivery requires the later global model.

## P1 — Explicit proofs, frames, and trusted boundaries

- `spec` functions, `ghost` state, `proof` blocks, and `lemma` declarations live in a terminating proof fragment. Information may flow from runtime to ghost state through admitted snapshots. Ghost values cannot affect executable control flow, outputs, layout, resource lifetime, drop, dispatch, or scheduling. Test erasure against the executable MIR and observable traces.
- `old(e)` in a postcondition denotes a pre-state snapshot of its declared read footprint. `reads` and `modifies` are checked against actual effects and aliasing. A `decreases` expression is an explicit measure in a well-founded order. Begin with nonnegative integer measures and require strict decrease on admitted recursive calls or loop iterations.
- `forall` and `exists` denote quantification with a resolved domain. Start with bounded, enumerable domains and checked witnesses. Solver-supported general quantification requires an explicit theory and a sound proof path. Solver exhaustion is an unknown result.
- `assert(P)` creates a static obligation in proof code. A runtime assertion is an explicit checked operation whose fact becomes available on successful continuation only. `assume(P)` and `trusted` require scoped, reviewed trust identifiers and appear in transitive dependency evidence. Track imported summaries, foreign calls, native intrinsics, axioms, and proof-engine dependencies. Reject an inconsistent approval context when detected, including a literal `assume(false)`, and reject any attempt to discard the dependency by import, caching, or inlining. Approval records the scope of trust and cannot establish consistency.

```atto
lemma positive_sum(i32 x, i32 y) -> void
    requires x > 0
    requires y > 0
    requires x <= i32.MAX - y
    ensures x + y > 0
{
    proof {
        assert(x + y > 0);
    }
}
```

## P2 — Relational parametricity and temporal verification

### Generics and representation independence

Ordinary `<T>` is Atto's mechanism for reuse, declared bounds, and specialization. `<parametric T>` adds a compiler-enforced relational obligation to that parameter. Keep default and named binding semantics. Kind-check types, compile-time constants, dimensions, units, protocols, effects, and lifetime binders independently. Preserve Atto's `lt!`/`lifetime!` ownership vocabulary. Phase admission is separate for each kind. P0 needs concrete dimensions and units, typed constant indices can follow the bounds work, and unit, dimension, protocol, and effect polymorphism require their own elaboration rules.

Begin with pure, total code over owned abstract values and compiler-specified products, sums, and containers. For each admissible relation `R` between interpretations of `T`, related input values must produce results related by the interpretation of the result type. Prove the fundamental theorem for typed terms of the fragment. Specify which observations the relation preserves and how relations lift through aggregates, callbacks, and ownership. Exclude unresolved `T`-dependent reflection, type identity, layout, address identity, casts, user-defined drop effects, and unmodeled foreign or type-specific behavior. A bound admits an operation only with a matching relational rule. The first effectful extensions require a relation over outcomes and traces, including transfer, mailbox behavior, cancellation, and failure.

```atto
fnc identity<parametric T>(T value) -> T pure {
    return value;
}

fnc describe<T>(T value) -> String where T: Display {
    return value.to_string();
}

fnc square<unit U>(x: (f64 where units in U))
    -> (f64 where units in U * U) pure
{
    return x * x;
}
```

Pure total `identity` returns its input under the abstract-value semantics. A parametric `reverse` needs its own proof to establish a length or permutation contract. A parametric `map` relates corresponding callbacks. They must map related arguments to related results and their effects must meet the enclosing contract. A generic actor `send_value` requires a trace relation for the relevant send outcomes. Backend specialization may vary layout and code generation while preserving the declared observations. Kennedy's dimensional-invariance theorem concerns its underlying numeric model. Claim unit-rescaling invariance in Atto only for an explicitly proved fragment with rounding, truncation, overflow, and failure accounted for. [Wadler, Theorems for Free!](https://www-verimag.imag.fr/~perin/enseignement/Magistere/articles/Wadler-ICFP%271989-structure.pdf), [Kennedy, Relational Parametricity and Units of Measure](https://www.microsoft.com/en-us/research/publication/relational-parametricity-and-units-of-measure/)

```atto
// Signature sketches. The list and actor APIs must be resolved for admission.
fnc reverse<parametric T>(List<T> values) -> List<T>
    ensures result.length == old(values.length)
    pure;

fnc map<parametric T, parametric R>(List<T> values, fnc(T) -> R transform)
    -> List<R> pure;

fnc send_value<parametric T>(Actor<T> actor, T value) -> Result<(), SendError> {
    return actor.send(take! value);
}
```

### Temporal properties

`temporal`, `always`, and `eventually` describe properties of versioned execution traces. Interpret `always` over reachable states and `eventually` over complete traces under explicit progress assumptions. Include scheduling fairness, mailbox bounds and ordering, CONTROL priority, cancellation, supervision and restart, external delivery assumptions, and the trace alphabet. For example, `always(no_commit_after_close)` needs the declared close and incarnation semantics. Progress for an admitted connection attempt may state that it reaches Connected, Failed, or cancellation, conditional on a schedulable handler and eventual delivery of the relevant external event.

Model-check small actor topologies first. Record model digest, topology and bounds, fairness constraints, explored state/transition counts, and counterexample traces with step-level provenance. Bounded success supports the stated finite model. A global liveness claim needs a complete model or an induction/refinement argument linking the bounded result to deployed semantics. Connect source checks to real scheduler and mailbox traces under the trace-refinement contract. Keep any relaxed runtime evidence scope explicit.

## Verification status, build behavior, and proof evidence

| Outcome | Meaning and build rule |
| --- | --- |
| `proven` | A supported obligation was discharged under the identified semantics and proof-engine trust base, with no application-level trust dependency. Runtime evidence and backend validation have their own scopes. |
| `refuted` | A checked counterexample or a decisive checker rule establishes failure. Contracts and guarantees with a refuted obligation fail compilation. An invalid certificate or an unvalidated solver candidate supplies no refutation by itself. |
| `unknown` | Inconclusive, timed out, unsupported, or evidence missing. Strict builds reject it. For an explicitly checkable site, an ordinary build may emit an explicit runtime guard. A failed or unavailable guard cannot grant the type or contract. |
| `assumed` | The derivation depends on an identified application-level assumption or trusted summary. Strict builds accept the claim only under a matching explicit approval policy and report it as conditional. |

Ordinary builds always enforce type, ownership, unit, and supported protocol rules for code using them. Ordinary contracts can use a configured checked boundary whose failure behavior and cost are visible. Type refinements carry evidence from that boundary's successful continuation. Strict verification (proposed CLI spelling `atto build --verify=strict`) rejects unknown, refuted, missing, unsupported, or runtime-only proof obligations and unapproved trust dependencies. A verified protocol may deliberately specify a dynamic dispatch guard, whose enforcement and outcomes are themselves proof obligations. An absent verifier or unsupported backend cannot silently fall back to accepting lowered artifacts. Keep compile-time and runtime enforcement status distinct in tooling.

Define a versioned manifest validated by an independent reader. Each obligation carries a canonical ID, kind, statement/IR digest, module identity, source span and success/failure scope. Bind the manifest to source commit/tree and fixture where applicable, dependency and imported-summary digests, semantic/numeric/policy profiles, build flags, compiler and verifier versions, proof or counterexample digests, assumption dependency IDs and approvals, and target/artifact digest when claimed. Persist the evidence class, enforcement method, and any runtime checks left in the executable. Stable IDs and deterministic resource limits should allow reproducible triage. Never reuse cached proof evidence across changes to transitive summaries, toolchain, target-sensitive semantics, or approval scope. Test stale, malformed, substituted, and incomplete manifests and certificates.

State the trusted computing base for each profile, including elaboration, obligation generation, proof kernel or trusted solver, lowering, runtime and host boundaries as applicable. Where certificates are supported, strict certificate profiles require independent checking. Baseline solver-backed profiles must name the exact solver/theories they trust. Source proof establishes a property of elaborated Atto. Executable attestation additionally requires semantic preservation or translation validation of the relevant lowering and trusted runtime assumptions tied to the artifact. Unsupported instructions, foreign calls, or optimizations narrow the attested scope. [CompCert semantic preservation](https://compcert.org/man/manual001.html)

## Delivery sequence and exit gates

| Slice | Implementation responsibility and dependency | Exit evidence |
| --- | --- | --- |
| P0.0 Core and grammar | `atto-spec` proposes the versioned formal core and syntax, resolves #209/#204 grammar choices with compiler and tooling owners, and defines stable diagnostics and manifest vocabulary. | Reviewed grammar, executable reference semantics, first preservation and VC soundness lemmas, parser/formatter non-regressions, and a signed-off numeric profile. |
| P0.1 Concrete quantities | Reuse the Rust algebra where compatible. Rust, LLVM, and Java implement declarations, kind resolution, arithmetic, explicit conversions, and executable lowering through their normal paths. | Cross-backend quantities corpus, exact conversion and arithmetic-edge tests, and unit-compatibility diagnostics before IR. |
| P0.2 Refined values and contracts | Reuse interval checkers and constructor paths. Integrate scalar refinements, checked ingress, pre/postconditions, assertions, simple invariants, and named strict/ordinary policy. Add indexed bounds incrementally. | Checked positive and negative contracts, boundary and mutation tests, real guest failures, independent manifest verification, and no proof-dependent check removal without evidence. |
| P0.3 Local protocols | Add typed edges and payloads to actor checkers, prove local preservation, and implement serial dispatch integration against the trace model. | Paired positive and negative guest traces with races, stale messages, failure/cancel/stop behavior, exact-once ownership settlement, and invariant restoration. |
| P1 Proof language | Add frames, snapshots, ghost erasure, tactics as library operations, termination, quantifiers and audited trust after core obligations and manifests stabilize. | Proof checker and erasure tests, frame/call/FFI negatives, assumption-taint and cache invalidation checks, and strict verification evidence. |
| P2 Relational and temporal | Prove the first logical-relation theorem, then extend kinding and effects. Model-check protocols and independently refine the model against compiled guest traces. Validate native lowering as a separate evidence track. | Free-theorem and counterexample fixtures, effect and callback checks, temporal safety/liveness receipts with modeled bounds, and artifact-linked translation evidence for any machine-code claim. |

The first runnable vertical slice combines `PositiveMillis`, a proven `halve`, checked and rejected quantity conversions, and a `Connection` message carrying a checked `Timeout`. The work can stage the typed P0.3 branch behind P0.1 and P0.2, while the draft RFC remains reviewable on its own. Never count completion of an earlier slice as a claim about the rest.

### Mandatory conformance matrix

| Claim | Positive case | Negative or failure case |
| --- | --- | --- |
| Unit identity and algebra | `5000 ms → 5 s`, same-scale ratio, exact unit aliases, and a velocity quantity | Wrong dimension or scale, inverse time assigned to a scalar, non-unit scale erased, cyclic/zero metadata, inexact conversion, narrowing overflow |
| Refined values and fixed-width arithmetic | Construct positive milliseconds, `halve(2 ms)`, nonempty indexed array | Reject negative, zero or wrong-unit input, invalid bounds, `MIN / -1`, division by zero, false contract, mutable refinement invalidation, fabricated or inconsistent external value |
| Contracts and proofs | Verified caller/callee pair, invariant initiation and preservation, explicit approved trust chain | Refuted postcondition, weakening an inherited guarantee, timeout, malformed certificate, stale cache, hidden transitive assumption, `assume(false)` even with an approval, ghost flow into runtime |
| Actors and protocols | Payment authorization then capture, Connection timeout payload, successful atomic state and invariant update | Illegal compile-time edge, dispatch before authorization, wrong payload, duplicate or stale generation, stop/restart race, invalid terminal transition, invariant-breaking update, leaked or double-consumed message |
| Parametricity and temporal | Pure total identity, relational map, well-kinded unit square, eventual outcome under explicit fairness | Reflective or divergent identity, representation-sensitive specialization, impure or unrelated callback, false list-length theorem, unit-kind mismatch, scheduler starvation, trace/model mismatch |
| Syntax and backends | Parse and format contextual spellings and run the same accepted sources and observables across Rust, LLVM and Java | Legacy identifier capture, text-only detection, changed meaning on round trip, required rejection emitted as executable IR, backend host fallback, forged guest receipt |

Use the existing [conformance policy](https://github.com/atto-lang/atto-spec/blob/88119cbf1aaaccb0c8657697902b38521c86f76b/spec/11-conformance.md). Reference evaluators, parser fixtures, static checks, compiled guest execution, and machine-code validation establish different claims. For an execution claim build each supported compiler at an immutable source revision and execute equivalent source and negative cases on the real path, including native/AOT or browser Wasm where claimed. Compare diagnostics, values, failures, evidence scope, and traces. Record origin head/tree, fixture blob, compiler/toolchain and artifact digests, wrapper/mode, passing and failed cases, and authenticated run/job/step evidence. Mark other Atto backends pending until their own receipts satisfy the relevant gate. An authorized private exact-tree Actions carrier uses its own token, verifies paths/modes/blobs/tree, and preserves required origin checks.

Before normative adoption, resolve the overlapping syntax with the numeric and newtype proposals, select the checker and arithmetic representation, and agree on the callable and actor syntax and stable exported unit/protocol identities. Broader affine units, floating-point proofs, temporal profiles, full dependent types, and effectful parametricity each require a bounded scope and their own exit evidence.


## P0.2 first slice — absence of implicit arithmetic failures

**Proposed strict safety contract; HOST_REFERENCE_ONLY.** Preserve the existing [numeric semantics](https://github.com/atto-lang/atto-spec/blob/main/language/numeric-semantics-v1.md): ordinary dynamic overflow and zero division are typed guest throws (candidate codes 65537/65538), not fatal panics. The strict profile requires proof of safety or explicitly represented and handled failure; it must not reinterpret ordinary programs or weaken checks. A verified no-implicit-failure callable may return Result/Option as ordinary data, but escaping implicit throw, raise or panic is inadmissible. Explicit fatal operations are outside that guarantee. Resource exhaustion and host/FFI faults retain their separately validated containment scope.

The [arithmetic reference oracle](verified-core-arithmetic-oracle.mjs) uses exact BigInt interval arithmetic for signed addition, subtraction, multiplication, negation and division. Run `node docs/rfcs/verified-core-arithmetic-oracle.mjs`. Both zero divisor and MIN/-1 must be excluded for division. Proven means every pair in the inclusive Cartesian interval is safe; refuted in this small oracle means every pair fails (not a reachable source counterexample); otherwise unknown. Missing, invalid or unsupported facts never certify safety. No check may be removed on this host-reference result alone.

Production integration must derive typed facts from SSA definitions and invalidate them on assignment, mutable aliases, effectful calls, suspension, actor guard release and joins that lose correlation. Evaluate eager operands before folding, preserve lazy branch semantics and exceptional ordering, and prove conversions and indexing separately. At backend lowering preserve signed width, zero/overflow distinction and catchability, and avoid LLVM poison, Wasm trap substitution and JS Number precision loss. Proof-dependent check elimination requires source-to-artifact preservation and trusted dependency scope.

### Evidence and overlap disposition

- Reference model execution: 223601 assertions; 18496 signed 4-bit interval pairs, 2664240 concrete oracle evaluations, 17 targeted i64 cases. Executed as self-contained JavaScript; fetched committed source matches tested text. Not compiled Atto guest, solver proof or backend certification.
- Rust [#439](https://github.com/atto-lang/atto-source.rs/pull/439) owns the restricted constant/checked arithmetic path but reports fatal Wasm panic code 1, which differs from the current ordinary-throw target. It is explicitly on security/release hold; do not merge, export or relabel historical carrier results. Reconcile exception identity and catchability before integration.
- Rust [#453](https://github.com/atto-lang/atto-source.rs/pull/453) owns older stacked native MIR constant facts; compare typed SSA facts and preserve nonduplicative tests rather than create a third constant evaluator.
- Java [#1799](https://github.com/atto-lang/atto-source.java/pull/1799) and LLVM [#894](https://github.com/atto-lang/atto-source.llvm/pull/894) are companion RFCs. Normative changes belong in atto-spec; shared guest fixtures belong in atto-platform-conformance. This increment adds no production syntax or codegen and is not a replacement RFC.
- Next acceptance gate: integrate proof obligations in a current-main compiler through existing checked operations, with safe guarded division, literal zero, MIN/-1, unknown denominator, eager unused overflow, lazy dead branch, stale facts after rebinding/suspension, handled typed throws and no emitted artifact on strict rejection. Execute identical guest sources across independently pinned native/AOT and Wasm paths. Unsupported and unexecuted cases remain explicit.
