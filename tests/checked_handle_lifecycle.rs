//! Real compiler tests for the bounded local checked-handle profile.
//! Compiled guest execution and compiler byte parity are covered by the two
//! tests/fixtures/pass/checked-handle-*.atto files in conformance.mjs.
//! These tests are NOT proof of arbitrary multi-actor or freed-arena handles.

const CLASS: &str = r#"
define class Parcel as
  pub let int value = 42;
  pub get(): int { return self.value; }
end
"#;

fn program(body: &str) -> String {
    format!("{CLASS}\npub fnc main(): int {{\n{body}\n}}\n")
}

fn check_error(body: &str, code: &str) {
    let src = program(body);
    let diagnostic = atto_compiler::compile(&src)
        .expect_err("invalid checked handle must not produce guest bytes");
    assert_eq!(diagnostic.code, code, "{body}: {diagnostic}");
}

#[test]
fn canonical_and_legacy_lifecycle_compile() {
    for form in ["bang", "rt", "compact"] {
        let (create, read, release) = if form == "bang" {
            ("ref! original", "deref! handle", "unref! handle")
        } else if form == "compact" {
            ("ref!original", "deref!handle", "unref!handle")
        } else {
            ("rt ref original", "rt deref handle", "rt unref handle")
        };
        let source = program(&format!(r#"
            val original = new Parcel();
            val handle = {create};
            block {{
                val view = {read};
                stdio.println(view.get());
            }}
            val restored = {release};
            return restored.get();
        "#));
        let bytes = atto_compiler::compile(&source)
            .unwrap_or_else(|d| panic!("{form} form failed: {d}"));
        assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
        assert_eq!(atto_compiler::compile(&source).unwrap(), bytes);
    }
}

#[test]
fn rejects_use_after_ref_move_and_conflicting_loans() {
    check_error(r#"
        val original = new Parcel();
        val handle = ref! original;
        val illegal = original.value;
        val restored = unref! handle;
        return illegal + restored.value;
    "#, "E_MOVED");
    check_error(r#"
        val original = new Parcel();
        val handle = ref! original;
        val view = deref! handle;
        val restored = unref! handle;
        return view.value + restored.value;
    "#, "E_BORROW");
    check_error(r#"
        val original = new Parcel();
        val view = borrow! original;
        val handle = ref! original;
        val restored = unref! handle;
        return restored.value + view.value;
    "#, "E_BORROW");
}

#[test]
fn rejects_double_release_stale_view_and_forgery() {
    check_error(r#"
        val original = new Parcel();
        val handle = ref! original;
        val restored = unref! handle;
        val other = unref! handle;
        return restored.value + other.value;
    "#, "E_MOVED");
    check_error(r#"
        val original = new Parcel();
        val handle = ref! original;
        val restored = unref! handle;
        block { val invalid = deref! handle; }
        return restored.value;
    "#, "E_MOVED");
    check_error(r#"val view = deref! 42; return 42;"#, "E_HANDLE");
    check_error(r#"val bogus = unref! 42; return 42;"#, "E_HANDLE");
    check_error(r#"val bogus = ref! 42; return 42;"#, "E_HANDLE");
}

#[test]
fn rejects_alias_escape_and_implicit_release() {
    check_error(r#"
        val original = new Parcel();
        val handle = ref! original;
        val alias = handle;
        val restored = unref! handle;
        return restored.value;
    "#, "E_HANDLE");
    check_error(r#"
        val original = new Parcel();
        val handle = ref! original;
        return 42;
    "#, "E_HANDLE");
    check_error(r#"
        val original = new Parcel();
        val handle = ref! original;
        unref! handle;
        return 42;
    "#, "E_HANDLE");
    check_error(r#"
        val handle = ref! new Parcel();
        val restored = unref! handle;
        return restored.value;
    "#, "E_HANDLE");
}

#[test]
fn rejects_mutable_views_nested_release_and_bad_qualifiers() {
    check_error(r#"
        val original = new Parcel();
        val handle = ref! original;
        let view = deref! handle;
        val restored = unref! handle;
        return restored.value;
    "#, "E_HANDLE");
    check_error(r#"
        val original = new Parcel();
        val handle = ref! original;
        if true then
            val restored = unref! handle;
            return restored.value;
        fi
        return 42;
    "#, "E_HANDLE");
    check_error(r#"
        val original = new Parcel();
        val handle = ref! mut original;
        return 42;
    "#, "E_SYNTAX");
    // A gap before ! must not parse as an ownership operator.
    assert!(atto_compiler::compile(&program(r#"
        val original = new Parcel();
        val handle = ref !original;
        return 42;
    "#)).is_err());
}

#[test]
fn reject_borrowed_view_escape() {
    let source = format!(r#"{CLASS}
    fnc invalid_escape(): Parcel {{
        val original = new Parcel();
        val handle = ref! original;
        val view = deref! handle;
        return view;
    }}
    pub fnc main(): int {{ return 42; }}"#);
    let d = atto_compiler::compile(&source).unwrap_err();
    assert_eq!(d.code, "E_BORROW");
}

#[test]
fn rejects_early_exit_before_later_unwrap() {
    check_error(r#"
        val original = new Parcel();
        val handle = ref! original;
        if true then return 1; fi
        val recovered = unref! handle;
        return recovered.value;
    "#, "E_HANDLE");
    check_error(r#"
        val original = new Parcel();
        val handle = ref! original;
        block { return 1; }
        val recovered = unref! handle;
        return recovered.value;
    "#, "E_HANDLE");
    check_error(r#"
        val original = new Parcel();
        val handle = ref! original;
        return 1;
        val recovered = unref! handle;
    "#, "E_HANDLE");
}

#[test]
fn forbids_conditional_unwrap_and_handle_argument_escape() {
    check_error(r#"
        val original = new Parcel();
        val handle = ref! original;
        val restored = true ? unref! handle : new Parcel();
        return restored.value;
    "#, "E_HANDLE");
    check_error(r#"
        val original = new Parcel();
        val handle = ref! original;
        stdio.println(handle);
        val restored = unref! handle;
        return restored.value;
    "#, "E_HANDLE");
}

#[test]
fn gaps_before_bang_and_inequality_are_not_handle_prefixes() {
    for spelling in ["ref !original", "ref!=original", "deref !original", "unref !original"] {
        let source = program(&format!(r#"
            val original = new Parcel();
            val handle = {spelling};
            return 42;
        "#));
        assert!(atto_compiler::compile(&source).is_err(), "{spelling}");
    }
}

#[test]
fn block_keyword_ends_two_immutable_loans_before_unref() {
    let source = program(r#"
        val original = new Parcel();
        val handle = ref! original;
        block {
            val first = deref! handle;
            val second = deref! handle;
            stdio.println(first.get() + second.get());
        }
        val restored = unref! handle;
        return restored.get();
    "#);
    let bytes = atto_compiler::compile(&source)
        .unwrap_or_else(|d| panic!("canonical block loan retirement: {d}"));
    assert_eq!(&bytes[..8], b"\0asm\x01\0\0\0");
}

#[test]
fn standalone_braces_are_not_block_scope_syntax() {
    let source = program(r#"
        val original = new Parcel();
        val handle = ref! original;
        {
            val view = deref! handle;
            stdio.println(view.get());
        }
        val restored = unref! handle;
        return restored.get();
    "#);
    let error = atto_compiler::compile(&source).unwrap_err();
    assert_eq!(error.code, "E_UNSUPPORTED");
}

#[test]
fn lexical_loan_must_end_before_consuming_checked_handle() {
    check_error(r#"
        val original = new Parcel();
        val handle = ref! original;
        block {
            val view = deref! handle;
            val restored = unref! handle;
            stdio.println(view.get());
        }
        return 42;
    "#, "E_HANDLE");
}

#[test]
fn runtime_handle_ops_require_val_not_const_or_let() {
    for keyword in ["const", "let"] {
        check_error(&format!(r#"
            val original = new Parcel();
            {keyword} handle = ref! original;
            return 42;
        "#), "E_HANDLE");
        check_error(&format!(r#"
            val original = new Parcel();
            val handle = ref! original;
            block {{
                {keyword} view = deref! handle;
            }}
            val restored = unref! handle;
            return restored.value;
        "#), "E_HANDLE");
        check_error(&format!(r#"
            val original = new Parcel();
            val handle = ref! original;
            {keyword} restored = unref! handle;
            return restored.value;
        "#), "E_HANDLE");
    }
}
