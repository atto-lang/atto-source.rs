//! Namespace-qualified sealed-class link policy, not an executable class linker.
//!
//! Source-local lowering emits class graphs with an explicit local-only domain.
//! A future package linker must authenticate both the compiler-produced module
//! and the unit identity BEFORE invoking this verifier. No runtime class ABI,
//! imported method body, constructor layout, or hot-load authority is created
//! by passing a synthetic LinkClass here.
use std::collections::{BTreeMap, BTreeSet};

use crate::diag::{Diagnostic, Pos};
use crate::nominal_lower::{ClassHierarchy, ClassVisibility, Hierarchy};

type R<T> = Result<T, Diagnostic>;
const MAX_LINK_CLASSES: usize = 512;
const MAX_EDGES: usize = 96;
const MAX_ID_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct NominalId {
    pub package: String,
    pub namespace: String,
    pub name: String,
}
#[derive(Clone, Debug)]
pub struct LinkClass {
    pub id: NominalId,
    pub pos: Pos,
    pub visibility: ClassVisibility,
    pub hierarchy: Hierarchy,
    pub direct_parents: Vec<NominalId>,
    pub permitted_children: Vec<NominalId>,
}
fn error(pos: Pos, code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(pos, code, message)
}
fn valid_segment(segment: &str, allow_hyphen: bool) -> bool {
    let mut chars = segment.chars();
    let Some(first) = chars.next() else { return false; };
    if !first.is_alphabetic() && first != '_' { return false; }
    segment.len() <= MAX_ID_BYTES && chars.all(|ch|
        ch.is_alphanumeric() || ch == '_' || (allow_hyphen && ch == '-'))
}
fn validate_identity(id: &NominalId, pos: Pos) -> R<()> {
    if !valid_segment(&id.package, true) ||
        !valid_segment(&id.namespace, true) ||
        !valid_segment(&id.name, false) {
        return Err(error(pos, "E_NAMESPACE",
            "nominal identity must contain bounded, unambiguous package/namespace/class segments"));
    }
    Ok(())
}
/// Scope a fully compiler-validated *local* manifest under build-assigned
/// package and namespace labels. Every edge is explicitly qualified to the
/// SAME unit. It cannot make a foreign class look like a permitted local one.
pub fn qualify_local(classes: &[ClassHierarchy], package: &str, namespace: &str) -> R<Vec<LinkClass>> {
    let mut result = Vec::with_capacity(classes.len());
    for class in classes {
        let id = NominalId {
            package: package.into(), namespace: namespace.into(), name: class.name.clone(),
        };
        validate_identity(&id, class.pos)?;
        let scoped = |name: &String| NominalId {
            package: package.into(), namespace: namespace.into(), name: name.clone(),
        };
        result.push(LinkClass {
            id,
            pos: class.pos,
            visibility: class.visibility,
            hierarchy: class.hierarchy,
            direct_parents: class.direct_parents.iter().map(scoped).collect(),
            permitted_children: class.permitted_children.iter().map(scoped).collect(),
        });
    }
    Ok(result)
}
fn visible(from: &NominalId, to: &LinkClass) -> bool {
    if from.package == to.id.package && from.namespace == to.id.namespace {
        return true;
    }
    match to.visibility {
        ClassVisibility::Public => true,
        ClassVisibility::Package => from.package == to.id.package,
        ClassVisibility::Private => false,
    }
}
fn visit<'a>(
    id: &'a NominalId,
    graph: &BTreeMap<NominalId, &'a LinkClass>,
    visited: &mut BTreeSet<NominalId>,
    visiting: &mut BTreeSet<NominalId>,
) -> R<()> {
    if visited.contains(id) { return Ok(()); }
    let class = graph.get(id).expect("validated class identity exists");
    if !visiting.insert(id.clone()) {
        return Err(error(class.pos, "E_CYCLE", "cycle in nominal inheritance graph"));
    }
    for parent in &class.direct_parents {
        if visiting.contains(parent) {
            return Err(error(class.pos, "E_CYCLE", "cycle in nominal inheritance graph"));
        }
        visit(parent, graph, visited, visiting)?;
    }
    visiting.remove(id);
    visited.insert(id.clone());
    Ok(())
}

