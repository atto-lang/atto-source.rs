//! atto.lock v1: canonical form, content-verified vendoring, offline mode,
//! deterministic install order and fail-closed validation.
use atto_compiler::dependency_lock::{install_order, parse, render, tree_digest, verify, Lock, LockedPackage, Source, VendoredTree};

const JSON_MAIN: &[u8] = b"pub fnc parse(): int { return 1; }\n";
const JSON_UTIL: &[u8] = b"pub fnc helper(): int { return 2; }\n";
const B64: &[u8] = b"pub fnc encode(): int { return 3; }\n";

fn json_tree() -> Vec<(&'static str, &'static [u8])> {
    vec![("src/json.atto", JSON_MAIN), ("src/util.atto", JSON_UTIL)]
}
fn b64_tree() -> Vec<(&'static str, &'static [u8])> {
    vec![("src/base64.atto", B64)]
}

fn sample(offline: bool) -> Lock {
    Lock {
        root: "acme@1.0.0".into(),
        offline,
        packages: vec![
            LockedPackage {
                name: "json".into(),
                version: "1.2.3".into(),
                source: Source::Vendor("vendor/json".into()),
                tree_sha256: tree_digest(&json_tree()).unwrap(),
                deps: vec!["base64@0.4.1".into()],
            },
            LockedPackage {
                name: "base64".into(),
                version: "0.4.1".into(),
                source: Source::Vendor("vendor/base64".into()),
                tree_sha256: tree_digest(&b64_tree()).unwrap(),
                deps: vec![],
            },
        ],
    }
}

fn vendored() -> Vec<VendoredTree<'static>> {
    vec![
        VendoredTree {
            dir: "vendor/json",
            files: json_tree(),
        },
        VendoredTree {
            dir: "vendor/base64",
            files: b64_tree(),
        },
    ]
}

#[test]
fn render_parse_round_trip_is_byte_stable() {
    let text = render(&sample(true));
    let parsed = parse(&text).expect("canonical lock parses");
    assert_eq!(render(&parsed), text);
    assert_eq!(parsed.packages[0].name, "base64", "packages are sorted by name");
}

#[test]
fn non_canonical_lock_is_rejected() {
    let text = render(&sample(true));
    let reformatted = text.replace("name = \"json\"", "name   = \"json\"");
    assert_eq!(parse(&reformatted).unwrap_err().code, "E_LOCK_CANONICAL");
    let crlf = text.replace('\n', "\r\n");
    assert_eq!(parse(&crlf).unwrap_err().code, "E_LOCK");
}

#[test]
fn tree_digest_is_order_independent_and_content_sensitive() {
    let mut reversed = json_tree();
    reversed.reverse();
    assert_eq!(tree_digest(&json_tree()).unwrap(), tree_digest(&reversed).unwrap());
    let edited: Vec<(&str, &[u8])> = vec![("src/json.atto", b"changed"), ("src/util.atto", JSON_UTIL)];
    assert_ne!(tree_digest(&json_tree()).unwrap(), tree_digest(&edited).unwrap());
    let renamed: Vec<(&str, &[u8])> = vec![("src/json2.atto", JSON_MAIN), ("src/util.atto", JSON_UTIL)];
    assert_ne!(tree_digest(&json_tree()).unwrap(), tree_digest(&renamed).unwrap());
}

#[test]
fn tree_digest_rejects_escapes_and_duplicates() {
    for bad in ["../x.atto", "/abs.atto", "a/./b.atto", "a//b.atto", ""] {
        let files: Vec<(&str, &[u8])> = vec![(bad, b"x")];
        assert_eq!(tree_digest(&files).unwrap_err().code, "E_LOCK_TREE", "{bad}");
    }
    let dup: Vec<(&str, &[u8])> = vec![("a.atto", b"1"), ("a.atto", b"2")];
    assert_eq!(tree_digest(&dup).unwrap_err().code, "E_LOCK_TREE");
}

#[test]
fn offline_build_verifies_every_vendored_tree() {
    let lock = parse(&render(&sample(true))).unwrap();
    assert_eq!(verify(&lock, &vendored()).unwrap(), Vec::<String>::new());
}

#[test]
fn tampered_vendored_tree_fails_closed() {
    let lock = sample(true);
    let tampered = vec![
        VendoredTree {
            dir: "vendor/json",
            files: vec![("src/json.atto", b"evil" as &[u8]), ("src/util.atto", JSON_UTIL)],
        },
        VendoredTree {
            dir: "vendor/base64",
            files: b64_tree(),
        },
    ];
    assert_eq!(verify(&lock, &tampered).unwrap_err().code, "E_LOCK_HASH");
    assert_eq!(verify(&lock, &vendored()[..1]).unwrap_err().code, "E_LOCK_MISSING");
}

#[test]
fn offline_mode_refuses_registry_only_packages() {
    let mut lock = sample(true);
    lock.packages[1].source = Source::Registry("atto-lang/base64".into());
    assert_eq!(verify(&lock, &vendored()).unwrap_err().code, "E_LOCK_OFFLINE");
    lock.offline = false;
    assert_eq!(verify(&lock, &vendored()).unwrap(), vec!["base64@0.4.1".to_string()]);
}

#[test]
fn install_order_is_dependencies_first_and_deterministic() {
    assert_eq!(install_order(&sample(true)).unwrap(), vec!["base64@0.4.1", "json@1.2.3"]);
}

#[test]
fn cycles_duplicates_and_unresolved_deps_fail_closed() {
    let mut cyclic = sample(true);
    cyclic.packages[1].deps = vec!["json@1.2.3".into()];
    assert_eq!(parse(&render(&cyclic)).unwrap_err().code, "E_LOCK_CYCLE");

    let mut dup = sample(true);
    let mut second = dup.packages[1].clone();
    second.version = "0.5.0".into();
    dup.packages.push(second);
    assert_eq!(parse(&render(&dup)).unwrap_err().code, "E_LOCK_DUPLICATE");

    let mut unresolved = sample(true);
    unresolved.packages[0].deps = vec!["utf8@1.0.0".into()];
    assert_eq!(parse(&render(&unresolved)).unwrap_err().code, "E_LOCK_UNRESOLVED");
}

#[test]
fn strict_grammar_rejects_unknown_keys_versions_and_sources() {
    let text = render(&sample(false));
    assert_eq!(
        parse(&text.replace("version = \"1\"\n", "version = \"2\"\n")).unwrap_err().code,
        "E_LOCK_VERSION"
    );
    assert_eq!(
        parse(&text.replace("deps = \"\"", "deps = \"\"\nfeatures = \"x\""))
            .unwrap_err()
            .code,
        "E_LOCK"
    );
    assert_eq!(
        parse(&text.replace("vendor:vendor/base64", "git:https://x")).unwrap_err().code,
        "E_LOCK"
    );
    assert_eq!(parse(&text.replace("\"0.4.1\"", "\"0.4\"")).unwrap_err().code, "E_LOCK");
    assert_eq!(parse(&text.replace("\"0.4.1\"", "\"00.4.1\"")).unwrap_err().code, "E_LOCK");
}
