//! Exact-source linked-guest runtime smoke: emits two self-contained Wasm
//! binaries for an independent Node WebAssembly.instantiate() execution check.
use atto_compiler::linker::{compile_linked, LinkUnit};

fn main() {
    let mut args = std::env::args().skip(1);
    let public_out = args.next().expect("public guest output path");
    let package_out = args.next().expect("package guest output path");
    assert!(args.next().is_none(), "exactly two output paths are required");

    let root = r#"
use namespace web;
import module Math from "./math.atto";
pub fnc main(): int { return Math.double(21); }
"#;
    let public = r#"
use namespace math_lib;
export pub module Math;
define module Math as
    pub fnc double(int x): int { return x * 2; }
    fnc secret(): int { return 91; }
end
"#;
    let package_only = public.replace("export pub module Math;", "export pub[pkg] module Math;");
    let units = |dep: &str, package: &str| {
        let module = [
            LinkUnit { path: "src/main.atto", package: "web.application", source: root },
            LinkUnit { path: "src/math.atto", package, source: dep },
        ];
        compile_linked("src/main.atto", &module).expect("link compiled guest")
    };

    std::fs::write(public_out, units(public, "third.party"))
        .expect("write independent public linked guest");
    std::fs::write(package_out, units(&package_only, "web.application"))
        .expect("write independent same-package linked guest");
}
