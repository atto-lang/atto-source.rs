// Narrow allowlist for the *exact* mutation reproduced in the private CI
// artifact. Never infer that arbitrary timed-out guest programs are safe.
export function classifyTimeout({
  fixture, case_index, initial_seed, per_fixture, rng_before_mutation,
  source_sha256, replay_result,
}) {
  const documented =
    fixture === '139-generator-two-live-locals.atto' &&
    case_index === 1 &&
    initial_seed === 99 &&
    per_fixture === 2 &&
    rng_before_mutation === 1011214194 &&
    source_sha256 === '84cbf5459137cf704e640ae1837dfcdebb671cd81c93b4f3871a222926d26c7f' &&
    replay_result === 'timeout_3000ms';
  if (documented) {
    return {
      status: 'expected_nonterminating_mutant',
      reason: 'The mutated generator assigns count = 1 each iteration while its caller uses limit = 3; the loop condition remains true and the generator continues yielding.',
    };
  }
  return {
    status: 'unclassified_timeout',
    reason: 'New or changed timeout: requires source inspection, runtime investigation, and independent classification.',
  };
}
