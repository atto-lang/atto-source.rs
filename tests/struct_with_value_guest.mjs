// Real compiler -> guest Wasm. No host-side interpretation of Atto source.
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, stat, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const decl = `type Point = struct {
    int x;
    int y;
} with {
    constructor(int x, int y) { self.x = x; self.y = y; }
    mtd int • sum() { return self.x + self.y; }
    mtd int <- product() { return self.x * self.y; }
    mtd is_origin() : bool { return self.x == 0 && self.y == 0; }
}`;
const main = `pub fnc main(): int {
    let p = Point(10, 20);
    stdio.println(p.sum());
    stdio.println(p.product());
    stdio.println(p.is_origin());
    return 0;
}`;
const chained = decl.replace('} with {', '} with')
    .replace('    mtd', 'with mtd').replaceAll('\n    mtd', '\nwith mtd')
    .replace(/\n\}$/, '');
const two = `pub fnc main(): int {
    let p = Point(10, 20);
    let q = Point(p.sum(), 2);
    stdio.println(p.sum());
    stdio.println(q.product());
    stdio.println(q.is_origin());
    return 0;
}`;
const overloaded = decl.replace('    mtd int • sum()', `    constructor(bool flag, int y) { self.x = flag ? 1 : 0; self.y = y; }
    mtd int • pick(int n) { return self.x + n; }
    mtd pick(bool b) : int { return b ? self.y : self.x; }
    mtd int • sum()`);
const overloadMain = `pub fnc main(): int {
    let enabled = true;
    let p = Point(enabled, 20);
    let q = Point(10, 20);
    stdio.println(p.sum());
    stdio.println(q.pick(p.is_origin()));
    stdio.println(q.pick(p.sum()));
    return 0;
}`;
const branching = decl.replace('self.x = x; self.y = y;', 'if x > 0 { self.x = x; } else { self.x = 0 - x; } fi self.y = self.x + y;');
const nestedBranching = decl.replace('self.x = x; self.y = y;', 'if x > 0 { if y > 0 { self.x = x; } else { self.x = y; } fi } else { self.x = 0; } fi self.y = self.x + y;');
const skippedTrap = decl.replace('self.x = x; self.y = y;', 'if x > 0 { self.x = x; } else { self.x = 9223372036854775807 + (y + 1); } fi self.y = y;');
const mutating = decl.replace('    mtd int • sum()', `    mut mtd shift(int n): void { self.x = self.x + n; self.y = self.y + n; }
    mut mtd swap(int old_x, int old_y): void { self.x = old_y; self.y = old_x; return; }
    mtd int • sum()`);
const mutationMain = `pub fnc main(): int {
    let mut p = Point(10, 20);
    let q = p;
    let mut r = copy! p;
    p.shift(5);
    r.shift(2);
    stdio.println(p.sum());
    stdio.println(q.sum());
    stdio.println(r.sum());
    return 0;
}`;
const snapshotMain = `pub fnc main(): int {
    let mut p = Point(10, 20);
    p.swap(p.sum(), p.product());
    stdio.println(p.product());
    stdio.println(p.sum());
    stdio.println(p.is_origin());
    return 0;
}`;
const direct = s => s.replace('type Point = struct', 'define struct Point').replace('is_origin() : bool', 'is_origin() -> bool');
// Equal field shapes and signatures must not share nominal dispatch or storage.
const secondOwner = direct(decl).replaceAll('Point', 'Offset').replace('return self.x + self.y;', 'return self.x - self.y;');
const twoOwners = direct(decl) + '\n' + secondOwner;
const twoOwnerMain = `pub fnc main(): int {
    let Point p = Point(10, 20);
    let Offset q = Offset(10, 20);
    let Offset r = copy! q;
    stdio.println(p.sum());
    stdio.println(q.sum());
    stdio.println(r.product());
    return 0;
}`;
const twoMutOwners = direct(mutating) + '\n' + direct(mutating).replaceAll('Point', 'Offset').replace('return self.x + self.y;', 'return self.x - self.y;');
const twoMutMain = `pub fnc main(): int {
    let mut p = Point(10, 20);
    let mut q = Offset(10, 20);
    let r = q;
    p.shift(5);
    q.swap(3, 7);
    stdio.println(p.sum());
    stdio.println(q.sum());
    stdio.println(r.sum());
    return 0;
}`;

