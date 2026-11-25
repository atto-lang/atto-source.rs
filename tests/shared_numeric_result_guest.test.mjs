// Source-pinned shared numeric gate for the Result/Error integration draft.
// Expected values are from atto-platform-conformance, not captured from this compiler.
// All eight value cases must actually compile and execute. Three root error
// cases remain explicitly NOT_EXECUTED for certification until the typed
// ordinary guest error ABI has an authenticated observer. Terminal panic_code
// and generic compile failure are never substitutes.
import test from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {mkdtemp,writeFile,readFile,rm} from 'node:fs/promises';
import {spawnSync} from 'node:child_process';
import {tmpdir} from 'node:os';
import {resolve,join} from 'node:path';
import {fileURLToPath} from 'node:url';

const attoc=resolve(process.env.ATTOC||fileURLToPath(new URL('../target/debug/attoc',import.meta.url)));
const fixtures=[
  {
    "id": "checked-i64-quotient",
    "blob": "d7b61afd0fe074395b092aa4abaa08de4961602e",
    "source": "// Candidate guest source: integer truncation and dividend-signed remainder.\n// Target main return 42. Not yet compiler/AOT executed.\npub fnc divide(int lhs, int rhs): int {\n  return lhs / rhs;\n}\npub fnc remainder(int lhs, int rhs): int {\n  return Integer.rem(lhs, rhs);\n}\npub fnc main(): int {\n  val int a = -7;\n  val int b = 3;\n  return divide(a, b) * -21 + (remainder(a, b) + 1);\n}\n",
    "expected": {
      "kind": "value",
      "value": "42"
    }
  },
  {
    "id": "reject-i64-zero-divisor",
    "blob": "6736c0fb356074a753406efe8277aaed724892a3",
    "source": "// Candidate guest source: must fail with division_by_zero (not host exception).\n// Backend may issue a statically proved diagnostic or a typed guest runtime fault.\npub fnc checked(int divisor): int {\n  return 7 / divisor;\n}\npub fnc main(): int {\n  return checked(0);\n}\n",
    "expected": {
      "kind": "error",
      "code": "division_by_zero"
    }
  },
  {
    "id": "reject-i64-min-overflow",
    "blob": "aff2c8170924a3866c8e4f8ec095560148f8b9b5",
    "source": "// Candidate guest source: MIN / -1 must be checked overflow, never MIN.\npub fnc checked(int divisor): int {\n  val int minimum = -9223372036854775808;\n  return minimum / divisor;\n}\npub fnc main(): int {\n  return checked(-1);\n}\n",
    "expected": {
      "kind": "error",
      "code": "integer_overflow"
    }
  },
  {
    "id": "reject-i64-zero-remainder",
    "blob": "a0325c64972b42825f6a82f5c2c941c67dcec7d1",
    "source": "// Candidate guest source: integer remainder by zero is division_by_zero.\npub fnc checked(int divisor): int {\n  return Integer.rem(7, divisor);\n}\npub fnc main(): int {\n  return checked(0);\n}\n",
    "expected": {
      "kind": "error",
      "code": "division_by_zero"
    }
  },
  {
    "id": "trap-division-by-zero",
    "blob": "5eee4119bb0849be397ce0bc38eac66cb8e7b020",
    "source": "// Guest candidate: ordinary DivisionByZeroError must become Result.Err.\n// Expects main() = 42. Not executed on any compiler.\npub fnc checked(int divisor): int {\n  return 7 / divisor;\n}\npub fnc main(): int {\n  val result = trap checked(0);\n  return result.is_err() ? 42 : 1;\n}\n",
    "expected": {
      "kind": "value",
      "value": "42"
    }
  },
  {
    "id": "trap-integer-overflow",
    "blob": "44d0975d36915f7fea115c7ea3cd1bbfa8c62747",
    "source": "// Guest candidate: ordinary ArithmeticOverflowError must become Result.Err.\n// Expects main() = 42. Not executed on any compiler.\npub fnc checked(int increment): int {\n  return 9223372036854775807 + increment;\n}\npub fnc main(): int {\n  val result = trap checked(1);\n  return result.is_err() ? 42 : 1;\n}\n",
    "expected": {
      "kind": "value",
      "value": "42"
    }
  },
  {
    "id": "trap-result-no-doublewrap",
    "blob": "d7e536f97c703621f61959240e4e44f43d7be3fd",
    "source": "// Guest candidate: trap of an existing Result does NOT produce Result<Result<...>,Error>.\n// Expects main() = 42. Not executed on any compiler.\npub fnc main(): int {\n  val result = trap Ok(42);\n  return result.is_ok() ? result.unwrap() : 1;\n}\n",
    "expected": {
      "kind": "value",
      "value": "42"
    }
  },
  {
    "id": "ordered-float-nan",
    "blob": "01079335342d403bcb42d731012588321edbc460",
    "source": "// Candidate guest test: IEEE-754 NaN is unordered for every ordered predicate.\n// Expected main exit value 42. NO actual guest/AOT execution certified.\npub fnc main(): int {\n  val float nan = 0.0 / 0.0;\n  if nan < 1.0 || nan <= 1.0 || nan > 1.0 || nan >= 1.0 then\n    return 1;\n  fi\n  return 42;\n}\n",
    "expected": {
      "kind": "value",
      "value": "42"
    }
  },
  {
    "id": "ordered-float-signed-zero",
    "blob": "8030687c74d7243d8275451f89b96ab9757534c0",
    "source": "// Candidate guest test: IEEE-754 -0.0 and +0.0 compare equal in ordering.\n// Expected main exit value 42. NO actual guest/AOT execution certified.\npub fnc main(): int {\n  val float minus_zero = -0.0;\n  if minus_zero < 0.0 || minus_zero > 0.0 then\n    return 1;\n  fi\n  if minus_zero <= 0.0 && minus_zero >= 0.0 then\n    return 42;\n  fi\n  return 2;\n}\n",
    "expected": {
      "kind": "value",
      "value": "42"
    }
  },
  {
    "id": "ordered-int-float-precision",
    "blob": "2d57f49dd16760f94bef1525b95dfc20ba654a9a",
    "source": "// Candidate guest test: avoid lossy binary64 comparison beyond 2^53.\n// Expected main exit value 42. No implied mixed-type coercion or AOT certification.\npub fnc main(): int {\n  val int exact = 9007199254740993;\n  val float rounded = 9007199254740992.0;\n  if exact > rounded then\n    return 42;\n  fi\n  return 1;\n}\n",
    "expected": {
      "kind": "value",
      "value": "42"
    }
  },
  {
    "id": "equality-numeric-exact",
    "blob": "b09deb61e5ead17261174200cb409ed46566f053",
    "source": "// Candidate guest program: exact mixed i64/float equality, NaN, signed zero.\n// Expected main() = 42; NOT_EXECUTED on any guest compiler or AOT backend.\npub fnc main(): int {\n  val int exact = 9007199254740993;\n  val float rounded = 9007199254740992.0;\n  val float nan = 0.0 / 0.0;\n  val float negative_zero = -0.0;\n  if exact eq rounded || rounded eq exact then\n    return 1;\n  fi\n  if nan eq nan || negative_zero neq 0.0 then\n    return 2;\n  fi\n  if exact neq rounded && nan neq nan && negative_zero eq 0.0 then\n    return 42;\n  fi\n  return 3;\n}\n",
    "expected": {
      "kind": "value",
      "value": "42"
    }
  }
];
test('exact upstream numeric manifest fixture identities are preserved',()=>{
  assert.equal(fixtures.length,11);
  assert.equal(new Set(fixtures.map(c=>c.id)).size,11);
  for(const c of fixtures){
    const bytes=Buffer.from(c.source);
    assert.equal(createHash('sha1').update('blob '+bytes.length+'\0').update(bytes).digest('hex'),c.blob,c.id);
  }
  assert.equal(fixtures.filter(c=>c.expected.kind==='value').length,8);
  assert.equal(fixtures.filter(c=>c.expected.kind==='error').length,3);
});
for(const c of fixtures.filter(c=>c.expected.kind==='value')){
  test('actual native attoc -> Wasm: '+c.id,async()=>{
    const dir=await mkdtemp(join(tmpdir(),'atto-shared-numeric-'));
    try{
      const input=join(dir,'guest.atto'),out=join(dir,'guest.wasm');
      await writeFile(input,c.source,{flag:'wx'});
      const r=spawnSync(attoc,[input,'-o',out],{encoding:'utf8',timeout:30000,maxBuffer:65536});
      assert.ifError(r.error);
      assert.equal(r.signal,null,c.id);
      assert.equal(r.status,0,c.id+': '+r.stderr);
      const bytes=await readFile(out);
      assert.ok(bytes.length>8&&bytes.length<=16*1024*1024);
      const mod=new WebAssembly.Module(bytes);
      assert.deepEqual(WebAssembly.Module.imports(mod),[],c.id);
      const instance=new WebAssembly.Instance(mod,{});
      assert.ok(instance.exports.panic_code instanceof WebAssembly.Global);
      assert.equal(instance.exports.main(),BigInt(c.expected.value),c.id);
      assert.equal(instance.exports.panic_code.value,0,c.id);
    }finally{await rm(dir,{recursive:true,force:true});}
  });
}
for(const c of fixtures.filter(c=>c.expected.kind==='error'))
  test('typed root error receipt required: '+c.id,{todo:'ordinary Error ABI observer not implemented; NOT_EXECUTED'},()=>{});
// In particular a green positive-case run alone is not 11/11 parity.
