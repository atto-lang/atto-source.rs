//! Backend-neutral data-layout IR v1 producer for oreslang-static-analysis.rs.
//!
//! The producer exports only evidence already present in validated MIR. It does
//! not infer relocation safety or hotness from syntax, and it fails closed when
//! layout evidence is incomplete or internally inconsistent.
use crate::mir::{LayoutRegion, LayoutScope, Module, PhysicalLayout};
use serde_json::{json, Value};
use std::{collections::HashSet, fmt};

pub const LAYOUT_SCHEMA_VERSION: u32 = 1;
const MAX_REGIONS: usize = 10_000;
const MAX_FIELDS_PER_REGION: usize = 16_384;
const MAX_TOTAL_FIELD_ITEMS: usize = 1_000_000;
const MAX_REGION_NAME_BYTES: usize = 1_024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TargetLayout {
    pub cache_line_bytes: Option<u32>,
    pub vector_bytes: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutExportError {
    pub class: &'static str,
    pub message: String,
}

impl fmt::Display for LayoutExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.class, self.message)
    }
}

impl std::error::Error for LayoutExportError {}

fn failure(class: &'static str, message: impl Into<String>) -> LayoutExportError {
    LayoutExportError {
        class,
        message: message.into(),
    }
}

fn target_width_valid(value: Option<u32>) -> bool {
    value.is_none_or(|bytes| bytes >= 8 && bytes.is_power_of_two())
}

fn scope_name(scope: LayoutScope) -> &'static str {
    match scope {
        LayoutScope::Core => "core",
        LayoutScope::Stdlib => "stdlib",
        LayoutScope::Runtime => "runtime",
        LayoutScope::Userland => "userland",
    }
}

fn layout_name(layout: PhysicalLayout) -> &'static str {
    match layout {
        PhysicalLayout::Aos => "aos",
        PhysicalLayout::Soa => "soa",
        PhysicalLayout::Aosoa => "aosoa",
        PhysicalLayout::SparseSet => "sparse_set",
        PhysicalLayout::Archetype => "archetype",
    }
}

fn validate_region(region: &LayoutRegion) -> Result<(), LayoutExportError> {
    if region.name.trim().is_empty() {
        return Err(failure("layout_invalid_region", "layout region name is blank"));
    }
    if region.name.len() > MAX_REGION_NAME_BYTES {
        return Err(failure(
            "layout_budget",
            format!("layout region name exceeds {MAX_REGION_NAME_BYTES} UTF-8 bytes"),
        ));
    }
    if region.field_bytes.is_empty() || region.field_bytes.iter().any(|bytes| *bytes == 0) {
        return Err(failure(
            "layout_invalid_region",
            format!("{}: field widths must be non-zero", region.name),
        ));
    }
    if region.field_bytes.len() > MAX_FIELDS_PER_REGION
        || region.accessed_fields.len() > MAX_FIELDS_PER_REGION
    {
        return Err(failure(
            "layout_budget",
            format!(
                "{}: field/access list exceeds {MAX_FIELDS_PER_REGION} entries",
                region.name
            ),
        ));
    }

    let mut indexes = HashSet::new();
    for index in &region.accessed_fields {
        if !indexes.insert(*index) {
            return Err(failure(
                "layout_invalid_region",
                format!("{}: duplicate accessed field index {index}", region.name),
            ));
        }
        if (*index as usize) >= region.field_bytes.len() {
            return Err(failure(
                "layout_invalid_region",
                format!("{}: accessed field index {index} is out of bounds", region.name),
            ));
        }
    }
    if region.hot && region.rows != Some(0) && region.accessed_fields.is_empty() {
        return Err(failure(
            "layout_invalid_region",
            format!("{}: non-empty hot pass has no accessed fields", region.name),
        ));
    }
    Ok(())
}