const rectangle = `define struct Rectangle {
    Point top_left;
    Point bottom_right;
} with {
    constructor(Point a, Point b) { self.top_left = a; self.bottom_right = b; }
    mtd area() -> int { return (self.bottom_right.x - self.top_left.x) * (self.bottom_right.y - self.top_left.y); }
    mut mtd shift(int n): void { self.top_left.x = self.top_left.x + n; }
}`;
const nestedOwners = direct(mutating) + '\n' + rectangle;
const nestedMain = `pub fnc main(): int {
    let mut p = Point(1, 2);
    let q = Point(4, 6);
    let mut r = Rectangle(p, q);
    let copy: Rectangle = copy! r;
    p.shift(10);
    stdio.println(r.area());
    r.shift(1);
    stdio.println(r.area());
    stdio.println(copy.area());
    return 0;
}`.replace('let copy: Rectangle', 'let snapshot: Rectangle').replace('copy.area()', 'snapshot.area()');
const boxOwner = `define struct BoxValue {
    Rectangle inner;
} with {
    constructor(Rectangle r) { self.inner = r; }
    mtd area() -> int { return (self.inner.bottom_right.x - self.inner.top_left.x) * (self.inner.bottom_right.y - self.inner.top_left.y); }
}`;
const threeLevelMain = nestedMain.replace('    p.shift(10);', '    let b = BoxValue(r);\n    p.shift(10);').replace('stdio.println(copy.area());','stdio.println(b.area());').replace('stdio.println(snapshot.area());','stdio.println(b.area());');
const positives = [
    ['nested-nominal-value-copy', nestedOwners + '\n' + nestedMain, '12\n8\n12\n'],
    ['nested-three-level', nestedOwners + '\n' + boxOwner + '\n' + threeLevelMain, '12\n8\n12\n'],
    ['nested-forward-owner', rectangle + '\n' + direct(mutating) + '\n' + nestedMain, '12\n8\n12\n'],
    ['distinct-nominal-owners', twoOwners + '\n' + twoOwnerMain, '30\n-10\n200\n'],
    ['distinct-mutable-owner-copies', twoMutOwners + '\n' + twoMutMain, '40\n4\n-10\n'],
    ['define-grouped-arrow', direct(decl) + '\n' + main, '30\n200\nfalse\n'],
    ['define-chained-arrow', direct(chained) + '\n' + main, '30\n200\nfalse\n'],
    ['define-all-right-arrow', direct(decl).replace('int • sum()', 'sum() -> int').replace('int <- product()', 'product() -> int') + '\n' + main, '30\n200\nfalse\n'],
    ['mutable-copy-isolation', mutating + '\n' + mutationMain, '40\n30\n34\n'],
    ['mutable-snapshot-arguments', mutating + '\n' + snapshotMain, '6000\n230\nfalse\n'],
    ['mutable-val-authority', mutating + '\n' + mutationMain.replace('let mut p =', 'val p ='), '40\n30\n34\n'],

    ['constructor-branch-true', branching + '\n' + main, '40\n300\nfalse\n'],
    ['constructor-branch-false', branching + '\n' + main.replace('10, 20', '-10, 20'), '40\n300\nfalse\n'],
    ['constructor-nested-branch', nestedBranching + '\n' + main.replace('10, 20', '10, -2'), '-6\n8\nfalse\n'],
    ['constructor-untaken-trap', skippedTrap + '\n' + main.replace('10, 20', '10, 0'), '10\n0\nfalse\n'],

    ['same-arity-overloads', overloaded + '\n' + overloadMain, '21\n10\n31\n'],
    ['exact-overload-expression-types', overloaded + '\n' + overloadMain.replace('Point(enabled, 20)', 'Point(1 == 1, 20)').replace('q.pick(p.sum())', 'q.pick(p.sum() + 1)'), '21\n10\n32\n'],

    ['type-first-binding', decl + '\n' + main.replace('let p =', 'let Point p ='), '30\n200\nfalse\n'],
    ['postfix-type-binding', decl + '\n' + main.replace('let p =', 'let p: Point ='), '30\n200\nfalse\n'],
    ['independent-scalar-copies', decl + '\n' + main.replace('    stdio.println(p.sum());', '    let Point q = p;\n    let r: Point = copy! q;\n    stdio.println(r.sum());'), '30\n200\nfalse\n'],

    ['grouped', decl + '\n' + main, '30\n200\nfalse\n'],
    ['chained', chained + '\n' + main, '30\n200\nfalse\n'],
    ['postfix', decl.replace('int • sum()', 'sum(): int').replace('int <- product()', 'product(): int') + '\n' + main, '30\n200\nfalse\n'],
    ['independent-local-values', decl + '\n' + two, '30\n60\nfalse\n'],
    ['constructor-dependency', decl.replace('self.y = y;', 'self.y = self.x + y;') + '\n' + main, '40\n300\nfalse\n'],
    ['origin', decl + '\n' + main.replace('10, 20', '0, 0'), '0\n0\ntrue\n'],
];
const negatives = [
    ['nested-mutual-cycle', direct(decl).replace('int x;', 'Rectangle x;') + '\n' + rectangle + '\n' + nestedMain, /E_STRUCT_LAYOUT_CYCLE/],
    ['nested-slot-budget', nestedOwners + '\n' + Array.from({length:4}, (_,i)=>'define struct Wide'+i+' { '+Array.from({length:3},(_,j)=>(i===0?'Rectangle':'Wide'+(i-1))+' f'+j+';').join(' ')+' } with { constructor() {} }').join('\n') + '\n' + nestedMain, /E_SOURCE_LIMIT/],
    ['nested-parameter-snapshot-budget', nestedOwners + '\n' + 'define struct TooWide { int x; } with { constructor('+Array.from({length:9},(_,i)=>'Rectangle a'+i).join(', ')+') { self.x = 1; } }' + '\n' + nestedMain],

    ['nested-wrong-nominal-argument', nestedOwners + '\n' + secondOwner + '\n' + nestedMain.replace('let q = Point', 'let q = Offset'), /E_TYPE/],
    ['nested-wrong-field-identity', nestedOwners.replace('Point bottom_right;', 'Offset bottom_right;') + '\n' + secondOwner + '\n' + nestedMain, /E_TYPE/],
    ['nested-uninitialized-field', nestedOwners.replace('self.bottom_right = b;', '') + '\n' + nestedMain, /E_STRUCT_INIT/],
    ['nested-duplicate-field', nestedOwners.replace('self.bottom_right = b;', 'self.top_left = b; self.bottom_right = b;') + '\n' + nestedMain, /E_STRUCT_INIT/],
    ['nested-unknown-owner', nestedOwners.replace('Point top_left;', 'Missing top_left;') + '\n' + nestedMain, /E_TYPE/],
    ['nested-cycle', nestedOwners.replace('Point top_left;', 'Rectangle top_left;') + '\n' + nestedMain, /E_STRUCT_LAYOUT_CYCLE/],
    ['nested-missing-projection', nestedOwners.replace('self.bottom_right.x -', 'self.bottom_right.z -') + '\n' + nestedMain],
    ['nested-immutable-receiver', nestedOwners + '\n' + nestedMain.replace('let mut r =', 'let r ='), /E_BORROW_MUT_REQUIRED/],
    ['wrong-second-owner-annotation', twoOwners + '\n' + twoOwnerMain.replace('let Offset q =', 'let Point q =')],
    ['wrong-second-owner-copy', twoOwners + '\n' + twoOwnerMain.replace('let Offset r =', 'let Point r =')],
    ['duplicate-nominal-owner', twoOwners.replaceAll('Offset', 'Point') + '\n' + twoOwnerMain],
    ['owner-method-not-structural', twoOwners.replace('mtd int • sum() { return self.x - self.y; }', 'mtd int • other() { return self.x - self.y; }') + '\n' + twoOwnerMain],
    ['owner-shadow', twoOwners + '\n' + twoOwnerMain.replace('let Point p =', 'let Point Offset =')],
    ['owner-count-limit', Array.from({length:17}, (_,i)=>direct(decl).replaceAll('Point', 'Point'+i)).join('\n')+'\n'+main, /E_SOURCE_LIMIT/],
    ['define-duplicate-arrow', direct(decl).replace('int <- product()', 'sum() -> int') + '\n' + main],
    ['define-duplicate-result-annotation', direct(decl).replace('is_origin() -> bool', 'is_origin() -> bool: bool') + '\n' + main],
    ['define-nested-layout-unimplemented', direct(decl).replace('int x;', 'Point x;') + '\n' + main],
    ['mut-readonly-owner', mutating + '\n' + mutationMain.replace('let mut p =', 'let p =')],
    ['mut-readonly-copy', mutating + '\n' + mutationMain.replace('p.shift(5);', 'q.shift(5);')],
    ['mut-wrong-argument', mutating + '\n' + mutationMain.replace('p.shift(5)', 'p.shift(true)')],
    ['mut-escape-borrow', mutating + '\n' + mutationMain.replace('let q = p;', 'let q = borrow! mut p;')],
    ['mut-unused-invalid-body', mutating.replace('self.x = old_y;', 'self.x = true;') + '\n' + mutationMain],
    ['mut-unknown-field', mutating.replace('self.x = old_y;', 'self.z = old_y;') + '\n' + mutationMain],
    ['mut-no-expression-result', mutating + '\n' + mutationMain.replace('p.shift(5);', 'let v = p.shift(5);')],
    ['mut-no-unmarked-write', mutating.replace('mut mtd shift', 'mtd shift') + '\n' + mutationMain],
    ['mut-no-callback', mutating.replace('self.x = old_y;', 'self.x = self.sum();') + '\n' + mutationMain],
    ['mut-duplicate-readonly-signature', mutating.replace('mut mtd shift', 'mtd int • shift(int n) { return n; } mut mtd shift') + '\n' + mutationMain],

    ['constructor-missing-branch-field', branching.replace('else { self.x = 0 - x; }', 'else {}') + '\n' + main],
    ['constructor-branch-read-before-init', branching.replace('self.x = 0 - x;', 'self.x = self.y;') + '\n' + main],
    ['constructor-branch-double-init', branching.replace('self.y = self.x + y;', 'self.x = x; self.y = y;') + '\n' + main],
    ['constructor-nonbool-condition', branching.replace('if x > 0', 'if x') + '\n' + main],
    ['constructor-missing-else', branching.replace('else { self.x = 0 - x; }', '') + '\n' + main],

    ['overload-no-exact-constructor', overloaded + '\n' + overloadMain.replace('Point(enabled, 20)', 'Point(enabled, true)')],
    ['overload-no-exact-method', overloaded + '\n' + overloadMain.replace('q.pick(p.sum())', 'q.pick("bad")')],
    ['overload-return-only-duplicate', overloaded.replace('mtd pick(bool b) : int', 'mtd pick(int b) : bool') + '\n' + overloadMain],
    ['overload-wrong-typed-local', overloaded + '\n' + overloadMain.replace('let enabled = true;', 'let int enabled = true;')],

    ['wrong-owner-annotation', decl + '\n' + main.replace('let p =', 'let Other p =')],
    ['scalar-annotation-on-struct', decl + '\n' + main.replace('let p =', 'let int p =')],
    ['runtime-const-struct', decl + '\n' + main.replace('let p =', 'const p =')],
    ['wrong-copy-annotation', decl + '\n' + main.replace('    stdio.println(p.sum());', '    let Other q = copy! p;\n    stdio.println(q.sum());')],
    ['unsupported-deep-copy', decl + '\n' + main.replace('    stdio.println(p.sum());', '    let q = copy!! p;\n    stdio.println(q.sum());')],

    ['asterisk', decl.replace('is_origin() : bool', 'bool * is_origin()') + '\n' + main],
    ['missing-field', decl.replace('self.y = y;', '') + '\n' + main],
    ['bad-field-read', decl.replace('self.x = x;', 'self.x = self.y;') + '\n' + main],
    ['wrong-type', decl + '\n' + main.replace('10, 20', 'true, 20')],
    ['wrong-arity', decl + '\n' + main.replace('10, 20', '10')],
    ['readonly-write', decl.replace('return self.x + self.y;', 'self.x = 1; return self.x;') + '\n' + main],
    ['heap-new', decl + '\n' + main.replace('Point(10, 20)', 'new Point(10, 20)')],
    ['bypass-literal', decl + '\n' + main.replace('Point(10, 20)', 'Point {x: 10, y: 20}')],
    ['value-escape', decl + '\n' + main.replace('return 0;', 'return p;')],
    ['wrong-receiver', decl + '\n' + main.replace('p.sum()', '10.sum()')],
    ['duplicate-method', decl.replace('int <- product()', 'int <- sum()') + '\n' + main],
];
// A compact source must not amplify repeated mutating method bodies without bound.
const largeMutator = decl.replace('    mtd int • sum()', '    mut mtd bump():void { ' +
    'self.x = self.x + 1; '.repeat(128) + '}\n    mtd int • sum()');
