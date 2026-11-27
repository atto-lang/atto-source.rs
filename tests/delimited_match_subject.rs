//! Standalone delimited match headers: single-subject frontend slice.
//! These tests use the production compiler, not a shadow parser.
use atto_compiler::compile;

#[test]
fn scalar_and_grouped_subjects_compile() {
    for source in [
        "pub fnc main(): int { const int v = 7; return match |v| over on 0 -> 1, on _ -> v * 6 end; }",
        "pub fnc main(): int { return match |(2 | 4)| over on 6 -> 42, on _ -> 0 end; }",
        "pub fnc main(): int { return match |true || false| over on true -> 42, on false -> 0 end; }",
        "fnc subject(): int { return 7; } pub fnc main(): int { return match |subject()| over on 7 -> 42, on _ -> 0 end; }",
        "pub fnc main(): int { match |7| over on 7 -> { return 42; } on _ -> { return 0; } end }",
        "pub fnc main(): int { return match 7 over on 7 -> 42, on _ -> 0 end; }",
        "pub fnc main(): int { return (2 | 4); }",
    ] {
        assert!(compile(source).is_ok(), "{source}: {:?}", compile(source).err());
    }
}

#[test]
fn malformed_or_unimplemented_headers_reject() {
    for source in [
        "pub fnc main(): int { return match | | over on _ -> 42 end; }",
        "pub fnc main(): int { return match |7 over on _ -> 42 end; }",
        "pub fnc main(): int { return match |2 | 4| over on _ -> 42 end; }",
        "pub fnc main(): int { return match |2, 4| over on _ -> 42 end; }",
        "pub fnc main(): int { return match |2,| over on _ -> 42 end; }",
        "pub fnc main(): int { return match |missing_subject| over on _ -> 42 end; }",
        "pub fnc main(): int { return 7 |> match |x| over on _ -> x end; }",
        "pub fnc main(): int { match |7| on _ -> { return 42; } end }",
    ] {
        assert!(compile(source).is_err(), "unexpected admission: {source}");
    }
    assert!(compile("pub fnc main(): int { return 42; }").is_ok());
}
