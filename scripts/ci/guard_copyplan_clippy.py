#!/usr/bin/env python3
"""Fail a CopyPlan CI admission if Clippy reports any NEW error.

The SHA-pinned canonical copy verifier is deliberately never rewritten just
to silence style warnings. Only 13 known pre-existing diagnostic locations
are temporarily tolerated; this does NOT make whole-project Clippy green.
"""
import json
import sys

# (source path, lint id): pin the precise existing debt. The same location
# can be emitted in multiple Cargo targets; counting deduplicates targets.
KNOWN = {
    ("src/any_type_lower.rs", "clippy::collapsible_if"),
    ("src/shape_lower.rs", "unused_imports"),
    ("src/routine_lower.rs", "clippy::int_plus_one"),
    ("src/target_syntax.rs", "clippy::collapsible_if"),
    ("src/module_lower.rs", "unused_variables"),
    ("src/nominal_lower.rs", "unused_variables"),
    ("src/visibility_lower.rs", "unused_variables"),
    ("src/copy_graph_v1.rs", "clippy::needless_return"),
    ("src/io_shorthand.rs", "clippy::needless_range_loop"),
    ("src/nominal_lower.rs", "clippy::needless_range_loop"),
    ("src/routine_lower.rs", "clippy::needless_range_loop"),
    ("src/target_syntax.rs", "clippy::needless_range_loop"),
}
MAX_BASELINE_DIAGNOSTICS = 13

found = set()
unexpected = set()
seen_json = False
for line in sys.stdin:
    try:
        event = json.loads(line)
    except json.JSONDecodeError:
        continue
    if event.get("reason") != "compiler-message":
        continue
    seen_json = True
    msg = event.get("message", {})
    if msg.get("level") != "error":
        continue
    code_obj = msg.get("code")
    code = (code_obj or {}).get("code") or ""
    if not code and msg.get("message", "").startswith("could not compile"):
        continue
    spans = msg.get("spans", [])
    primary = next((s for s in spans if s.get("is_primary")), None)
    if not primary and spans:
        primary = spans[0]
    path = (primary or {}).get("file_name", "<unknown>")
    key = (path.replace("\\", "/"), code)
    # Count source location, not just lint family, so recurring new errors
    # in legacy modules cannot quietly consume more baseline allowance.
    line_no = (primary or {}).get("line_start", 0)
    record = (key[0], key[1], line_no)
    found.add(record)
    if key not in KNOWN:
        unexpected.add(record)

if not seen_json:
    print("COPYPLAN_LINT_GATE=FAIL missing Cargo JSON diagnostics", file=sys.stderr)
    sys.exit(2)
if unexpected or len(found) > MAX_BASELINE_DIAGNOSTICS:
    print("COPYPLAN_LINT_GATE=FAIL new or excess errors:", file=sys.stderr)
    for item in sorted(unexpected or found):
        print(f"  {item}", file=sys.stderr)
    sys.exit(1)
print(f"COPYPLAN_LINT_GATE=PASS new_errors=0 legacy_errors={len(found)}")
if found:
    print("NOTE: full-project strict Clippy remains NOT PASS.")