const expansionSource = largeMutator + '\npub fnc main():int { let mut p = Point(0, 0); ' +
    'p.bump(); '.repeat(128) + 'return 0; }';
negatives.push(['mut-expansion-budget', expansionSource, /E_SOURCE_LIMIT: attached struct expansion node budget exceeded/]);
positives.push(['mut-expansion-below-budget', largeMutator +
    '\npub fnc main():int { let mut p = Point(0, 0); p.bump(); p.bump(); stdio.println(p.sum()); return 0; }', '256\n']);
const fixtureIds = [...positives, ...negatives].map(([id]) => id);
assert.equal(new Set(fixtureIds).size, fixtureIds.length, 'duplicate guest fixture ID');
const attoc = process.env.ATTOC || './target/debug/attoc';
const js = process.env.ATTO_STRUCT_TARGET === 'js';
const work = await mkdtemp(join(tmpdir(), 'atto-struct-value-'));
async function build(name, source) {
    const input = join(work, name + '.atto'), output = join(work, name + (js ? '.mjs' : '.wasm'));
    await writeFile(input, source);
    return { result: spawnSync(attoc, [...(js ? ['--emit=js'] : []), input, '-o', output], { encoding:'utf8', timeout:30_000 }), output };
}
try {
    for (const [name, source, expected] of positives) {
        const { result, output } = await build(name, source);
        assert.ifError(result.error);
        assert.equal(result.signal, null, name + ': compiler terminated');
        assert.equal(result.status, 0, name + ': ' + result.stderr);
        const bytes = await readFile(output);
        let stdout = '';
        const host = {
            print_i64: n => { stdout += String(n); },
            print_bool: b => { stdout += b ? 'true' : 'false'; },
            print_newline: () => { stdout += '\n'; },
        };
        let guest;
        if (js) {
            const source = bytes.toString('utf8');
            assert.ok(source.startsWith('export function instantiate'), name);
            assert.ok(!source.includes('WebAssembly.Memory'), name);
            const esm = await import(pathToFileURL(output).href);
            assert.deepEqual(esm.actorNames, []);
            guest = esm.instantiate(host);
            assert.deepEqual(Object.keys(guest.exports).sort(), ['main','panic_code']);
        } else {
            const module = new WebAssembly.Module(bytes);
            const exports = WebAssembly.Module.exports(module);
            assert.ok(exports.every(e => !e.name.startsWith('$struct_')), name + ': generated symbol escaped');
            assert.ok(exports.every(e => e.kind !== 'memory'), name + ': unexpected guest memory');
            const imports = WebAssembly.Module.imports(module);
            assert.ok(imports.every(i => i.module === 'atto' && ['print_i64','print_bool','print_newline'].includes(i.name)), name);
            guest = new WebAssembly.Instance(module, {atto:host});
        }
        assert.equal(guest.exports.main(), 0n, name);
        assert.equal(guest.exports.panic_code.value, 0, name);
        assert.equal(stdout, expected, name);
        const again = await build(name + '-again', source);
        assert.equal(again.result.status, 0, again.result.stderr);
        assert.deepEqual(await readFile(again.output), bytes, name + ': nondeterministic guest');
        console.log(js ? 'PASS actual ESM struct guest' : 'PASS actual struct guest', name, JSON.stringify(stdout));
    }
    for (const [name, source, diagnostic = /E_[A-Z_]+/] of negatives) {
        const { result, output } = await build(name, source);
        assert.ifError(result.error);
        assert.equal(result.signal, null, name + ': compiler terminated');
        assert.equal(result.status, 1, name + ': ' + result.stderr);
        assert.match(result.stderr, diagnostic, name);
        await assert.rejects(stat(output), {code:'ENOENT'}, name + ': rejected program emitted artifact');
        console.log(js ? 'PASS rejected ESM struct source' : 'PASS rejected struct source', name);
    }
} finally {
    await rm(work, {recursive:true, force:true});
}
