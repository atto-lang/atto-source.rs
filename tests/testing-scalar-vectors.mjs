// Compiler-specific test ORACLES (not shared compiler library source).
// Reused by the native attoc and independently compiled wasm32-compiler paths.
// 0 = assertion passed; 1 = assertion failed.
export const SCALAR_TEST_CASES = Object.freeze([
  ['equal-pass', 'equal_int(42, 42)', 0n],
  ['equal-fail', 'equal_int(41, 42)', 1n],
  ['not-equal-pass', 'not_equal_int(41, 42)', 0n],
  ['not-equal-fail', 'not_equal_int(42, 42)', 1n],
  ['greater-pass', 'greater_than_int(42, 41)', 0n],
  ['greater-equal-fail', 'greater_than_int(42, 42)', 1n],
  ['greater-below-fail', 'greater_than_int(41, 42)', 1n],
  ['less-pass', 'less_than_int(41, 42)', 0n],
  ['less-equal-fail', 'less_than_int(42, 42)', 1n],
  ['less-above-fail', 'less_than_int(43, 42)', 1n],
  ['range-lower', 'in_range_int(40, 40, 42)', 0n],
  ['range-upper', 'in_range_int(42, 40, 42)', 0n],
  ['range-middle', 'in_range_int(41, 40, 42)', 0n],
  ['range-below', 'in_range_int(39, 40, 42)', 1n],
  ['range-above', 'in_range_int(43, 40, 42)', 1n],
  ['range-inverted', 'in_range_int(41, 42, 40)', 1n],
  ['range-i64-min', 'in_range_int(-9223372036854775808, -9223372036854775808, 9223372036854775807)', 0n],
  ['range-i64-max', 'in_range_int(9223372036854775807, -9223372036854775808, 9223372036854775807)', 0n],
  ['combined-pass', 'combine_status(equal_int(42, 42), less_than_int(41, 42))', 0n],
  ['combined-fail', 'combine_status(equal_int(42, 42), less_than_int(42, 41))', 1n],
  ['combined-two-fail', 'combine_status(equal_int(41, 42), greater_than_int(42, 42))', 1n],
]);

export const SCALAR_TEST_NEGATIVES = Object.freeze([
    ['legacy-suffix','import fnc equal_int from "std/testing_scalar.ores"; pub fnc main():int{return equal_int(42,42);}'],
    ['path-traversal','import fnc equal_int from "std/../testing_scalar"; pub fnc main():int{return equal_int(42,42);}'],
    ['unknown-library','import fnc equal_int from "std/unknown"; pub fnc main():int{return equal_int(42,42);}'],
    ['unknown-export','import fnc missing from "std/testing_scalar"; pub fnc main():int{return missing(42,42);}'],
    ['duplicate-binding','import fnc equal_int as foo, not_equal_int as foo from "std/testing_scalar"; pub fnc main():int{return foo(42,42);}'],
    ['collision','import fnc equal_int from "std/testing_scalar"; pub fnc equal_int(int x,int y):int{return 1;} pub fnc main():int{return 0;}'],
    ['relative-file-without-loader','import fnc equal_int from "./stdlib/testing_scalar.atto"; pub fnc main():int{return equal_int(42,42);}'],
    ['forged-internal-namespace','import fnc equal_int from "std/testing_scalar"; pub fnc main():int{return __atto_file_import_0.equal_int(42,42);}'],
  ]);

export function scalarTestProgram(expression) {
  return 'import fnc equal_int, not_equal_int, greater_than_int, less_than_int, ' +
    'in_range_int, combine_status from "std/testing_scalar";\n' +
    'pub fnc main(): int { return ' + expression + '; }\n';
}