/// Validate the complete, closed, namespace-qualified hierarchy policy.
///
/// This is a pre-link policy for future authenticated compiler artifacts, NOT
/// a replacement for source parsing, linking code, layout/override checking,
/// nor acceptance of untrusted custom sections as class credentials.
pub fn validate_prelink(classes: &[LinkClass]) -> R<()> {
    if classes.len() > MAX_LINK_CLASSES {
        return Err(error(Pos { line: 1, col: 1 }, "E_SOURCE_LIMIT",
            "bounded nominal prelink class budget exceeded"));
    }
    let mut graph: BTreeMap<NominalId, &LinkClass> = BTreeMap::new();
    for class in classes {
        validate_identity(&class.id, class.pos)?;
        if class.direct_parents.len() > MAX_EDGES || class.permitted_children.len() > MAX_EDGES {
            return Err(error(class.pos, "E_SOURCE_LIMIT",
                "bounded nominal prelink edge budget exceeded"));
        }
        let mut parents = BTreeSet::new();
        let mut permitted = BTreeSet::new();
        for id in &class.direct_parents {
            validate_identity(id, class.pos)?;
            if !parents.insert(id) {
                return Err(error(class.pos, "E_DUPLICATE", "duplicate fully-qualified parent"));
            }
        }
        for id in &class.permitted_children {
            validate_identity(id, class.pos)?;
            if !permitted.insert(id) || *id == class.id {
                return Err(error(class.pos, "E_SEALED", "duplicate or self permitted class identity"));
            }
        }
        if class.hierarchy == Hierarchy::Sealed {
            if class.permitted_children.is_empty() {
                return Err(error(class.pos, "E_SEALED", "sealed class needs a nonempty permits graph"));
            }
        } else if !class.permitted_children.is_empty() {
            return Err(error(class.pos, "E_SEALED", "only sealed classes can declare permitted children"));
        }
        if graph.insert(class.id.clone(), class).is_some() {
            return Err(error(class.pos, "E_DUPLICATE", "duplicate fully-qualified class identity"));
        }
    }
    for class in classes {
        let mut directly_extends_sealed = false;
        for parent_id in &class.direct_parents {
            if parent_id == &class.id {
                return Err(error(class.pos, "E_CYCLE", "class cannot extend itself"));
            }
            let parent = graph.get(parent_id).ok_or_else(|| error(class.pos,
                "E_UNBOUND", "parent class has no authenticated fully-qualified identity"))?;
            if !visible(&class.id, parent) {
                return Err(error(class.pos, "E_VISIBILITY", "inaccessible class cannot be extended across a namespace or package"));
            }
            match parent.hierarchy {
                Hierarchy::Final => return Err(error(class.pos, "E_FINAL", "final class cannot be extended")),
                Hierarchy::Sealed => {
                    directly_extends_sealed = true;
                    if !parent.permitted_children.contains(&class.id) {
                        return Err(error(class.pos, "E_SEALED",
                            "sealed parent did not permit this exact fully-qualified class identity"));
                    }
                    if class.hierarchy == Hierarchy::Open {
                        return Err(error(class.pos, "E_SEALED",
                            "direct child of sealed class must be final, sealed or non-sealed"));
                    }
                }
                _ => {}
            }
        }
        if class.hierarchy == Hierarchy::NonSealed && !directly_extends_sealed {
            return Err(error(class.pos, "E_SEALED",
                "non-sealed class requires a direct sealed parent"));
        }
        if class.hierarchy == Hierarchy::Sealed {
            for child_id in &class.permitted_children {
                let child = graph.get(child_id).ok_or_else(|| error(class.pos,
                    "E_UNBOUND", "permitted subclass has no authenticated fully-qualified identity"))?;
                if !child.direct_parents.contains(&class.id) {
                    return Err(error(class.pos, "E_SEALED",
                        "permitted child does not directly extend the exact fully-qualified parent"));
                }
                if !visible(&class.id, child) {
                    return Err(error(class.pos, "E_VISIBILITY",
                        "sealed declaration cannot authorize an inaccessible subclass"));
                }
                if child.hierarchy == Hierarchy::Open {
                    return Err(error(class.pos, "E_SEALED", "permitted child cannot be open"));
                }
            }
        }
    }
    let mut visited = BTreeSet::new();
    let mut visiting = BTreeSet::new();
    for id in graph.keys() {
        visit(id, &graph, &mut visited, &mut visiting)?;
    }
    Ok(())
}
