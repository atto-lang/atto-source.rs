//! Build the actual user-facing namespace/import example as a Wasm guest.
//! Independent Node execution verifies printed output and return value.
use atto_compiler::linker::{compile_linked, LinkUnit};

fn main() {
    let mut args = std::env::args().skip(1);
    let output = args.next().expect("output Wasm path");
    assert!(args.next().is_none(), "expected exactly one output Wasm path");

    let entry = r#"use namespace storefront;
import module math from "./math.atto";

pub fnc main(): int {
  stdio.println(math.square(7)); // 49, if the companion module is available
  return 0;
}"#;
    let math = r#"use namespace arithmetic;
define pub module math as
    pub fnc square(int x): int { return x * x; }
end"#;

    let guest = compile_linked(
        "src/main.atto",
        &[
            LinkUnit {
                path: "src/main.atto",
                package: "storefront@1.0.0",
                source: entry,
            },
            LinkUnit {
                path: "src/math.atto",
                package: "storefront@1.0.0",
                source: math,
            },
        ],
    )
    .expect("compile namespace + lowercase math module + stdio.println");
    std::fs::write(output, guest).expect("write linked guest Wasm");
}
