//! Compiler-owned static marker/brand proof preparation. No guest ABI.
//! Resolver-issued module/declaration/type IDs are trusted compiler inputs,
//! not guest Symbol values, wire metadata, or process-global identities.
//! Only flat, checked declarations are admitted; inheritance, delegated
//! brands and runtime reification need separate proofs.
use std::collections::{BTreeMap, BTreeSet};
use crate::structural_member_witness::{
    prove_structural_members, MemberError, RequiredMembers, SourceMembers,
    StructuralMemberWitness,
};

const MAX_TAGS: usize = 64;
const MAX_DECLARATIONS: usize = 4096;
const MAX_TYPES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ModuleId {
    package: u64,
    module: u64,
}

impl ModuleId {
    pub fn new(package: u64, module: u64) -> Result<Self, TagError> {
        if package == 0 || module == 0 {
            return Err(TagError::InvalidIdentity);
        }
        Ok(Self { package, module })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Scope {
    tenant: u64,
    epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TagKind {
    Marker,
    Brand,
}

/// Opaque typed declaration identity within a compiler resolver epoch.
/// Not a persistent UUID or a capability token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TagId {
    scope: Scope,
    owner: ModuleId,
    declaration: u64,
    kind: TagKind,
}

impl TagId {
    pub fn kind(self) -> TagKind { self.kind }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility { Private, Public }

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagError {
    InvalidIdentity,
    Budget,
    DuplicateDeclaration,
    DuplicateType,
    DuplicateTag,
    UnknownTag,
    WrongScope,
    WrongKind,
    InvisibleTag,
    UnauthorizedBrand,
    UnknownType,
    StaleLayout,
    MissingMarker,
    MissingBrand,
    Members(MemberError),
}

#[derive(Debug)]
struct RegisteredType {
    layout_epoch: u64,
    tags: BTreeSet<TagId>,
}

/// One compiler-owned registry per unique tenant/resolver epoch. The caller
/// must obtain these IDs from the checked package resolver and must never
/// reuse an epoch for a different registry. No serialization or guest access.
pub struct TagRegistry {
    scope: Scope,
    declarations: BTreeMap<(ModuleId, u64), (TagKind, Visibility)>,
    types: BTreeMap<u32, RegisteredType>,
}

#[derive(Clone, Copy, Debug)]
pub enum TagTarget<'a> {
    /// Resolved target declaration; all its tags are automatically required.
    Named { nominal_id: u32, layout_epoch: u64, additional: &'a [TagId] },
    /// Shape-only unless explicitly constrained.
    Anonymous { required: &'a [TagId] },
}

/// Retains only the proven required tags, never the source's erased tags.
/// Existing member proof retains the actual source layout and loan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaggedMemberWitness {
    scope: Scope,
    required_tags: Vec<TagId>,
    members: StructuralMemberWitness,
}

impl TaggedMemberWitness {
    pub fn required_tags(&self) -> &[TagId] { &self.required_tags }
    pub fn members(&self) -> &StructuralMemberWitness { &self.members }
}

impl TagRegistry {
    pub fn new(tenant: u64, resolver_epoch: u64) -> Result<Self, TagError> {
        if tenant == 0 || resolver_epoch == 0 {
            return Err(TagError::InvalidIdentity);
        }
        Ok(Self {
            scope: Scope { tenant, epoch: resolver_epoch },
            declarations: BTreeMap::new(),
            types: BTreeMap::new(),
        })
    }

    pub fn declare(
        &mut self, owner: ModuleId, declaration: u64, kind: TagKind,
        visibility: Visibility,
    ) -> Result<TagId, TagError> {
        if declaration == 0 { return Err(TagError::InvalidIdentity); }
        if self.declarations.contains_key(&(owner, declaration)) {
            return Err(TagError::DuplicateDeclaration);
        }
        if self.declarations.len() >= MAX_DECLARATIONS { return Err(TagError::Budget); }
        self.declarations.insert((owner, declaration), (kind, visibility));
        Ok(TagId { scope: self.scope, owner, declaration, kind })
    }

    fn visibility(&self, id: TagId) -> Result<Visibility, TagError> {
        if id.scope != self.scope { return Err(TagError::WrongScope); }
        let (kind, visibility) = self.declarations.get(&(id.owner, id.declaration))
            .ok_or(TagError::UnknownTag)?;
        if *kind != id.kind { return Err(TagError::WrongKind); }
        Ok(*visibility)
    }

    /// Resolving a public brand is not authorization to attach it.
    pub fn resolve(
        &self, requester: ModuleId, id: TagId, expected: TagKind,
    ) -> Result<TagId, TagError> {
        let visibility = self.visibility(id)?;
        if id.kind != expected { return Err(TagError::WrongKind); }
        if visibility == Visibility::Private && requester != id.owner {
            return Err(TagError::InvisibleTag);
        }
        Ok(id)
    }

    fn check_set(
        &self, requester: ModuleId, ids: &[TagId], attachment: bool,
    ) -> Result<BTreeSet<TagId>, TagError> {
        if ids.len() > MAX_TAGS { return Err(TagError::Budget); }
        let mut tags = BTreeSet::new();
        for &id in ids {
            self.resolve(requester, id, id.kind)?;
            if attachment && id.kind == TagKind::Brand && requester != id.owner {
                return Err(TagError::UnauthorizedBrand);
            }
            if !tags.insert(id) { return Err(TagError::DuplicateTag); }
        }
        Ok(tags)
    }

    /// The caller certifies this is a flat checked nominal declaration,
    /// including construction and mutation checks where needed. An empty tag
    /// set still requires a registered source type; missing metadata is not
    /// silently treated as an unbranded type.
    pub fn register_flat_type(
        &mut self, nominal_id: u32, layout_epoch: u64,
        owner: ModuleId, ids: &[TagId],
    ) -> Result<(), TagError> {
        if nominal_id == 0 || layout_epoch == 0 {
            return Err(TagError::InvalidIdentity);
        }
        if self.types.contains_key(&nominal_id) { return Err(TagError::DuplicateType); }
        if self.types.len() >= MAX_TYPES { return Err(TagError::Budget); }
        let tags = self.check_set(owner, ids, true)?;
        self.types.insert(nominal_id, RegisteredType { layout_epoch, tags });
        Ok(())
    }

    fn checked_type(&self, nominal_id: u32, epoch: u64) -> Result<&RegisteredType, TagError> {
        let entry = self.types.get(&nominal_id).ok_or(TagError::UnknownType)?;
        if entry.layout_epoch != epoch { return Err(TagError::StaleLayout); }
        Ok(entry)
    }

    /// The caller binds the required member descriptor to the same checked
    /// target declaration used here. Never call the legacy untagged planner
    /// directly for a named/tagged target. This is non-emitting proof only.
    pub fn prove(
        &self, requester: ModuleId, source: &SourceMembers,
        required: &RequiredMembers, target: TagTarget<'_>,
    ) -> Result<TaggedMemberWitness, TagError> {
        if source.tenant_vm_id != self.scope.tenant
            || required.tenant_vm_id != self.scope.tenant {
            return Err(TagError::WrongScope);
        }
        let actual = self.checked_type(source.shape.source_nominal_id, source.shape.layout_epoch)?;
        let required_tags = match target {
            TagTarget::Named { nominal_id, layout_epoch, additional } => {
                let mut tags = self.checked_type(nominal_id, layout_epoch)?.tags.clone();
                tags.extend(self.check_set(requester, additional, false)?);
                tags
            }
            TagTarget::Anonymous { required } => self.check_set(requester, required, false)?,
        };
        if required_tags.len() > MAX_TAGS { return Err(TagError::Budget); }
        for tag in &required_tags {
            if !actual.tags.contains(tag) {
                return Err(match tag.kind {
                    TagKind::Marker => TagError::MissingMarker,
                    TagKind::Brand => TagError::MissingBrand,
                });
            }
        }
        // Independent visibility, layout, effects, ownership and loan proofs.
        let members = prove_structural_members(source, required).map_err(TagError::Members)?;
        Ok(TaggedMemberWitness {
            scope: self.scope,
            required_tags: required_tags.into_iter().collect(),
            members,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape_witness::{RequiredField, ShapeTy, SourceField, SourceKind, SourceShape};
    use crate::structural_member_witness::RequiredKind;

    fn module(package: u64, module: u64) -> ModuleId {
        ModuleId::new(package, module).unwrap()
    }
    fn source(id: u32) -> SourceMembers {
        SourceMembers {
            tenant_vm_id: 7,
            shape: SourceShape {
                kind: SourceKind::Class, source_nominal_id: id, layout_epoch: 12,
                region_loan_id: 99, sealed_actor_state: false, map_snapshot_id: None,
                fields: vec![SourceField {
                    name: "value".into(), ty: ShapeTy::Int, public: true, readable: true,
                    guaranteed_present: true, effect_free: true, sealed_actor_state: false,
                    actual_offset: Some(16),
                }],
            },
            methods: vec![], tuple_elements: vec![], map_elements: None,
        }
    }
    fn required() -> RequiredMembers {
        RequiredMembers {
            kind: RequiredKind::Class, tenant_vm_id: 7,
            fields: vec![RequiredField { name: "value".into(), ty: ShapeTy::Int }],
            methods: vec![], tuple_elements: vec![], map_elements: None,
        }
    }
    fn named(id: u32) -> TagTarget<'static> {
        TagTarget::Named { nominal_id: id, layout_epoch: 12, additional: &[] }
    }

    #[test]
    fn same_brand_cross_nominal_and_actual_layout() {
        let mut r = TagRegistry::new(7, 1).unwrap();
        let owner = module(1, 1);
        let brand = r.declare(owner, 1, TagKind::Brand, Visibility::Public).unwrap();
        r.register_flat_type(1, 12, owner, &[brand]).unwrap();
        r.register_flat_type(2, 12, owner, &[brand]).unwrap();
        let proof = r.prove(owner, &source(2), &required(), named(1)).unwrap();
        assert_eq!(proof.required_tags(), &[brand]);
        assert_eq!(proof.members().source_nominal_id, 2);
        assert_eq!(proof.members().region_loan_id, 99);
        assert_eq!(proof.members().fields[0].access, crate::shape_witness::ReadAccess::SourceOffset(16));
    }

    #[test]
    fn same_payload_wrong_brand_or_unbranded_fails() {
        let mut r = TagRegistry::new(7, 1).unwrap();
        let m = module(1, 1);
        let a = r.declare(m, 1, TagKind::Brand, Visibility::Public).unwrap();
        let b = r.declare(m, 2, TagKind::Brand, Visibility::Public).unwrap();
        r.register_flat_type(1, 12, m, &[a]).unwrap();
        r.register_flat_type(2, 12, m, &[b]).unwrap();
        r.register_flat_type(3, 12, m, &[]).unwrap();
        for id in [2, 3] {
            assert_eq!(r.prove(m, &source(id), &required(), named(1)), Err(TagError::MissingBrand));
        }
    }

    #[test]
    fn anonymous_shape_erases_tag_evidence() {
        let mut r = TagRegistry::new(7, 1).unwrap();
        let m = module(1, 1);
        let tag = r.declare(m, 1, TagKind::Brand, Visibility::Private).unwrap();
        r.register_flat_type(1, 12, m, &[tag]).unwrap();
        let proof = r.prove(module(2, 1), &source(1), &required(),
            TagTarget::Anonymous { required: &[] }).unwrap();
        assert!(proof.required_tags().is_empty());
    }

    #[test]
    fn marker_constraints_are_conjunctive_and_sorted() {
        let mut r = TagRegistry::new(7, 1).unwrap();
        let m = module(1, 1);
        let a = r.declare(m, 1, TagKind::Marker, Visibility::Public).unwrap();
        let b = r.declare(m, 2, TagKind::Marker, Visibility::Public).unwrap();
        r.register_flat_type(1, 12, m, &[a, b]).unwrap();
        r.register_flat_type(2, 12, m, &[a]).unwrap();
        let t = TagTarget::Anonymous { required: &[b, a] };
        assert_eq!(r.prove(m, &source(2), &required(), t), Err(TagError::MissingMarker));
        assert_eq!(r.prove(m, &source(1), &required(), t).unwrap().required_tags(), &[a, b]);
    }

    #[test]
    fn public_marker_is_attachable_but_public_brand_is_not() {
        let mut r = TagRegistry::new(7, 1).unwrap();
        let m = module(1, 1);
        let other = module(1, 2);
        let a = r.declare(m, 1, TagKind::Marker, Visibility::Public).unwrap();
        let b = r.declare(m, 2, TagKind::Brand, Visibility::Public).unwrap();
        assert_eq!(r.resolve(other, b, TagKind::Brand), Ok(b));
        assert_eq!(r.register_flat_type(1, 12, other, &[a, b]), Err(TagError::UnauthorizedBrand));
        assert_eq!(r.register_flat_type(1, 12, other, &[a]), Ok(()));
    }

    #[test]
    fn unrelated_packages_distinct_and_wrong_kind_rejected() {
        let mut r = TagRegistry::new(7, 1).unwrap();
        let m = module(1, 1);
        let n = module(2, 1);
        let a = r.declare(m, 1, TagKind::Brand, Visibility::Public).unwrap();
        let b = r.declare(n, 1, TagKind::Brand, Visibility::Public).unwrap();
        assert_ne!(a, b);
        assert_eq!(r.resolve(n, a, TagKind::Marker), Err(TagError::WrongKind));
        r.register_flat_type(1, 12, m, &[a]).unwrap();
        r.register_flat_type(2, 12, n, &[b]).unwrap();
        assert_eq!(r.prove(n, &source(2), &required(), named(1)), Err(TagError::MissingBrand));
    }

    #[test]
    fn private_tags_duplicate_declarations_and_duplicate_constraints_fail() {
        let mut r = TagRegistry::new(7, 1).unwrap();
        let m = module(1, 1);
        let a = r.declare(m, 1, TagKind::Marker, Visibility::Private).unwrap();
        assert_eq!(r.declare(m, 1, TagKind::Brand, Visibility::Public), Err(TagError::DuplicateDeclaration));
        assert_eq!(r.resolve(module(2, 1), a, TagKind::Marker), Err(TagError::InvisibleTag));
        assert_eq!(r.register_flat_type(1, 12, m, &[a, a]), Err(TagError::DuplicateTag));
        r.register_flat_type(1, 12, m, &[a]).unwrap();
        assert_eq!(r.prove(m, &source(1), &required(), TagTarget::Anonymous { required: &[a, a] }),
            Err(TagError::DuplicateTag));
    }

    #[test]
    fn stale_scope_layout_missing_metadata_and_tenant_fail_closed() {
        let mut r = TagRegistry::new(7, 1).unwrap();
        let m = module(1, 1);
        let a = r.declare(m, 1, TagKind::Brand, Visibility::Public).unwrap();
        let mut other = TagRegistry::new(7, 2).unwrap();
        assert_eq!(other.register_flat_type(1, 12, m, &[a]), Err(TagError::WrongScope));
        r.register_flat_type(1, 12, m, &[a]).unwrap();
        assert_eq!(r.prove(m, &source(2), &required(), named(1)), Err(TagError::UnknownType));
        let mut s = source(1);
        s.shape.layout_epoch = 13;
        assert_eq!(r.prove(m, &s, &required(), named(1)), Err(TagError::StaleLayout));
        s.tenant_vm_id = 8;
        assert_eq!(r.prove(m, &s, &required(), named(1)), Err(TagError::WrongScope));
    }

    #[test]
    fn correct_tag_never_waives_member_or_loan_proofs() {
        let mut r = TagRegistry::new(7, 1).unwrap();
        let m = module(1, 1);
        let a = r.declare(m, 1, TagKind::Brand, Visibility::Public).unwrap();
        r.register_flat_type(1, 12, m, &[a]).unwrap();
        let mut s = source(1);
        s.shape.fields[0].public = false;
        assert!(matches!(r.prove(m, &s, &required(), named(1)), Err(TagError::Members(_))));
        s.shape.fields[0].public = true;
        s.shape.region_loan_id = 0;
        assert!(matches!(r.prove(m, &s, &required(), named(1)), Err(TagError::Members(_))));
        s.shape.region_loan_id = 99;
        s.shape.sealed_actor_state = true;
        assert!(matches!(r.prove(m, &s, &required(), named(1)), Err(TagError::Members(_))));
    }
}
