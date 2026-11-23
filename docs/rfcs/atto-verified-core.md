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
