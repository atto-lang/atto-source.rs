import assert from 'node:assert/strict';
import { classifyTimeout } from './fuzz-timeout-classifier.mjs';

const knownSource = `// expect-exit: 10
// java: llvm-design
// Both counter and running sum survive every suspension.
fnc *gen running_totals(int limit): Generator<int> {
  let count = 0;
  let total = 0;
  while count < limit do
    count = 1;
    total += count;
    yield total;
  done
}
pub fnc main(): int {
  let sum = 0;
  for value in running_totals(3) do
    sum += value;
  done
  return sum; // 1 + 3 + 6 = 10
}
`;

const known = {
  fixture:'139-generator-two-live-locals.atto',
  case_index:1,initial_seed:99,per_fixture:2,
  rng_before_mutation:1011214194,
  source_sha256:'84cbf5459137cf704e640ae1837dfcdebb671cd81c93b4f3871a222926d26c7f',
  wasm_sha256:'100dcc9c05473cd4c2fbd95733aa3ced01d948290e3ca989479428e7eddfac6a',
  source:knownSource,
  replay_result:'timeout_3000ms',
};
assert.equal(classifyTimeout(known).status,'expected_nonterminating_mutant');
assert.match(classifyTimeout(known).reason,/count = 1/);

for (const override of [
  {fixture:'different-generator.atto'}, {case_index:2},
  {initial_seed:12}, {per_fixture:3}, {rng_before_mutation:99},
  {source_sha256:'0000000000000000000000000000000000000000000000000000000000000000'},
  {replay_result:'completed_3000ms'},
  {replay_result:'trap_3000ms'},
  {source:knownSource.replace('count = 1;', 'count = 2;')},
  {source:'', source_sha256:known.source_sha256},
  {source:undefined},
  {wasm_sha256:'0000000000000000000000000000000000000000000000000000000000000000'},
  {wasm_sha256:undefined},
]) {
  const result=classifyTimeout({...known,...override});
  assert.equal(result.status,'unclassified_timeout',
    'a timeout is exempt only with its exact reproducible source and conditions');
}
assert.equal(classifyTimeout({}).status,'unclassified_timeout');
console.log('PASS strict mutation-timeout classification (15)');
