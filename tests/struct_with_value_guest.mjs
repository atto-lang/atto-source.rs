// Real compiler -> guest Wasm. No host-side interpretation of Atto source.
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, stat, rm } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

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
const skippedTrap = decl.replace('self.x = x; self.y = y;', 'if x > 0 { self.x = x; } else { self.x = 1 / y; } fi self.y = y;');
const positives = [
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
const attoc = process.env.ATTOC || './target/debug/attoc';
const work = await mkdtemp(join(tmpdir(), 'atto-struct-value-'));
async function build(name, source) {
    const input = join(work, name + '.atto'), output = join(work, name + '.wasm');
    await writeFile(input, source);
    return { result: spawnSync(attoc, [input, '-o', output], { encoding:'utf8', timeout:30_000 }), output };
}
try {
    for (const [name, source, expected] of positives) {
        const { result, output } = await build(name, source);
        assert.equal(result.status, 0, name + ': ' + result.stderr);
        const bytes = await readFile(output);
        const module = new WebAssembly.Module(bytes);
        const exports = WebAssembly.Module.exports(module);
        assert.ok(exports.every(e => !e.name.startsWith('$struct_')), name + ': generated symbol escaped');
        assert.ok(exports.every(e => e.kind !== 'memory'), name + ': unexpected guest memory');
        const imports = WebAssembly.Module.imports(module);
        assert.ok(imports.every(i => i.module === 'atto' && ['print_i64','print_bool','print_newline'].includes(i.name)), name);
        let stdout = '';
        const guest = new WebAssembly.Instance(module, {atto:{
            print_i64: n => { stdout += String(n); },
            print_bool: b => { stdout += b ? 'true' : 'false'; },
            print_newline: () => { stdout += '\n'; },
        }});
        assert.equal(guest.exports.main(), 0n, name);
        assert.equal(guest.exports.panic_code.value, 0, name);
        assert.equal(stdout, expected, name);
        const again = await build(name + '-again', source);
        assert.equal(again.result.status, 0, again.result.stderr);
        assert.deepEqual(await readFile(again.output), bytes, name + ': nondeterministic guest');
        console.log('PASS actual struct guest', name, JSON.stringify(stdout));
    }
    for (const [name, source] of negatives) {
        const { result, output } = await build(name, source);
        assert.equal(result.status, 1, name + ': ' + result.stderr);
        assert.match(result.stderr, /E_[A-Z_]+/, name);
        await assert.rejects(stat(output), {code:'ENOENT'}, name + ': rejected program emitted artifact');
        console.log('PASS rejected struct source', name);
    }
} finally {
    await rm(work, {recursive:true, force:true});
}
