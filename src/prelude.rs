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
    TYPE_INFO.iter().any(|info| {
        info.name == name && (info.kind == Kind::Constructor || info.scope != Scope::Explicit)
    }) || matches!(name, "FixedList" | "Vector" | "ReadLoan" | "WriteLoan")
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

/// Ownership category is descriptive metadata only; actual move/loan authority
/// comes from the production checker and source call-site analysis.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ownership {
    Copy,
    PayloadDependent,
    BorrowedView,
    Owned,
    Trait,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Allocation { None, BorrowedView, AllocatorRequired }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropPolicy {
    None,
    ActivePayloadOnce,
    AllInitializedMembers,
    OwnedStorage,
    NotAValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendPolicy { Always, PayloadDependent, NeverBorrowed, ProofRequired, NotApplicable }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoweringStatus { WasmPrimitive, WasmScalarAdt, LegacyStringView, ContractOnly }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypeInfo {
    pub name: &'static str,
    pub canonical: &'static str,
    pub scope: Scope,
    pub kind: Kind,
    pub min_generic_arity: u8,
    pub max_generic_arity: u8,
    pub ownership: Ownership,
    pub allocation: Allocation,
    pub drop_policy: DropPolicy,
    pub send_policy: SendPolicy,
    pub lowering: LoweringStatus,
}

impl TypeInfo {
    pub fn accepts_arity(&self, arity: usize) -> bool {
        arity >= self.min_generic_arity as usize
            && arity <= self.max_generic_arity as usize
    }
}

const fn type_info(
    name: &'static str, canonical: &'static str, scope: Scope, kind: Kind,
    min: u8, max: u8, ownership: Ownership, allocation: Allocation,
    drop_policy: DropPolicy, send_policy: SendPolicy, lowering: LoweringStatus,
) -> TypeInfo {
    TypeInfo { name, canonical, scope, kind, min_generic_arity: min, max_generic_arity: max,
        ownership, allocation, drop_policy, send_policy, lowering }
}

/// Single immutable semantic metadata table for the automatic prelude.
/// CONTRACT_ONLY entries are discoverable for specification/conformance but
/// MUST still fail closed in this backend until real lowering exists.
pub const TYPE_INFO: &[TypeInfo] = &[
    type_info("int","core/primitive/i64",Scope::Core,Kind::Type,0,0,Ownership::Copy,Allocation::None,DropPolicy::None,SendPolicy::Always,LoweringStatus::WasmPrimitive),
    type_info("Int","core/primitive/i64",Scope::Core,Kind::Type,0,0,Ownership::Copy,Allocation::None,DropPolicy::None,SendPolicy::Always,LoweringStatus::ContractOnly),
    type_info("i64","core/primitive/i64",Scope::Core,Kind::Type,0,0,Ownership::Copy,Allocation::None,DropPolicy::None,SendPolicy::Always,LoweringStatus::WasmPrimitive),
    type_info("i8","core/primitive/i8",Scope::Core,Kind::Type,0,0,Ownership::Copy,Allocation::None,DropPolicy::None,SendPolicy::Always,LoweringStatus::ContractOnly),
    type_info("i16","core/primitive/i16",Scope::Core,Kind::Type,0,0,Ownership::Copy,Allocation::None,DropPolicy::None,SendPolicy::Always,LoweringStatus::ContractOnly),
    type_info("i32","core/primitive/i32",Scope::Core,Kind::Type,0,0,Ownership::Copy,Allocation::None,DropPolicy::None,SendPolicy::Always,LoweringStatus::ContractOnly),
    type_info("uint","core/primitive/u64",Scope::Core,Kind::Type,0,0,Ownership::Copy,Allocation::None,DropPolicy::None,SendPolicy::Always,LoweringStatus::ContractOnly),
    type_info("u8","core/primitive/u8",Scope::Core,Kind::Type,0,0,Ownership::Copy,Allocation::None,DropPolicy::None,SendPolicy::Always,LoweringStatus::ContractOnly),
    type_info("u16","core/primitive/u16",Scope::Core,Kind::Type,0,0,Ownership::Copy,Allocation::None,DropPolicy::None,SendPolicy::Always,LoweringStatus::ContractOnly),
    type_info("u32","core/primitive/u32",Scope::Core,Kind::Type,0,0,Ownership::Copy,Allocation::None,DropPolicy::None,SendPolicy::Always,LoweringStatus::ContractOnly),
    type_info("u64","core/primitive/u64",Scope::Core,Kind::Type,0,0,Ownership::Copy,Allocation::None,DropPolicy::None,SendPolicy::Always,LoweringStatus::ContractOnly),
    type_info("bool","core/primitive/bool",Scope::Core,Kind::Type,0,0,Ownership::Copy,Allocation::None,DropPolicy::None,SendPolicy::Always,LoweringStatus::WasmPrimitive),
    type_info("Bool","core/primitive/bool",Scope::Core,Kind::Type,0,0,Ownership::Copy,Allocation::None,DropPolicy::None,SendPolicy::Always,LoweringStatus::ContractOnly),
    type_info("float","core/primitive/f64",Scope::Core,Kind::Type,0,0,Ownership::Copy,Allocation::None,DropPolicy::None,SendPolicy::Always,LoweringStatus::ContractOnly),
    type_info("f32","core/primitive/f32",Scope::Core,Kind::Type,0,0,Ownership::Copy,Allocation::None,DropPolicy::None,SendPolicy::Always,LoweringStatus::ContractOnly),
    type_info("f64","core/primitive/f64",Scope::Core,Kind::Type,0,0,Ownership::Copy,Allocation::None,DropPolicy::None,SendPolicy::Always,LoweringStatus::ContractOnly),
    type_info("string","core/primitive/string-view",Scope::Core,Kind::Type,0,0,Ownership::BorrowedView,Allocation::BorrowedView,DropPolicy::None,SendPolicy::NeverBorrowed,LoweringStatus::LegacyStringView),
    type_info("Option","core/option/Option",Scope::Core,Kind::Type,1,1,Ownership::PayloadDependent,Allocation::None,DropPolicy::ActivePayloadOnce,SendPolicy::PayloadDependent,LoweringStatus::WasmScalarAdt),
    type_info("Some","core/option/Some",Scope::Core,Kind::Constructor,1,1,Ownership::PayloadDependent,Allocation::None,DropPolicy::ActivePayloadOnce,SendPolicy::PayloadDependent,LoweringStatus::WasmScalarAdt),
    type_info("None","core/option/None",Scope::Core,Kind::Constructor,0,0,Ownership::PayloadDependent,Allocation::None,DropPolicy::None,SendPolicy::PayloadDependent,LoweringStatus::WasmScalarAdt),
    type_info("Result","core/result/Result",Scope::Core,Kind::Type,2,2,Ownership::PayloadDependent,Allocation::None,DropPolicy::ActivePayloadOnce,SendPolicy::PayloadDependent,LoweringStatus::WasmScalarAdt),
    type_info("Ok","core/result/Ok",Scope::Core,Kind::Constructor,1,1,Ownership::PayloadDependent,Allocation::None,DropPolicy::ActivePayloadOnce,SendPolicy::PayloadDependent,LoweringStatus::WasmScalarAdt),
    type_info("Err","core/result/Err",Scope::Core,Kind::Constructor,1,1,Ownership::PayloadDependent,Allocation::None,DropPolicy::ActivePayloadOnce,SendPolicy::PayloadDependent,LoweringStatus::WasmScalarAdt),
    type_info("Error","core/error/Error",Scope::Core,Kind::Type,0,0,Ownership::Owned,Allocation::None,DropPolicy::OwnedStorage,SendPolicy::PayloadDependent,LoweringStatus::ContractOnly),
    type_info("Array","core/array/Array",Scope::Core,Kind::Type,1,2,Ownership::PayloadDependent,Allocation::None,DropPolicy::AllInitializedMembers,SendPolicy::PayloadDependent,LoweringStatus::ContractOnly),
    type_info("FixedArray","core/array/FixedArray",Scope::Core,Kind::Type,2,2,Ownership::PayloadDependent,Allocation::None,DropPolicy::AllInitializedMembers,SendPolicy::PayloadDependent,LoweringStatus::ContractOnly),
    type_info("Slice","core/slice/Slice",Scope::Core,Kind::Type,1,1,Ownership::BorrowedView,Allocation::BorrowedView,DropPolicy::None,SendPolicy::NeverBorrowed,LoweringStatus::ContractOnly),
    type_info("Tuple","core/tuple/Tuple",Scope::Core,Kind::Type,0,32,Ownership::PayloadDependent,Allocation::None,DropPolicy::AllInitializedMembers,SendPolicy::PayloadDependent,LoweringStatus::ContractOnly),
    type_info("Pair","core/tuple/Pair",Scope::Core,Kind::Type,2,2,Ownership::PayloadDependent,Allocation::None,DropPolicy::AllInitializedMembers,SendPolicy::PayloadDependent,LoweringStatus::ContractOnly),
    type_info("Iterator","core/iter/Iterator",Scope::Core,Kind::Trait,1,1,Ownership::Trait,Allocation::None,DropPolicy::NotAValue,SendPolicy::NotApplicable,LoweringStatus::ContractOnly),
    type_info("Range","core/range/Range",Scope::Core,Kind::Type,1,1,Ownership::PayloadDependent,Allocation::None,DropPolicy::None,SendPolicy::PayloadDependent,LoweringStatus::ContractOnly),
    type_info("Eq","core/cmp/Eq",Scope::Core,Kind::Trait,0,0,Ownership::Trait,Allocation::None,DropPolicy::NotAValue,SendPolicy::NotApplicable,LoweringStatus::ContractOnly),
    type_info("Ord","core/cmp/Ord",Scope::Core,Kind::Trait,0,0,Ownership::Trait,Allocation::None,DropPolicy::NotAValue,SendPolicy::NotApplicable,LoweringStatus::ContractOnly),
    type_info("Ordering","core/cmp/Ordering",Scope::Core,Kind::Type,0,0,Ownership::Copy,Allocation::None,DropPolicy::None,SendPolicy::Always,LoweringStatus::ContractOnly),
    type_info("From","core/convert/From",Scope::Core,Kind::Trait,1,1,Ownership::Trait,Allocation::None,DropPolicy::NotAValue,SendPolicy::NotApplicable,LoweringStatus::ContractOnly),
    type_info("Into","core/convert/Into",Scope::Core,Kind::Trait,1,1,Ownership::Trait,Allocation::None,DropPolicy::NotAValue,SendPolicy::NotApplicable,LoweringStatus::ContractOnly),
    type_info("TryFrom","core/convert/TryFrom",Scope::Core,Kind::Trait,1,1,Ownership::Trait,Allocation::None,DropPolicy::NotAValue,SendPolicy::NotApplicable,LoweringStatus::ContractOnly),
    type_info("TryInto","core/convert/TryInto",Scope::Core,Kind::Trait,1,1,Ownership::Trait,Allocation::None,DropPolicy::NotAValue,SendPolicy::NotApplicable,LoweringStatus::ContractOnly),
    type_info("Display","core/fmt/Display",Scope::Core,Kind::Trait,0,0,Ownership::Trait,Allocation::None,DropPolicy::NotAValue,SendPolicy::NotApplicable,LoweringStatus::ContractOnly),
    type_info("Debug","core/fmt/Debug",Scope::Core,Kind::Trait,0,0,Ownership::Trait,Allocation::None,DropPolicy::NotAValue,SendPolicy::NotApplicable,LoweringStatus::ContractOnly),
    type_info("Sendable","core/marker/Sendable",Scope::Core,Kind::Trait,0,0,Ownership::Trait,Allocation::None,DropPolicy::NotAValue,SendPolicy::NotApplicable,LoweringStatus::ContractOnly),
    type_info("String","alloc/string/String",Scope::Alloc,Kind::Type,0,0,Ownership::BorrowedView,Allocation::BorrowedView,DropPolicy::None,SendPolicy::NeverBorrowed,LoweringStatus::LegacyStringView),
    type_info("List","alloc/list/List",Scope::Alloc,Kind::Type,1,1,Ownership::Owned,Allocation::AllocatorRequired,DropPolicy::OwnedStorage,SendPolicy::PayloadDependent,LoweringStatus::ContractOnly),
    type_info("Vec","alloc/vec/Vec",Scope::Alloc,Kind::Type,1,1,Ownership::Owned,Allocation::AllocatorRequired,DropPolicy::OwnedStorage,SendPolicy::PayloadDependent,LoweringStatus::ContractOnly),
    type_info("Map","alloc/map/Map",Scope::Alloc,Kind::Type,2,2,Ownership::Owned,Allocation::AllocatorRequired,DropPolicy::OwnedStorage,SendPolicy::PayloadDependent,LoweringStatus::ContractOnly),
    type_info("Set","alloc/set/Set",Scope::Alloc,Kind::Type,1,1,Ownership::Owned,Allocation::AllocatorRequired,DropPolicy::OwnedStorage,SendPolicy::PayloadDependent,LoweringStatus::ContractOnly),
    type_info("Box","alloc/box/Box",Scope::Alloc,Kind::Type,1,1,Ownership::Owned,Allocation::AllocatorRequired,DropPolicy::OwnedStorage,SendPolicy::PayloadDependent,LoweringStatus::ContractOnly),
    type_info("HashMap","alloc/hash_map/HashMap",Scope::Explicit,Kind::Type,2,2,Ownership::Owned,Allocation::AllocatorRequired,DropPolicy::OwnedStorage,SendPolicy::PayloadDependent,LoweringStatus::ContractOnly),
    type_info("HashSet","alloc/hash_set/HashSet",Scope::Explicit,Kind::Type,1,1,Ownership::Owned,Allocation::AllocatorRequired,DropPolicy::OwnedStorage,SendPolicy::PayloadDependent,LoweringStatus::ContractOnly),
    type_info("Entry","alloc/map/Entry",Scope::Explicit,Kind::Type,2,2,Ownership::PayloadDependent,Allocation::None,DropPolicy::AllInitializedMembers,SendPolicy::PayloadDependent,LoweringStatus::ContractOnly),
];

pub fn type_info(name: &str) -> Option<&'static TypeInfo> {
    TYPE_INFO.iter().find(|entry| entry.name == name)
}

pub fn ownership(name: &str) -> Option<Ownership> {
    type_info(name).map(|entry| entry.ownership)
}

pub fn core_sum_carrier(name: &str, arity: usize) -> bool {
    type_info(name).is_some_and(|entry| {
        entry.scope == Scope::Core
            && entry.kind == Kind::Type
            && entry.ownership == Ownership::PayloadDependent
            && entry.accepts_arity(arity)
            && (name == "Option" || name == "Result")
    })
}
