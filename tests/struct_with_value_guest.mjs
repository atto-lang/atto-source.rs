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
const positives = [
    ['grouped', decl + '\n' + main, '30\n200\nfalse\n'],
    ['chained', chained + '\n' + main, '30\n200\nfalse\n'],
    ['postfix', decl.replace('int • sum()', 'sum(): int').replace('int <- product()', 'product(): int') + '\n' + main, '30\n200\nfalse\n'],
    ['independent-local-values', decl + '\n' + two, '30\n60\nfalse\n'],
    ['constructor-dependency', decl.replace('self.y = y;', 'self.y = self.x + y;') + '\n' + main, '40\n300\nfalse\n'],
    ['origin', decl + '\n' + main.replace('10, 20', '0, 0'), '0\n0\ntrue\n'],
];
const negatives = [
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
