//! Compiler-owned automatic prelude catalog for the bounded browser backend.
//!
//! This is a *compile-time symbol scope*, never a runtime global. Listing a
//! public name here must not imply that browser/Wasm lowering exists. Frontend
//! type admission and guest code generation remain fail-closed.
//!
//! Implementations of core/alloc/std can differ across Atto backends; these
//! names and canonical destinations are the shared source-level contract.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    Core,
    Alloc,
    /// Not imported automatically; requires a qualified/explicit import.
    Explicit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Type,
    Constructor,
    Trait,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Core,
    Alloc,
    Hosted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub name: &'static str,
    pub canonical: &'static str,
    pub kind: Kind,
    pub scope: Scope,
}

const fn binding(
    name: &'static str,
    canonical: &'static str,
    kind: Kind,
    scope: Scope,
) -> Binding {
    Binding { name, canonical, kind, scope }
}

/// Immutable catalog: symbol visibility only, not an executable runtime ABI.
/// Sized primitive types are the compiler's syntax/builtin table, not imports.
/// Browser `string`/`String` currently use immutable module-data views; an
/// allocator-owned `String` must not be claimed as executable on that basis.
pub const BINDINGS: &[Binding] = &[
    binding("Option", "core/option/Option", Kind::Type, Scope::Core),
    binding("Some", "core/option/Some", Kind::Constructor, Scope::Core),
    binding("None", "core/option/None", Kind::Constructor, Scope::Core),
    binding("Result", "core/result/Result", Kind::Type, Scope::Core),
    binding("Ok", "core/result/Ok", Kind::Constructor, Scope::Core),
    binding("Err", "core/result/Err", Kind::Constructor, Scope::Core),
    binding("Error", "core/error/Error", Kind::Type, Scope::Core),
    binding("Array", "core/array/Array", Kind::Type, Scope::Core),
    binding("FixedArray", "core/array/FixedArray", Kind::Type, Scope::Core),
    binding("Slice", "core/slice/Slice", Kind::Type, Scope::Core),
    binding("Tuple", "core/tuple/Tuple", Kind::Type, Scope::Core),
    binding("Pair", "core/tuple/Pair", Kind::Type, Scope::Core),
    binding("Iterator", "core/iter/Iterator", Kind::Trait, Scope::Core),
    binding("Range", "core/range/Range", Kind::Type, Scope::Core),
    binding("Eq", "core/cmp/Eq", Kind::Trait, Scope::Core),
    binding("Ord", "core/cmp/Ord", Kind::Trait, Scope::Core),
    binding("Ordering", "core/cmp/Ordering", Kind::Type, Scope::Core),
    binding("From", "core/convert/From", Kind::Trait, Scope::Core),
    binding("Into", "core/convert/Into", Kind::Trait, Scope::Core),
    binding("TryFrom", "core/convert/TryFrom", Kind::Trait, Scope::Core),
    binding("TryInto", "core/convert/TryInto", Kind::Trait, Scope::Core),
    binding("Display", "core/fmt/Display", Kind::Trait, Scope::Core),
    binding("Debug", "core/fmt/Debug", Kind::Trait, Scope::Core),
    binding("Sendable", "core/marker/Sendable", Kind::Trait, Scope::Core),
    binding("String", "alloc/string/String", Kind::Type, Scope::Alloc),
    binding("List", "alloc/list/List", Kind::Type, Scope::Alloc),
    binding("Vec", "alloc/vec/Vec", Kind::Type, Scope::Alloc),
    binding("Map", "alloc/map/Map", Kind::Type, Scope::Alloc),
    binding("Set", "alloc/set/Set", Kind::Type, Scope::Alloc),
    binding("Box", "alloc/box/Box", Kind::Type, Scope::Alloc),
    binding("HashMap", "alloc/hash_map/HashMap", Kind::Type, Scope::Explicit),
    binding("HashSet", "alloc/hash_set/HashSet", Kind::Type, Scope::Explicit),
    binding("Entry", "alloc/map/Entry", Kind::Type, Scope::Explicit),
];

/// Lookup a prelude name in the specified source profile. `None` means that
/// the name must be brought into scope by an explicit import (or is unknown).
pub fn resolve(name: &str, profile: Profile) -> Option<&'static Binding> {
    BINDINGS.iter().find(|b| {
        b.name == name
            && match b.scope {
                Scope::Core => true,
                Scope::Alloc => profile != Profile::Core,
                Scope::Explicit => false,
            }
    })
}

/// Compiler-owned ADT/collection/loan identities cannot be spoofed by user
/// declarations, regardless of a file's lexical import order. Additional
/// convenience prelude names are intentionally not all globally reserved.
pub fn protected_identity(name: &str) -> bool {
    matches!(
        name,
        "Option" | "Result" | "Array" | "FixedArray" | "List"
            | "FixedList" | "Vector" | "Slice" | "Tuple"
            | "Map" | "Set" | "ReadLoan" | "WriteLoan"
            | "Some" | "None" | "Ok" | "Err"
    )
}

/// A scoped prelude name recognized by source semantics but not necessarily
/// compiled by this backend. The frontend must still check actual lowering.
pub fn recognized_type(name: &str) -> bool {
    BINDINGS.iter().any(|b| b.name == name && b.kind != Kind::Constructor)
}

/// The core ADT variants are implicitly visible in every source file.
/// Typechecking still validates their argument count and contextual ADT type.
pub fn core_constructor(name: &str) -> bool {
    resolve(name, Profile::Core).is_some_and(|b| b.kind == Kind::Constructor)
}

/// Safety category of a *resolved* canonical prelude name. This metadata
/// never independently grants a loan, allocation, move or Sendable proof.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ownership {
    /// ADT/aggregate copyability and sendability depend on payload members.
    PayloadDependent,
    /// A view; it cannot outlive the source owner.
    BorrowedView,
    /// Storage demands an admitted allocator/owned heap contract.
    Owned,
    /// Static contract rather than an owned runtime value.
    Trait,
}

pub fn ownership(name: &str) -> Option<Ownership> {
    let entry = BINDINGS.iter().find(|b| b.name == name)?;
    Some(match entry.name {
        "Slice" => Ownership::BorrowedView,
        "String" | "List" | "Vec" | "Map" | "Set" | "Box"
        | "HashMap" | "HashSet" | "Error" => Ownership::Owned,
        _ if entry.kind == Kind::Trait => Ownership::Trait,
        _ => Ownership::PayloadDependent,
    })
}

/// Only the supported Option<T>/Result<T,E> carrier arities may participate in
/// core ADT payload-dependent ownership rules. Never treat any user type that
/// merely resembles a sum as a license to extract an owned borrowed payload.
pub fn core_sum_carrier(name: &str, arity: usize) -> bool {
    (name == "Option" && arity == 1) || (name == "Result" && arity == 2)
}