/// Export the layout/access evidence attached to validated MIR.
///
/// The scalar frontend currently admits no aggregate storage, so a normally
/// lowered scalar Module exports a complete empty region list. When aggregate
/// lowering is added, it must populate Module::layout_regions and may only set
/// layout_evidence_complete once every relevant region is represented.
pub fn export_layout_ir_checked(
    module: &Module,
    target: TargetLayout,
) -> Result<Value, LayoutExportError> {
    if !module.layout_evidence_complete {
        return Err(failure(
            "layout_incomplete",
            "MIR layout evidence is not marked complete",
        ));
    }
    if !target_width_valid(target.cache_line_bytes) || !target_width_valid(target.vector_bytes) {
        return Err(failure(
            "layout_target",
            "target cache/vector widths must be null or powers of two >= 8 bytes",
        ));
    }
    if module.layout_regions.len() > MAX_REGIONS {
        return Err(failure(
            "layout_budget",
            format!("layout region count exceeds {MAX_REGIONS}"),
        ));
    }

    let total_items = module.layout_regions.iter().try_fold(0_usize, |total, region| {
        total
            .checked_add(region.field_bytes.len())?
            .checked_add(region.accessed_fields.len())
    });
    if total_items.is_none_or(|count| count > MAX_TOTAL_FIELD_ITEMS) {
        return Err(failure(
            "layout_budget",
            format!("layout evidence exceeds {MAX_TOTAL_FIELD_ITEMS} field/access items"),
        ));
    }

    let mut names = HashSet::new();
    let mut regions = Vec::with_capacity(module.layout_regions.len());
    for region in &module.layout_regions {
        validate_region(region)?;
        if !names.insert(region.name.as_str()) {
            return Err(failure(
                "layout_invalid_region",
                format!("duplicate layout region '{}'", region.name),
            ));
        }
        regions.push(json!({
            "name": region.name,
            "scope": scope_name(region.scope),
            "layout": layout_name(region.layout),
            "rows": region.rows,
            "field_bytes": region.field_bytes,
            "accessed_fields": region.accessed_fields,
            "dense_iteration": region.dense_iteration,
            "sequential_rows": region.sequential_rows,
            "hot": region.hot,
            "relocation_verified": region.relocation_verified,
            "stable_address_required": region.stable_address_required,
            "whole_record_access": region.whole_record_access,
            "iteration_independent": region.iteration_independent,
            "sparse_membership": region.sparse_membership,
            "independent_component_passes": region.independent_component_passes,
            "concurrent_mutable_sharing": region.concurrent_mutable_sharing
        }));
    }

    Ok(json!({
        "schema_version": LAYOUT_SCHEMA_VERSION,
        "evidence_complete": true,
        "target": {
            "cache_line_bytes": target.cache_line_bytes,
            "vector_bytes": target.vector_bytes
        },
        "regions": regions
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{frontend, mir};

    fn scalar_module() -> Module {
        let ast = frontend::parse("pub fnc main(): int { return 42; }").unwrap();
        mir::lower(ast).unwrap()
    }

    fn sample_region() -> LayoutRegion {
        LayoutRegion {
            name: "core.positions".into(),
            scope: LayoutScope::Core,
            layout: PhysicalLayout::Aos,
            rows: Some(4096),
            field_bytes: vec![4, 4, 4, 4, 8, 8],
            accessed_fields: vec![0, 1],
            dense_iteration: true,
            sequential_rows: true,
            hot: true,
            relocation_verified: true,
            stable_address_required: false,
            whole_record_access: false,
            iteration_independent: true,
            sparse_membership: false,
            independent_component_passes: false,
            concurrent_mutable_sharing: false,
        }
    }

    #[test]
    fn scalar_subset_exports_complete_empty_layout_manifest() {
        let ir = export_layout_ir_checked(&scalar_module(), TargetLayout::default()).unwrap();
        assert_eq!(ir["schema_version"], 1);
        assert_eq!(ir["evidence_complete"], true);
        assert_eq!(ir["regions"].as_array().unwrap().len(), 0);
        assert!(ir["target"]["vector_bytes"].is_null());
    }

    #[test]
    fn typed_mir_region_exports_exact_layout_fields() {
        let mut module = scalar_module();
        module.layout_regions.push(sample_region());
        let ir = export_layout_ir_checked(
            &module,
            TargetLayout {
                cache_line_bytes: Some(64),
                vector_bytes: Some(32),
            },
        )
        .unwrap();
        assert_eq!(ir["regions"][0]["scope"], "core");
        assert_eq!(ir["regions"][0]["layout"], "aos");
        assert_eq!(ir["regions"][0]["field_bytes"][4], 8);
        assert_eq!(ir["regions"][0]["accessed_fields"][1], 1);
    }

    #[test]
    fn incomplete_or_forged_layout_evidence_fails_closed() {
        let mut module = scalar_module();
        module.layout_evidence_complete = false;
        assert_eq!(
            export_layout_ir_checked(&module, TargetLayout::default())
                .unwrap_err()
                .class,
            "layout_incomplete"
        );

        let mut module = scalar_module();
        let mut region = sample_region();
        region.accessed_fields = vec![0, 0];
        module.layout_regions.push(region);
        assert_eq!(
            export_layout_ir_checked(&module, TargetLayout::default())
                .unwrap_err()
                .class,
            "layout_invalid_region"
        );
    }

    #[test]
    fn out_of_range_field_and_invalid_target_are_rejected() {
        let mut module = scalar_module();
        let mut region = sample_region();
        region.accessed_fields = vec![99];
        module.layout_regions.push(region);
        assert_eq!(
            export_layout_ir_checked(&module, TargetLayout::default())
                .unwrap_err()
                .class,
            "layout_invalid_region"
        );

        assert_eq!(
            export_layout_ir_checked(
                &scalar_module(),
                TargetLayout {
                    cache_line_bytes: Some(64),
                    vector_bytes: Some(24),
                },
            )
            .unwrap_err()
            .class,
            "layout_target"
        );
    }

    #[test]
    fn semantic_suppression_evidence_is_preserved_not_inferred() {
        let mut module = scalar_module();
        let mut region = sample_region();
        region.relocation_verified = false;
        region.stable_address_required = true;
        region.whole_record_access = true;
        module.layout_regions.push(region);
        let ir = export_layout_ir_checked(&module, TargetLayout::default()).unwrap();
        assert_eq!(ir["regions"][0]["relocation_verified"], false);
        assert_eq!(ir["regions"][0]["stable_address_required"], true);
        assert_eq!(ir["regions"][0]["whole_record_access"], true);
    }
}
