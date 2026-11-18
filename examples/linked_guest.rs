//! Exact-source linked-guest runtime smoke: emits four self-contained Wasm
//! binaries for an independent Node WebAssembly.instantiate() execution check.
use atto_compiler::linker::{compile_linked, LinkUnit};
use atto_compiler::package_resolver::compile_package;

fn main() {
    let mut args = std::env::args().skip(1);
    let public_out = args.next().expect("public guest output path");
    let package_out = args.next().expect("package guest output path");
    let deep_out = args.next().expect("three-file guest output path");
    let filesystem_out = args.next().expect("filesystem package guest output path");
    assert!(args.next().is_none(), "exactly four output paths are required");

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
            LinkUnit {
                path: "src/main.atto",
                package: "web.application",
                source: root,
            },
            LinkUnit {
                path: "src/math.atto",
                package,
                source: dep,
            },
        ];
        compile_linked("src/main.atto", &module).expect("link compiled guest")
    };

    std::fs::write(public_out, units(public, "third.party")).expect("write independent public linked guest");
    std::fs::write(package_out, units(&package_only, "web.application")).expect("write independent same-package linked guest");

    // Full entry -> Orders -> Math chain. The Math package-private export
    // must be checked against Orders, not just against the entry.
    let multi_main = r#"
use namespace web;
import module Orders from "+lib/orders.atto";
pub fnc main(): int { return Orders.total(21); }
"#;
    let orders = r#"
use namespace sales;
import module Math from "./math.atto";
export pub module Orders;
define module Orders as
    pub fnc total(int x): int { return Math.double(x); }
end
"#;
    let math = package_only.as_str();
    let multi = [
        LinkUnit {
            path: "src/main.atto",
            package: "web.application",
            source: multi_main,
        },
        LinkUnit {
            path: "lib/orders.atto",
            package: "web.application",
            source: orders,
        },
        LinkUnit {
            path: "lib/math.atto",
            package: "web.application",
            source: math,
        },
    ];
    std::fs::write(
        deep_out,
        compile_linked("src/main.atto", &multi).expect("three-file deep linked guest"),
    )
    .expect("write deep multi-hop guest");

    // Prove the filesystem entrypoint independently authenticates its
    // own manifest and loads only canonical paths within its package root.
    let package_dir = std::env::temp_dir().join(format!("atto-guest-package-{}", std::process::id()));
    std::fs::create_dir_all(package_dir.join("src")).expect("create entry directory");
    std::fs::create_dir_all(package_dir.join("lib")).expect("create module directory");
    std::fs::write(
        package_dir.join(".atto.pkg.toml"),
        "[package]\nname=\"web.application\"\nversion=\"1.0.0\"\n",
    )
    .expect("write bounded package manifest");
    std::fs::write(package_dir.join("src/main.atto"), multi_main).expect("write entry");
    std::fs::write(package_dir.join("lib/orders.atto"), orders).expect("write orders");
    std::fs::write(package_dir.join("lib/math.atto"), math).expect("write math");
    let wasm = compile_package(&package_dir, "src/main.atto").expect("compile manifest-authenticated local package");
    std::fs::remove_dir_all(&package_dir).expect("clean source fixture tree");
    std::fs::write(filesystem_out, wasm).expect("write package-resolved guest");
}
