# Compile-time scalar typeclasses: first executable slice

Status: **draft Rust/Wasm compiler feature**, not general typeclass support or cross-backend parity.

The following source-local declarations are lowered before the ordinary typed compiler:

    define typeclass Additive
        type Output;
        fnc add(Self, Self): Output;
    end
    define impl Additive for int
        type Output = int;
        bind add = add_int;
    end
    fnc add_int(int lhs, int rhs): int { return lhs + rhs; }
    pub fnc main(): int {
        val n: Additive.Output<int> = Additive.add<int>(20,22);
        return n;
    }

The compiler rewrites Additive.Output<int> to int and Additive.add<int>(...) to add_int(...) before the existing checker emits a **direct Wasm call**. No runtime witness, reflection, generic JIT, dynamic proxy, method/operator overload or witness message transport is introduced.

Admission: declarations must be top-level in the compiler's single linked token unit; an implementation may target only int/bool/float/string, associated types are scalar or Self, explicit specialization is mandatory, every method/associated type must bind exactly once, target functions must have matching scalar signatures, and duplicate implementations fail. Maximum 16 typeclasses, 64 implementations, 32 members each. The normal parser/checker remains the final authority on executable calls and types.

**Not yet implemented:** real module ownership/orphan checks across imports; nominal implementations; associated type constructors, recursive aliases or generic associated types; inferred typeclass bounds; effect declarations on typeclass methods; overload selection; cross-backend parity. Source-local ownership is stricter than the proposed module-aware model, but the import linker currently combines files before lowering. Do not treat linked-token-unit admission as proof of package ownership.

Before Ready: execute Rust unit tests, complete compiler library tests, Clippy, built attoc guest Wasm execution and negative no-artifact cases on the exact PR head. Zero-step Actions runs are infrastructure failures, not passes. See atto-spec/proposals/typeclass-witness-v1.md for the broader proposed contract.
**Static member syntax:** use `.` for both associated-type projections and typeclass method calls. The `::` spelling is invalid for these expressions, and the compiler rejects it; this is not a formatter alias.
