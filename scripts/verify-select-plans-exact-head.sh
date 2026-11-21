#!/usr/bin/env bash
# Exact-head PRIVATE source verification. Never mirrors, uploads, or exports
# repository files to a different organization or CI carrier.
set -euo pipefail

if [[ $# != 1 || ! "$1" =~ ^[0-9a-f]{40}$ ]]; then
  echo "usage: bash scripts/verify-select-plans-exact-head.sh <exact-40-character-commit-sha>" >&2
  exit 2
fi
expected="$1"
actual="$(git rev-parse HEAD)"
tree="$(git rev-parse HEAD^{tree})"
if [[ "$actual" != "$expected" ]]; then
  echo "REFUSED: checked-out commit $actual differs from required $expected" >&2
  exit 2
fi
if [[ -n "$(git status --porcelain)" ]]; then
  echo "REFUSED: working tree is dirty; tests would not prove the exact head" >&2
  exit 2
fi
command -v cargo >/dev/null || { echo "cargo/rustup required" >&2; exit 2; }
command -v rustup >/dev/null || { echo "pinned Rust toolchain requires rustup" >&2; exit 2; }
command -v node >/dev/null || { echo "Node.js 22+ required" >&2; exit 2; }
node -e 'if (Number(process.versions.node.split(".")[0]) < 22) process.exit(2)'
if [[ -z "${CHROME_BIN:-}" ]]; then
  CHROME_BIN="$(command -v google-chrome || command -v chromium || true)"
fi
if [[ -z "$CHROME_BIN" || ! -x "$CHROME_BIN" ]]; then
  echo "Chromium/Google Chrome is required; supply executable CHROME_BIN" >&2
  exit 2
fi
export CHROME_BIN

rustup toolchain install 1.85.1 --profile minimal --target wasm32-unknown-unknown
rustup component add rustfmt clippy --toolchain 1.85.1
cargo +1.85.1 fmt --all -- --check
cargo +1.85.1 clippy --locked --all-targets -- -D warnings
bash tests/private-ci-export-policy-guard-test.sh
bash scripts/private-ci-export-policy-guard.sh .github/private-ci-approved-repositories.txt
cargo +1.85.1 test --locked --test v2_select_plans
cargo +1.85.1 test --locked
cargo +1.85.1 build --locked --bin attoc
node tests/node-v2-select-plans-guest.mjs
cargo +1.85.1 build --locked --release --bin attoc
cargo +1.85.1 rustc --locked --release --lib --target wasm32-unknown-unknown --crate-type cdylib -- -C link-arg=--max-memory=67108864
ATTOC=target/release/attoc node tests/conformance.mjs
node tests/browser-e2e.mjs

# Receipt records only identifiers and artifact digest; it contains no source,
# token, package contents, or unapproved cross-organization transfer.
REQUIRED_HEAD="$expected" REQUIRED_TREE="$tree" node --input-type=module <<'NODE'
import {readFileSync} from 'node:fs';
import {createHash} from 'node:crypto';
import {execFileSync} from 'node:child_process';
const bytes=readFileSync('target/wasm32-unknown-unknown/release/atto_compiler.wasm');
const mod=new WebAssembly.Module(bytes);
if(WebAssembly.Module.imports(mod).length!==0) throw new Error('compiler imports host capabilities');
if(execFileSync('git',['rev-parse','HEAD'],{encoding:'utf8'}).trim()!==process.env.REQUIRED_HEAD) throw new Error('head drift');
if(execFileSync('git',['rev-parse','HEAD^{tree}'],{encoding:'utf8'}).trim()!==process.env.REQUIRED_TREE) throw new Error('tree drift');
if(execFileSync('git',['status','--porcelain'],{encoding:'utf8'}).trim()) throw new Error('dirty tree');
console.log(JSON.stringify({
  schema:'atto.select-plan-local-execution-receipt.v1',
  head:process.env.REQUIRED_HEAD,
  tree:process.env.REQUIRED_TREE,
  rust:'1.85.1',
  node:process.version,
  wasm_sha256:createHash('sha256').update(bytes).digest('hex'),
  wasm_bytes:bytes.length,
  tests:'cargo-fmt,cargo-test,compiled-node-guest,wasm32-conformance,chromium-guest',
  result:'PASS'
},null,2));
NODE
