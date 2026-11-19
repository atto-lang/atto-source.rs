// Render a binary64 value exactly like Java's Double.toString (JDK 19+, which
// uses the shortest decimal that round-trips, as ECMAScript does). Java uses
// plain notation for 1e-3 <= |x| < 1e7, always with at least one fraction
// digit, and computerized scientific notation "d.dddE±n" otherwise.
export function formatJavaDouble(x) {
  if (Number.isNaN(x)) return 'NaN';
  if (x === Infinity) return 'Infinity';
  if (x === -Infinity) return '-Infinity';
  if (x === 0) return Object.is(x, -0) ? '-0.0' : '0.0';
  const abs = Math.abs(x);
  if (abs >= 1e-3 && abs < 1e7) {
    const s = String(x); // ECMAScript shortest round-trip, plain in this range
    return s.includes('.') ? s : s + '.0';
  }
  const [mantissa, exponent] = x.toExponential().split('e'); // shortest digits
  return (mantissa.includes('.') ? mantissa : mantissa + '.0') + 'E' + exponent.replace('+', '');
}
