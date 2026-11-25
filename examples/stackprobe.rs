//! Worst-case stack probe: compiles maximal programs inside each budget on a
//! thread with a fixed stack. `cargo run --example stackprobe [KiB]`.
fn main() {
    let kib: usize = std::env::args().nth(1).map_or(2048, |a| a.parse().unwrap());
    let h = atto_compiler::parser::MAX_TREE_HEIGHT as usize;
    let d = atto_compiler::parser::MAX_DEPTH as usize;
    let chain = |n: usize| vec!["1"; n].join(" + ");
    let cases = [
        ("operator chain", format!("pub fnc main(): int {{ return {}; }}", chain(h - 3))),
        (
            "parens",
            format!(
                "pub fnc main(): int {{ return {}1{}; }}",
                "(".repeat(d / 2 - 2),
                ")".repeat(d / 2 - 2)
            ),
        ),
        (
            "blocks + chain",
            format!(
                "pub fnc main(): int {{ {}return {};{} }}",
                "block { ".repeat(d / 2 - 4),
                chain(h - d / 2 - 8),
                " }".repeat(d / 2 - 4)
            ),
        ),
        ("elif arms", {
            let arms: Vec<String> = (1..h - 4).map(|i| format!("elif x eq {i} then return {i};")).collect();
            format!(
                "pub fnc main(): int {{\nval x = 1;\nif x eq 0 then return 0;\n{}\nfi\nreturn 0;\n}}",
                arms.join("\n")
            )
        }),
        ("match arms", {
            let arms: Vec<String> = (1..h - 4).map(|i| format!("{i} -> {{ return {i}; }}")).collect();
            format!(
                "pub fnc main(): int {{\nval x = 1;\nmatch x\n{}\n_ -> {{ return 0; }}\nend\n}}",
                arms.join("\n")
            )
        }),
        (
            "string concat chain",
            format!("pub fnc main(): int {{ val s = {}; return 0; }}", vec!["\"a\""; h - 3].join(" + ")),
        ),
        (
            "float chain",
            format!("pub fnc main(): int {{ val f = {}; return 0; }}", vec!["1.5"; h - 3].join(" * ")),
        ),
        (
            "member chain",
            format!("pub fnc main(): int {{ val a = 1; return a{}; }}", ".b".repeat(h - 4)),
        ),
        (
            "method chain",
            format!("pub fnc main(): int {{ val a = 1; return a{}; }}", ".b()".repeat(h - 4)),
        ),
        (
            "ternaries",
            format!("pub fnc main(): int {{ val t = true; return {}0; }}", "t ? 1 : ".repeat(d / 2 - 4)),
        ),
    ];
    for (name, src) in cases {
        let r = std::thread::Builder::new()
            .stack_size(kib * 1024)
            .spawn(move || atto_compiler::compile(&src).map(|_| ()).map_err(|d| d.render()))
            .unwrap()
            .join()
            .unwrap();
        println!("{name:>15} @ {kib} KiB: {r:?}");
    }
}
