# RFC: Atto Verified Core (draft)

Status: **Proposal only — no language syntax or compiler implementation is approved by this RFC.**

## Motivation

Make verification endemic to Atto: automatic ownership, effect and dimensional checks; opt-in contracts and refinements; explicit proofs for higher assurance. Verification must never silently treat a timeout or unsupported obligation as a proof.

## Proposed layers

1. **Baseline static guarantees:** existing type and borrow checking, move/borrow/share operations (prefer `take!`, `borrow!`, `share!`), initialization, exhaustive matching, sealed actor state, and checked arithmetic.
2. **Dimensional typing:** units of measure as type-level dimensions, multiplication/division/powers, dimensionless values, unit polymorphism, explicit conversion and separate handling for affine units.
3. **Contracts/refinements:** proposed `requires`, `ensures`, `invariant`, `decreases`, refinement predicates, and overflow-aware numeric semantics. Syntax is illustrative, not adopted.
4. **Parametricity:** a carefully defined subset of pure, total, representation-independent generic code with free-theorem reasoning. Reflection, unsafe operations, panic/divergence, pointer identity and observable effects must be accounted for.
5. **Actor verification:** invariants at handler boundaries, ownership of messages, state sealing, suspension/critical-region safety, optional protocol-state typing and model-checked liveness under explicit fairness assumptions.
6. **Proof integrity:** verification IR, SMT verification conditions, proof checking, optional model checking, proof/assumption manifests and backend lowering obligations.

## Verification outcomes

- `proven`: discharged within stated model and assumptions.
- `refuted`: failed obligation, ideally with counterexample.
- `unknown`: solver timeout, unsupported theory or inconclusive result.
- `assumed`: explicitly declared trusted boundary, visible in build artifacts.

Strict verification must fail on `unknown` and unapproved `assumed`. Runtime checks may be a separately configured fallback, never represented as static proofs.

## Illustrative examples (NOT yet valid Atto syntax)

```text
unit meter;
unit second;
type Velocity = f64<meter / second>;

fnc velocity(distance: f64<meter>, duration: f64<second>) -> Velocity
    requires duration > 0
    ensures result >= 0 when distance >= 0
{
    return distance / duration;
}
```

```text
type PositiveInt = i32 where self > 0;
fnc identity<parametric T>(x: T) -> T pure { return x; }
```


## Proposed first-class language declarations (design direction)

The language should support **`define dimension`**, **`define unit`**, and **`define protocol`** as native declarations, not library-only conventions. Grammar, namespace placement and lowering remain subject to review.

```atto
// Illustrative syntax, NOT yet accepted by any compiler.
define dimension Time;
define dimension Length;

namespace time {
    define unit Second: Time;
    define unit Millis: Time = Second / 1000;
    define unit Micros: Time = Second / 1000000;
}

namespace length {
    define unit Meter: Length;
    define unit Kilometer: Length = 1000 * Meter;
}

type PositiveMillis = i32
    where self > 0
    and units in time.Millis;

type Duration = i64
    where self >= 0
    and units in time;

type Velocity = f64
    where units in length.Meter / time.Second;

define protocol Payment as
    Start -> Authorized | Rejected;
    Authorized -> Captured | Cancelled;
end
```

### Dimension and unit semantics

- `dimension` defines a nominal base dimension; composite dimensions form a normalized exponent vector (supporting rational exponents where representable).
- `unit` belongs to a dimension and defines an exact scale relative to a canonical unit; cycles, conflicting definitions and dimensional mismatches are compile errors.
- `units in time.Millis` constrains a **specific unit**; `units in time` constrains a **dimension**, with the concrete scale retained or made explicit at each use. It must not permit implicit mixing of seconds and milliseconds.
- Refinement predicates (`where self > 0`) and unit predicates (`and units in ...`) are independent, composable compile-time constraints. `where`/ `and units in` are proposed syntax, not committed grammar.
- Addition/subtraction require compatible quantities with defined conversion rules; multiplication/division derive resulting dimensions. A dimensionless value divided by milliseconds has dimension `Time^-1`, **not** dimensionless `i32`.
- Compile-time unit annotations are erased by default. Optional runtime quantity reflection/conversion may be added later without changing the static semantics.
- Integer conversion must not silently truncate, overflow or change the meaning of refinement predicates; provide explicit checked/fallible conversions and rounding policies.
- Offset/affine quantities (e.g. Celsius) are distinct from multiplicative quantities; floating-point proofs must model rounding and NaN/Infinity explicitly.

