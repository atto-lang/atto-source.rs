#!/usr/bin/env python3
"""Run the pinned Rust scalar source -> effect IR -> analyzer conformance gate.

Requires both *locally checked out* repositories and Cargo for full execution.
Never fetches private repos, sends their source to another organization, or
claims that a matching fixture authenticates untrusted user-supplied JSON.
"""
import argparse
import hashlib
import json
import pathlib
import subprocess
import sys
import tempfile

SOURCE_SHA256 = "e35a1c32239212a8ce3a496e8381c956bd4292b6db6bac97dfa4a6830e0b1133"
EFFECT_SHA256 = "8ee1adbf6faef704c1eb6d2b2075c3b2d723097037f646f9e6e518a69d005463"
ROOT = pathlib.Path(__file__).resolve().parents[1]


def require(condition, message):
    if not condition:
        raise AssertionError(message)


def run_cargo(repo, arguments, expected_code):
    result = subprocess.run(
        ["cargo", "run", "--locked", *arguments], cwd=repo,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        check=False, timeout=240,
    )
    require(result.returncode == expected_code,
            "unexpected Cargo status\ncommand: " + str(arguments)
            + "\nstdout:\n" + result.stdout + "\nstderr:\n" + result.stderr)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--analyzer-dir", type=pathlib.Path,
                        help="Local checkout of oreslang-static-analysis.rs")
    parser.add_argument("--fixture-only", action="store_true",
                        help="Check pinned fixture without running Cargo")
    args = parser.parse_args()

    source = ROOT / "examples/answer.ores"
    golden = (ROOT / "examples/scalar-effect-ir-v1.json").read_text().strip()
    require(hashlib.sha256(source.read_bytes()).hexdigest() == SOURCE_SHA256,
            "golden source hash does not match")
    require(hashlib.sha256(golden.encode()).hexdigest() == EFFECT_SHA256,
            "golden effect-IR hash does not match")
    expected = json.loads(golden)
    require(expected["functions"] == [{"name": "answer",
        "operations": [{"kind": "compute", "units": 1}] * 3}],
        "unexpected scalar fixture structure")
    if args.fixture_only:
        print("scalar source and effect IR golden hashes: PASS (fixture-only)")
        return 0

    require(args.analyzer_dir is not None,
            "full test needs --analyzer-dir <local checkout>")
    analyzer = args.analyzer_dir.resolve()
    require((analyzer / "crates/ores-analysis/src/lib.rs").is_file(),
            "not an analyzer checkout")
    sibling = (analyzer / "examples/rust-scalar-expected.json").read_text().strip()
    require(sibling == golden, "cross-repo golden fixture drift")
    with tempfile.TemporaryDirectory() as scratch:
        effect_path = pathlib.Path(scratch) / "effects.json"
        run_cargo(ROOT, ["--bin", "oresc", "--", "--emit-effect-ir",
                         str(source), str(effect_path)], 0)
        actual = effect_path.read_text()
        require(actual == golden, "compiler output differs from golden IR")

        accepted = run_cargo(analyzer, ["-p", "ores-analyze", "--",
                             str(effect_path), "hard"], 0)
        report = json.loads(accepted.stdout)
        require(report["assurance_level"] == "untrusted_effect_ir_model_only",
                "analysis must not claim hardware verification")
        summary = report["functions"]["answer"]
        require(not report["diagnostics"], "unexpected analyzer diagnostics")
        require(summary["allocations_upper_bound"] == 0
                and summary["copies_upper_bound"] == 0
                and summary["operations_upper_bound"] == 3,
                "scalar effect summary does not match generated MIR work")

        # Mutation/negative controls must fail closed, not print a green model.
        bad = {"schema_version": 1, "functions": [{"name": "answer",
               "operations": [{"kind": "indirect_call"}]}]}
        effect_path.write_text(json.dumps(bad))
        rejected = run_cargo(analyzer, ["-p", "ores-analyze", "--",
                             str(effect_path), "hard"], 1)
        require(any(d["code"] == "ORES-SA103"
                    for d in json.loads(rejected.stdout)["diagnostics"]),
                "missing unresolved-call rejection")
        bad["functions"][0]["operations"] = [{"kind": "compute"}]
        effect_path.write_text(json.dumps(bad))
        run_cargo(analyzer, ["-p", "ores-analyze", "--",
                   str(effect_path), "hard"], 2)

        bad["functions"][0]["operations"] = [{"kind": "compute", "units": 0}]
        effect_path.write_text(json.dumps(bad))
        zero = run_cargo(analyzer, ["-p", "ores-analyze", "--",
                         str(effect_path), "hard"], 1)
        require(any(d["code"] == "ORES-SA005"
                    for d in json.loads(zero.stdout)["diagnostics"]),
                "zero-cost compute was not rejected")
    print("source -> MIR effect exporter -> analyzer + negatives: PASS (model-only)")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (AssertionError, OSError, ValueError, subprocess.TimeoutExpired) as exc:
        print("scalar bridge verification FAILED:", exc, file=sys.stderr)
        sys.exit(1)
