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
    // The policy was extracted into a dedicated binary-safe script. Verify
    // that the workflow actually invokes it and its adversarial test suite.
    let guard = include_str!("../scripts/private-ci-export-policy-guard.sh");
    assert_eq!(WORKFLOW.matches("run: bash scripts/private-ci-export-policy-guard.sh .github/private-ci-approved-repositories.txt").count(), 1);
    assert_eq!(WORKFLOW.matches("run: bash tests/private-ci-export-policy-guard-test.sh").count(), 1);
    assert_eq!(guard.matches("PRIVATE_CI_EXPORT_REJECTED:").count(), 1);
    assert!(guard.contains("0) reject 'active destination requires independently reviewed authorization'"));
    assert!(guard.contains("*) reject 'policy classifier failed'"));
    assert!(WORKFLOW.contains("persist-credentials: false"));
    assert!(WORKFLOW.contains("git diff --cached --quiet --exit-code HEAD --"));
    assert!(WORKFLOW.contains("git write-tree"));
    assert!(WORKFLOW.contains("git ls-files --others --ignored"));
    assert!(!WORKFLOW.contains("source mirror"));
    // Execute the actual classifier against synthetic source-free fixtures.
    // No checkout, export, network or credentials are involved.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = std::process::Command::new("bash")
        .current_dir(root)
        .arg("tests/private-ci-export-policy-guard-test.sh")
        .output().expect("bash required for policy guard regression");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(String::from_utf8_lossy(&out.stdout).contains("PASS: 25 deny-by-default private CI policy regression cases"));
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
    assert!(include_str!("../scripts/private-ci-export-policy-guard.sh").contains("! -L \"$file\""));
    let guard = include_str!("../scripts/private-ci-export-policy-guard.sh");
    assert!(guard.contains("\"$bytes\" -le 65536"));
    assert!(guard.contains("hardlinked policy file forbidden"));
    assert!(guard.contains("policy path contains a symlinked component"));
    assert!(guard.contains("binary, control or non-ASCII policy input"));
}
