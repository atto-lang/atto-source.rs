//! Source-owned workflow contract. These are host-only structural assertions;
//! they are not evidence that Actions jobs or native Oreslang guests ran.
const WORKFLOW: &str = include_str!("../.github/workflows/private-ci-policy.yml");
const GUARD: &str = include_str!("../scripts/private-ci-export-policy-guard.sh");
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
    assert_eq!(WORKFLOW.matches("run: bash scripts/private-ci-export-policy-guard.sh .github/private-ci-approved-repositories.txt").count(), 1);
    assert!(WORKFLOW.contains("run: bash tests/private-ci-export-policy-guard-test.sh"));
    assert!(WORKFLOW.contains("persist-credentials: false"));
    assert!(WORKFLOW.contains("git diff --cached --quiet --exit-code HEAD --"));
    assert!(WORKFLOW.contains("git write-tree"));
    assert!(WORKFLOW.contains("git ls-files --others --ignored"));
    assert!(!WORKFLOW.contains("source mirror"));
    assert_eq!(GUARD.matches("PRIVATE_CI_EXPORT_REJECTED:").count(), 1);
    assert!(GUARD.contains("0) reject 'active destination requires independently reviewed authorization'"));
    assert!(GUARD.contains("*) reject 'policy classifier failed'"));
    assert!(GUARD.contains("LC_ALL=C grep -aEvq"));
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
    assert!(GUARD.contains("[[ -f \"$file\" && ! -L \"$file\" ]] || reject"));
    assert!(GUARD.contains("[[ \"$bytes\" =~ ^[0-9]+$ && \"$bytes\" -le 65536 ]] || reject"));
    assert!(GUARD.contains("hardlinked policy file forbidden"));
    assert!(GUARD.contains("policy path contains a symlinked component"));
    assert!(GUARD.contains("binary, control or non-ASCII policy input"));
    assert!(GUARD.contains("policy replaced while reading"));
}

#[cfg(target_os = "linux")]
#[test]
fn actual_source_owned_policy_guard_regressions() {
    let output = std::process::Command::new("bash")
        .arg("tests/private-ci-export-policy-guard-test.sh")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output().expect("execute real policy classifier regression");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(String::from_utf8_lossy(&output.stdout).contains("deny-by-default private CI policy regression cases"));
}
