//! Source-owned policy contract. These are host-only structural assertions:
//! they neither approve private source export nor prove compiler guest execution.
//! The independently reviewed, protected-main policy and per-source security
//! decision remain prerequisites for transferring private Git objects.
const WORKFLOW: &str = include_str!("../.github/workflows/private-ci-policy.yml");
const GUARD: &str = include_str!("../scripts/private-ci-export-policy-guard.sh");
const PREFLIGHT: &str = include_str!("../scripts/private-ci-source-tree-preflight.sh");
const PREFLIGHT_TESTS: &str = include_str!("../tests/private-ci-source-tree-preflight-test.sh");
const ALLOWLIST: &str = include_str!("../.github/private-ci-approved-repositories.txt");
const APPROVED_ENTRY: &str = "channelsiege/atto-ci 1458328077";

// A separate Rust oracle for the checked-in shell guard. This intentionally
// models the *allowed file syntax*, not an export authorization decision.
// No arbitrary owner/repo pair may be inferred as approved.
fn classify_policy(source: &[u8]) -> Result<bool, &'static str> {
    if source.len() > 65_536 {
        return Err("policy exceeds 64 KiB");
    }
    if source.iter().any(|b| !matches!(*b, b'\t' | b'\n' | b'\r' | 0x20..=0x7e)) {
        return Err("control, binary or non-ASCII bytes");
    }
    for (i, b) in source.iter().enumerate() {
        if *b == b'\r' && source.get(i + 1) != Some(&b'\n') {
            return Err("bare or embedded carriage return");
        }
    }
    let text = std::str::from_utf8(source).map_err(|_| "invalid UTF-8")?;
    let mut found = false;
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let trimmed = line.trim_start_matches(|c| c == ' ' || c == '\t');
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // The shell guard compares the exact active line after grep. Do not
        // accept CRLF, padded, duplicated, or multi-destination active lines.
        if raw != line || found || line != APPROVED_ENTRY {
            return Err("unapproved or ambiguous destination");
        }
        found = true;
    }
    Ok(found)
}

#[test]
fn deny_by_default_and_only_one_exact_carrier_policy_syntax() {
    for disabled in [
        b"".as_slice(),
        b"\n",
        b"# all source exports disabled\n",
        b" \t \n  # still disabled\r\n",
        b"# final comment without newline",
    ] {
        assert_eq!(classify_policy(disabled), Ok(false));
    }
    for valid in [
        APPROVED_ENTRY.to_owned(),
        format!("# independent review still required\n{APPROVED_ENTRY}\n"),
        format!("  # verified immutable repo identity\n{APPROVED_ENTRY}\n# no implicit source scope\n"),
    ] {
        assert_eq!(classify_policy(valid.as_bytes()), Ok(true), "{valid}");
    }
}

#[test]
fn reject_wrong_identity_duplication_controls_or_ambiguous_spelling() {
    let cases = [
        "channelsiege/atto-ci 1458328078\n",
        "other-org/atto-ci 1458328077\n",
        "channelsiege/atto-ci 1458328077 \n",
        " channelsiege/atto-ci 1458328077\n",
        "channelsiege/atto-ci\t1458328077\n",
        "channelsiege/atto-ci 1458328077\r\n",
        "channelsiege/atto-ci 1458328077\nchannelsiege/atto-ci 1458328077\n",
        "channelsiege/atto-ci 1458328077\nother-org/repo 17\n",
        "channelsiege/atto-ci 1458328077 # extra\n",
        "# comment\rhidden/repo 12\n",
        "# comment\0\n",
        "# comment\u{00a0}\n",
        "# comment\x1b\n",
        "# comment\x7f\n",
        "# comment\r",
    ];
    for input in cases {
        assert!(classify_policy(input.as_bytes()).is_err(), "accepted {input:?}");
    }
    assert!(classify_policy(&vec![b'#'; 65_537]).is_err());
    assert_eq!(classify_policy(&vec![b'#'; 65_536]), Ok(false));
}

#[test]
fn checked_in_allowlist_never_grants_multiple_or_unknown_destinations() {
    assert!(ALLOWLIST.len() <= 65_536);
    // This PR proposes exactly one exception; until independent approval
    // AND protected-main merge, its branch-local state is not export authority.
    assert_eq!(classify_policy(ALLOWLIST.as_bytes()), Ok(true));
    let active: Vec<_> = ALLOWLIST.lines()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .collect();
    assert_eq!(active, vec![APPROVED_ENTRY]);
}

#[test]
fn shell_guard_is_pinned_and_binary_safe() {
    assert!(GUARD.contains("approved_entry='channelsiege/atto-ci 1458328077'"));
    assert!(GUARD.contains(r#"[[ "$active" == "$approved_entry" ]]"#));
    assert!(GUARD.contains("grep -aEv"));
    assert!(GUARD.contains("cmp -s --"));
    assert!(GUARD.contains("link_count"));
    assert!(GUARD.contains("realpath -e -s"));
    assert!(GUARD.contains("PRIVATE_CI_EXPORT_REJECTED:"));
    assert!(!GUARD.contains("GITHUB_TOKEN"));
}

#[test]
fn workflow_is_source_bound_and_invokes_the_external_guard() {
    assert!(WORKFLOW.contains("actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683"));
    assert!(WORKFLOW.contains("persist-credentials: false"));
    assert!(WORKFLOW.contains("github.event.pull_request.head.sha || github.sha"));
    assert!(WORKFLOW.contains("git diff --cached --quiet --exit-code HEAD --"));
    assert!(WORKFLOW.contains("git write-tree"));
    assert!(WORKFLOW.contains("git ls-files --others --ignored"));
    assert_eq!(WORKFLOW.matches("bash scripts/private-ci-export-policy-guard.sh .github/private-ci-approved-repositories.txt").count(), 1);
    assert_eq!(WORKFLOW.matches("bash tests/private-ci-export-policy-guard-test.sh").count(), 1);
    assert!(!WORKFLOW.contains("pull_request_target:"));
    assert!(!WORKFLOW.contains("curl "));
    assert!(!WORKFLOW.contains("git push"));
}

#[test]
fn source_preflight_rehashes_worktree_and_checks_lfs_at_any_size() {
    assert!(PREFLIGHT.contains("git hash-object --no-filters --"));
    assert!(PREFLIGHT.contains("stat -c '%h' --"));
    assert!(PREFLIGHT.contains("git cat-file blob"));
    assert!(PREFLIGHT.contains("head -c 42"));
    assert!(!PREFLIGHT.contains("size <= 1024"));
    for case in [
        "oversized LFS-looking blob",
        "normal large blob without LFS header",
        "dirty source hidden by assume-unchanged",
        "dirty source hidden by skip-worktree",
        "tracked source linked outside checkout",
        "clean after external hardlink removed",
        "executable permission hidden by core.filemode false",
    ] {
        assert!(PREFLIGHT_TESTS.contains(case), "missing preflight oracle: {case}");
    }
    assert!(WORKFLOW.contains("bash tests/private-ci-source-tree-preflight-test.sh"));
}
