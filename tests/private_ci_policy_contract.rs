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

fn comment_only_policy(source: &str) -> bool {
    source
        .lines()
        .all(|line| line.trim().is_empty() || line.trim_start().starts_with('#'))
}

#[test]
fn rust_classifier_accepts_only_comment_or_blank_destinations() {
    for allowed in [
        "",
        "\n",
        "# disabled by default\n",
        "  \t  \n # explanatory comment\n\t# trailing comment\n",
    ] {
        assert!(comment_only_policy(allowed));
    }
    for denied in [
        "third-party/private-repo 777\n",
        " \tthird-party/private-repo 777\n",
        "# comment\nthird-party/private-repo 777\n",
        "# comment\nthird-party/private-repo 777 # sneaky\n",
        "\0",
    ] {
        assert!(!comment_only_policy(denied));
    }
}

#[test]
fn checked_in_rust_policy_preserves_source_identity_and_allowlist_limits() {
    assert!(ALLOWLIST.len() <= 65_536);
    assert!(!ALLOWLIST.contains('\0'));
    assert!(comment_only_policy(ALLOWLIST));
    assert!(WORKFLOW.contains("test ! -L \"$file\""));
    assert!(WORKFLOW.contains("test \"$(stat -c %s -- \"$file\")\" -le 65536"));
}
