use oreslang_source_rs::frontend;

fn rejected_as_domain(source: &str) {
    match frontend::parse(source) {
        Err(error) => assert_eq!(
            error.class, "execution_domain_unsupported",
            "{source}: {error}"
        ),
        Ok(_) => panic!("{source}: execution-domain source was admitted"),
    }
}

#[test]
fn domain_words_are_reserved_in_every_position() {
    for word in frontend::EXECUTION_DOMAIN_WORDS {
        rejected_as_domain(&format!("static fnc {word}() -> int {{ return 1; }}"));
        rejected_as_domain(&format!(
            "static fnc f(int {word}) -> int {{ return {word}; }}"
        ));
        rejected_as_domain(&format!(
            "static fnc f() -> int {{ return {word} {{ 1 }}; }}"
        ));
        rejected_as_domain(&format!("static {word} fnc f() -> int {{ return 1; }}"));
        rejected_as_domain(&format!(
            "static fnc f() -> int {{ return {word} task {{ 1 }}; }}"
        ));
        rejected_as_domain(&format!(
            "static fnc f() -> int {{ const {word} = 1; return {word}; }}"
        ));
    }
}

#[test]
fn task_and_nb_blocks_are_rejected_as_execution_tasks() {
    rejected_as_domain("static fnc f() -> int { return task { 1 }; }");
    rejected_as_domain("static fnc f() -> int { return nb { 1 }; }");
}

#[test]
fn comments_and_longer_identifiers_stay_ordinary() {
    let source = "// gpu { cpu { quantum } } task { }\n\
                  static fnc compute_gpu(int task) -> int {\n\
                  /* nb { gpu task { 1 } } */\n\
                  return task + 1;\n\
                  }\n";
    frontend::parse(source).expect("ordinary source must parse");
}
