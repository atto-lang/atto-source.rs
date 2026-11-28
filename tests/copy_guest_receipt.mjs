// Host-only receipt validation; never substitutes for executing the compiler.
export function validRejection(input, expected, run, artifact) {
  if (run.error || run.signal !== null || run.status !== 1 || artifact) return false;
  if (typeof run.stderr !== 'string' || !/^E_[A-Z0-9_]+$/.test(expected)) return false;
  const lines = run.stderr.trimEnd().split(/\r?\n/);
  const prefix = input + ':';
  if (lines.length !== 1 || !lines[0].startsWith(prefix)) return false;
  const d = /^ ?(\d+):(\d+): (E_[A-Z0-9_]+): .+$/.exec(lines[0].slice(prefix.length));
  return d !== null && d[3] === expected;
}