### Protocol semantics

- `define protocol` declares a nominal, type-checkable state-transition contract for actors and message channels; it is **not** equivalent to an interface/trait.
- Define initial/terminal states, legal transitions, message payload types, ownership transfer, cancellation/failure and supervisor behavior.
- Static checking enforces legal local transitions. Global deadlock freedom, liveness and fairness require separately stated assumptions and optional model checking.
- Protocol names, states and transition arms must resolve consistently through namespacing and formatter round trips.

### Illustrative dimensionally valid division

```atto
type PositiveMillis = i32 where self > 0 and units in time.Millis;

fnc divide(a: PositiveMillis, b: PositiveMillis) -> i32 {
    return a / b; // dimensionless quotient; integer division semantics apply
}
```

### Follow-up acceptance fixtures

1. Accept valid unit declarations, namespace-qualified references, composite dimensions and same-unit division.
2. Reject `meter + second`, implicit mixed-scale arithmetic, incompatible unit aliases, declaration cycles and invalid protocol transitions.
3. Check unit conversion precision/overflow, zero denominators, refinement preservation and signed integer division edge cases.
4. Parse/format round-trip `define dimension`, `define unit`, `define protocol`, and combined `where ... and units in ...`.
5. Compare Rust/LLVM/Java diagnostics and behavior against the same semantics while allowing independent backend implementation.


## Worked examples: dimensions and protocols

All examples below are **illustrative proposed syntax**. They are not executable conformance fixtures until the grammar and semantics are approved.

### 1. Declaring dimensions and units

```atto
define dimension Time;
define dimension Length;

define module time as
    define unit Second: Time;
    define unit Millis: Time = Second / 1000;
    define unit Micros: Time = Second / 1000000;
end

define module length as
    define unit Meter: Length;
    define unit Kilometer: Length = 1000 * Meter;
end

type PositiveMillis = i32
    where self > 0
    and units in time.Millis;

type Distance = f64
    where self >= 0
    and units in length.Meter;

type Velocity = f64
    where units in length.Meter / time.Second;
```

Literal notation `500<time.Millis>` is **proposed**, not accepted grammar. Under the proposal, these checks would apply:

```atto
let timeout: PositiveMillis = 500<time.Millis>; // valid
let negative: PositiveMillis = -10<time.Millis>; // reject: refinement
let wrong: PositiveMillis = 5<length.Meter>;    // reject: dimension
```

### 2. Dimension-preserving functions

```atto
fnc speed(
    distance: Distance,
    duration: f64 where self > 0
        and units in time.Second
) -> Velocity {
    return distance / duration;
}
```

The inferred result dimension is Length / Time. Conversion across scales must be explicit or justified by a defined, lossless conversion policy; integer rounding and overflow cannot be silently ignored.

### 3. Protocol-checked actor transitions

```atto
define protocol Payment as
    initial Pending;

    Pending -> Authorized | Rejected;
    Authorized -> Captured | Cancelled;

    terminal Captured;
    terminal Cancelled;
    terminal Rejected;
end

define actor PaymentProcessor implements Payment as
    authorize() {
        transition Authorized;
    }

    capture() {
        transition Captured;
    }
end
```

A transition to `Captured` is valid only from `Authorized`. The compiler must ensure handlers are appropriately state-guarded or generate runtime protocol checks for messages arriving in different orders; merely declaring the protocol does not establish that the actor implementation is correct. A verified implementation should also specify the behavior of invalid transitions, concurrency, failures and supervision.

### 4. Protocol transitions with dimensionally constrained payloads

```atto
type Timeout = i32
    where self > 0
    and self <= 60_000
    and units in time.Millis;

define protocol Connection as
    initial Disconnected;

    Disconnected -> Connecting(timeout: Timeout);
    Connecting -> Connected | Failed;
    Connected -> Disconnected;

    terminal Failed;
end
```

A `Connecting` transition must carry a valid timeout. The checker must independently verify the payload's refinement and units and the legality of the transition.

### Required positive/negative fixtures

