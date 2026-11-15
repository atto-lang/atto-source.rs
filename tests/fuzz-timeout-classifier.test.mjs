import assert from 'node:assert/strict';
import { classifyTimeout } from './fuzz-timeout-classifier.mjs';

const known = {
  fixture:'139-generator-two-live-locals.atto',
  case_index:1,initial_seed:99,per_fixture:2,
  rng_before_mutation:1011214194,
  source_sha256:'84cbf5459137cf704e640ae1837dfcdebb671cd81c93b4f3871a222926d26c7f',
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
]) {
  const result=classifyTimeout({...known,...override});
  assert.equal(result.status,'unclassified_timeout',
    'a timeout is exempt only with its exact reproducible source and conditions');
}
assert.equal(classifyTimeout({}).status,'unclassified_timeout');
console.log('PASS strict mutation-timeout classification (10)');
