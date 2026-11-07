//! Source-owned workflow contract. These are host-only structural assertions;
//! they are not evidence that Actions jobs or native Oreslang guests ran.
const WORKFLOW: &str = include_str!("../.github/workflows/private-ci-policy.yml");
const ALLOWLIST: &str =
    include_str!("../.github/private-ci-approved-repositories.txt");

#[test]
fn private_source_export_stays_denied_by_default() {
    for entry in ALLOWLIST.lines() {
        let line = entry.trim();
        assert!(
            line.is_empty() || line.starts_with('#'),
            "active private-source export destination is forbidden"
        );
    }
}

#[test]
fn workflow_has_one_unbroken_active_destination_rejection() {
    let guard = "if grep -Evq '^[[:space:]]*(#.*)?$' \"$file\"; then";
    assert_eq!(WORKFLOW.matches(guard).count(), 1);
    assert_eq!(WORKFLOW.matches("PRIVATE_CI_EXPORT_REJECTED:").count(), 1);
    assert!(WORKFLOW.contains("persist-credentials: false"));
    assert!(WORKFLOW.contains("git diff --cached --quiet --exit-code HEAD --"));
    assert!(WORKFLOW.contains("git write-tree"));
    assert!(WORKFLOW.contains("git ls-files --others --ignored"));
    assert!(WORKFLOW.contains("if ! grep -Evq '^[[:space:]]*(#.*)?$'"));
    assert!(!WORKFLOW.contains("source mirror"));
}