- Accept `PositiveMillis` initialized with a positive millisecond quantity; reject negative, zero and wrong-dimension literals.
- Infer `Length / Time` for `speed`; reject returning the result as an unrelated dimension.
- Accept `Pending -> Authorized -> Captured`; reject `Pending -> Captured` and transitions from terminal states.
- Reject a connection timeout over 60,000 ms, nonpositive values, and a length quantity passed as timeout.
- Ensure compiler diagnostics distinguish unit mismatch, refinement failure and protocol-state violation.


## Native formal-methods syntax: phased proposal

The following is a **language design proposal**, not a claim of current parser or solver support. Formal verification should be endemic to the typed HIR/borrow/effect system, rather than implemented solely as external annotations.

### P0: declarations, contracts, and basic verification

| Construct | Intended meaning |
| --- | --- |
| `define dimension`, `define unit` | Dimension algebra and unit scales |
| `define protocol` | Nominal actor/channel state-transition contract |
| `where` / contextual `units in` | Refinement and dimension constraints |
| `requires` | Caller obligation / function precondition |
| `ensures` | Callee obligation / function postcondition |
| `invariant` | Loop, type or actor state preservation |
| `pure` | Effect-constrained, observationally pure function |
| `assert` | Check or prove an obligation, depending on verification mode |
| contextual `transition` | Protocol state transition, validated against the protocol |

### P1: explicit proofs and trust accounting

| Construct | Intended meaning |
| --- | --- |
| `proof` | Proof-only statement/expression context |
| `ghost` | Erasable specification state with noninterference checks |
| `decreases` | Well-founded termination measure |
| `old(expr)` | Snapshot of an allowed pre-state expression |
| `forall`, `exists` | Typed logical quantification |
| `assume` | Explicit trusted assumption; always audited |
| `lemma` | Reusable proof function/theorem |
| `spec` | Specification-only function |
| `reads`, `modifies` | Frame conditions governing effects/state access |
| `trusted` | Explicit trusted boundary, never a proof claim |

### P2: advanced reasoning

| Construct | Intended meaning |
| --- | --- |
| `temporal` | Temporal property context |
| `always`, `eventually` | Safety/liveness operators with specified trace/fairness model |
| `parametric` | Representation-independent polymorphism with restrictions on effects, reflection and divergence |

**Keyword policy:** reserve globally only where unambiguous parsing requires it. Prefer contextual keywords in declarations/contracts and library-level proof combinators over a large globally reserved vocabulary. Reuse existing Atto syntax where compatible. In particular, `old`, `reads`, `modifies`, `transition`, `units`, `temporal`, and `parametric` can initially be contextual. Parser design must check existing identifiers and compatibility before reserving any word.

### Illustrative contracts and dimension checks

```atto
type PositiveMillis = i32
    where self > 0
    and units in time.Millis;

fnc halve(duration: PositiveMillis) -> PositiveMillis
    requires duration >= 2<time.Millis>
    ensures result > 0<time.Millis>
    ensures result <= duration
    pure
{
    return duration / 2;
}
```

The type checker must model integer division, overflow and exact unit scaling. A declaration alone does not prove an implementation; the postconditions are verification obligations.

### Illustrative proofs and explicit assumptions

```atto
lemma positive_sum(x: i32, y: i32)
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

```atto
proof {
    assert(x > 0);  // Must be proved, or fail in strict mode.
    assume(y > 0);  // Creates an explicit, reviewable trust obligation.
}
```

Unrestricted `assume(false)` would make arbitrary propositions derivable. Every assumption and `trusted` boundary must be recorded in a machine-readable proof manifest, attributed to a source location, and rejected by strict mode unless explicitly approved. Unknown/timeout is never `proven`.

### Illustrative actor invariant, frame condition and protocol

```atto
define protocol Connection as
    initial Disconnected;
    Disconnected -> Connecting;
    Connecting -> Connected | Failed;
    Connected -> Disconnected;
    terminal Failed;
end

define actor Connector implements Connection as
    invariant retries >= 0;

    connect(timeout: PositiveMillis)
        requires timeout <= 60_000<time.Millis>
        modifies state
    {
        // Implementation must establish a legal transition.
        transition Connecting;
    }
end
```

Protocol transitions require a proof of the source state, correct message ownership, and preservation of actor invariants at observable boundaries. Concurrency, cancellation and supervision must be modeled explicitly; local type checking does not by itself prove global liveness or deadlock freedom.

### Illustrative advanced properties

```atto
fnc identity<parametric T>(x: T) -> T pure {
    return x;
}

