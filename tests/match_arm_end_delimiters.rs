//! Statement match arms admit either braces or a dedicated per-arm end.
//! This is compiler-backed syntax and fail-closed tests. Structural patterns
//! (~Point) are *not* yet executable and must still be rejected.

use atto_compiler::compile;

fn fixture(arms: &str) -> String {
    format!(r#"pub fnc main(): int {{
        val n = 4;
        match n over
          {arms}
        end
        return 42;
    }}"#)
}

#[test]
fn end_delimited_match_arms_emit_guest_wasm() {
    let src = fixture(r#"
        on 4 ->
            stdio.println("match");
        end
        on _ ->
            stdio.println("fallback");
        end
    "#);
    let guest = compile(&src).expect("one end per arm and final match end");
    assert_eq!(&guest[..8], b"\0asm\x01\0\0\0");
    assert_eq!(guest, compile(&src).unwrap());
}

#[test]
fn braces_and_end_delimiters_are_alternatives() {
    for arms in [
        r#"on 4 -> { stdio.println("hit"); }
           on _ -> { stdio.println("miss"); }"#,
        r#"on 4 -> stdio.println("hit"); end
           on _ -> { stdio.println("miss"); }"#,
        r#"on 4 -> { stdio.println("hit"); }
           on _ -> stdio.println("miss"); end"#,
        r#"on 4 -> stdio.println("hit"); end
           on _ -> stdio.println("miss"); end"#,
    ] {
        let source = fixture(arms);
        assert!(compile(&source).is_ok(), "{source}");
    }
}

#[test]
fn end_delimited_arms_allow_multiple_statements_and_nested_match() {
    let src = fixture(r#"
        on 4 ->
            val local = 4;
            match local over
                on 4 -> { stdio.println("inner"); }
                on _ -> { stdio.println("wrong"); }
            end
            stdio.println("outer");
        end
        on _ -> { stdio.println("fallback"); }
    "#);
    assert!(compile(&src).is_ok(), "{src}");
}

#[test]
fn do_match_uses_the_same_per_arm_delimiter_rules() {
    let src = r#"pub fnc main(): int {
        val n = 4;
        do match n over
            on 4 ->
                stdio.println("do");
            end
            on _ -> {
                stdio.println("other");
            }
        end
        return 42;
    }"#;
    assert!(compile(src).is_ok());
}

#[test]
fn missing_arm_end_and_missing_match_end_fail_closed() {
    for src in [
        fixture(r#"on 4 -> stdio.println("hit");
                    on _ -> { stdio.println("fallback"); }"#),
        r#"pub fnc main(): int {
          match 4 over
            on 4 -> stdio.println("hit"); end
            on _ -> stdio.println("other"); end
        }"#.to_owned(),
        fixture(r#"on 4 -> stdio.println("hit"); end
                    on _ -> stdio.println("fallback");"#),
        fixture(r#"on 4 -> end
                    on _ -> { stdio.println("fallback"); }"#),
    ] {
        let d = compile(&src).expect_err("unbalanced match must not compile");
        assert!(matches!(d.code, "E_SYNTAX" | "E_EOF"), "{src}: {d}");
    }
}

#[test]
fn structural_match_patterns_still_fail_closed_without_guest_artifacts() {
    for arm in [
        "on is ~Point as val p -> { return 42; }",
        "on is ~Point as val p -> return 42; end",
        "on is structural Point as val p -> return 42; end",
    ] {
        let src = fixture(&format!("{arm}\n on _ -> {{ return 0; }}"));
        let d = compile(&src).expect_err("structural match is not lowered yet");
        assert_eq!(d.code, "E_UNSUPPORTED", "{src}: {d}");
    }
}