temporal Connection {
    always(no_use_after_close);
    // eventually(Connected) requires explicit progress/fairness assumptions.
}
```

Parametricity free theorems require a sound semantic fragment (effects, divergence, reflection, unsafe and identity observations constrained). Temporal `eventually` cannot be guaranteed without a progress model.

### Acceptance and implementation gates

1. Produce a grammar/keyword collision audit and formatter round-trip cases for each proposed construct.
2. Define typed HIR nodes, effect/borrow interactions and verification IR obligations; do not implement verification as a text-only macro.
3. Test both valid and invalid contracts, refinements, quantifiers, termination, frame conditions, protocol transitions and unit arithmetic.
4. Verify that ghost/spec code cannot leak into observable runtime behavior and that proof erasure preserves executable semantics.
5. Check that `assume`/`trusted` and solver `unknown` cannot silently satisfy strict builds; emit complete proof manifests.
6. Verify compiler-core integration and semantic parity in Rust, LLVM and Java with executable positive/negative fixtures. Backend-specific implementations may differ.
7. Distinguish source-level proof from verified generated code; document compiler/runtime trusted computing base and translation-validation requirements.

## Architecture and parity

- Specify semantics independently of any particular backend: arithmetic, effects, ownership, actors, errors and unit conversion.
- Integrate borrow/effect checking into each compiler's core typed representation, not only into tests or external linters.
- Backends may have independent implementations and independently maintained `.atto` core/alloc/std libraries; require semantic conformance, not byte-identical sources.
- A verified source-level property is **not** automatically a verified executable property. Document trusted runtime/backend assumptions, add translation validation where feasible, and never overstate machine-code guarantees.
- Preserve snake_case function names and existing ownership-bang direction; do not introduce conflicting surface syntax without language-spec approval.

## Phased work

1. Agree on shared formal semantics, verification status schema and negative/positive conformance fixtures.
2. Implement units/dimension algebra and compile-fail diagnostics in the compiler core.
3. Add contract AST/HIR and proof-obligation generation; start with decidable arithmetic cases.
4. Add proof/ghost constructs, termination discipline, and explicit trusted assumptions.
5. Extend to actor invariants and protocol state, then relational parametricity.
6. Run equivalent acceptance and rejection cases across Rust, LLVM, Java and other Atto backends; require executable CI before any implementation claims.

## Acceptance criteria for follow-up implementation PRs

- Parse/format round-trip; positive and negative compile fixtures; deterministic diagnostics.
- Unit incompatibility, invalid refinement, arithmetic overflow and borrow-across-suspension cases rejected as appropriate.
- SMT `unknown` and `assumed` statuses cannot pass strict mode.
- Proof manifests enumerate solver/version, semantics profile, assumptions and backend trust boundary.
- CI verifies real compiler execution; if source-org Actions cannot run, use an authorized token-free, exact-SHA mirrored tree on a private alternate runner, with per-blob verification.

## Open design questions

- Should units use generic numeric type parameters or a dedicated dimension-kind syntax?
- What effect/termination fragment supports sound relational parametricity?
- How are affine units, floating-point rounding, FFI, unsafe and dynamic actors modeled?
- Which proof obligations are mandatory in ordinary builds versus strict mode?
- What is the smallest stable verification IR shared semantically across independent compiler implementations?

**Non-goals for this RFC:** claiming existing proof support, adding an SMT dependency now, modifying runtime behavior, or marking any compiler backend formally verified.


## Generics versus relational parametricity

**Design proposal, not implemented syntax.** Generics are Atto's general abstraction and specialization mechanism; relational parametricity is a **stronger semantic guarantee** attached to selected generic parameters. Do not conflate type correctness with representation independence.

### Core distinction

- Ordinary `<T>` permits the operations supported by its declared bounds, including type-dependent trait/interface behavior, allowed effects and specialization.
- `<parametric T>` proposes a relational uniformity obligation: the implementation must preserve all admissible relations between interpretations of `T` under the chosen semantic model. Reflection, concrete-type dispatch, representation tests, layout/size observations, unchecked casts, unsafe code and specialization that changes observable behavior must be disallowed or explicitly justified by a verified relational specification.
- Parametricity is not synonymous with purity. Effects require an effect-aware relational interpretation; a pure, total fragment is the initial proof target.
- `parametric` is proposed as a **contextual modifier of an existing generic parameter**, not a separate generic declaration system or necessarily a globally reserved keyword.
- A function declared `parametric` must not be automatically marked `proven`; the checker must establish the relational obligations or return `unknown`/`refuted`.

### Examples

```atto
// Ordinary generic: Display may depend on the concrete type.
fnc describe<T>(value: T) -> String
    where T: Display
{
    return value.to_string();
}

// Stronger proposed contract: no observations of the representation of T.
fnc identity<parametric T>(value: T) -> T
    pure
{
    return value;
}
```

For a **pure and total** `identity<parametric T>(T) -> T`, relational parametricity implies the result is its input (subject to the exact language semantics). In the presence of divergence, bottom values, effects or unsafe observations, the theorem needs corresponding qualifications.

```atto
fnc reverse<parametric T>(values: List<T>) -> List<T>
    pure
    ensures result.length == values.length
{
    // implementation
}
```

Parametricity alone does **not** prove preservation of length, order or contents. The `ensures` clause creates a separate implementation proof obligation.

```atto
fnc map<parametric T, parametric R>(
    values: List<T>,
    transform: fnc(T) -> R
) -> List<R>
    pure
{
    // implementation
}
```

Here `map` is relationally uniform **relative to its callback**. Its proof must quantify over callbacks that preserve the selected input/output relations. A concrete callback may be type-specific; it does not imply arbitrary callbacks satisfy all relational interpretations. The `pure` annotation requires a pure callback or an appropriate effect-constrained function type.

### Generic parameter kinds (proposed)

| Kind | Example | Meaning |
| --- | --- | --- |
| Type | `<T>` | Existing general type generic |
| Constant | `<const N: usize>` | Compile-time numeric/value parameter |
| Dimension | `<dimension D>` | Dimension-polymorphic operations |
| Unit | `<unit U>` | Unit-polymorphic operations and derived unit expressions |
| Protocol | `<protocol P>` | Protocol-polymorphic actor/channel contracts |
| Lifetime | syntax TBD | Borrow-region relationships |
| Effect | `<effect E>` | Effect-polymorphic operations |

These are **parameter kinds**, not proof that each requires a new globally reserved keyword. Preserve compatibility with existing generic defaults, named arguments, inference and specialization rules. `unit U` and `dimension D` require distinct kinds and dimension-consistent algebra.

```atto
fnc square<unit U>(
    x: f64 where units in U
) -> f64 where units in U * U
    pure
{
    return x * x;
}
```

The compiler must check unit expression kinds, scale factors, floating-point behavior and conversions. A type parameter `T` must not be silently confused with a unit or dimension parameter.

### Ownership, effects, actors and backends

```atto
fnc send_value<parametric T>(
    actor: Actor<T>,
    value: T
) -> Result<(), SendError>
{
    return actor.send(take! value);
}
```

This is **not automatically verified**. A relational interpretation of actor effects must account for ownership transfer, mailbox ordering, failures, cancellation, scheduler observations and error behavior. `take!`, `borrow!`, and `share!` remain governed by the borrow checker; their use does not itself establish parametricity. The actor example is illustrative and requires a defined `Actor<T>` API/effect contract.

Backend monomorphization or specialization is permitted only when it preserves the observable relational theorem. Proofs must not rely on unverified assumptions about compiler lowering; record the trusted computing base and any translation-validation requirement.

### Proof obligations and negative tests

1. Accept ordinary `<T>` generics with bounded type-specific operations; reject such observations under `<parametric T>` without a sound relational rule.
2. Verify pure total identity; reject or qualify unsound free-theorem claims involving divergence, effects, reflection, unsafe code or representation-sensitive primitives.
3. Prove `reverse` length preservation from implementation/contracts, not parametricity alone; reject a false postcondition.
4. Verify `map` only under a callback relation/effect contract; reject an unconstrained callback when the theorem requires relational preservation.
5. Type-check `<unit U>`, `<dimension D>` and derived units; reject kind mismatches, invalid scaling and unsafe conversions.
6. Check ownership transfers and effect/actor boundaries in relational verification; do not claim actor liveness or determinism from genericity.
7. Test parser/formatter round trips, named/default generic arguments, and Rust/LLVM/Java semantic parity. Emit `proven`, `refuted`, `unknown` or `assumed` status without upgrading unknown to proven.

