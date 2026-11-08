// Strict import surface for source-owned browser guest Wasm.
// A name allowlist alone accepts import-kind confusion and duplicate slots.
export const ACTOR_GUEST_IMPORTS = Object.freeze([
  'print_i64', 'print_bool', 'print_str', 'print_newline',
  'actor_spawn', 'actor_send', 'actor_join',
]);
const allowed = new Set(ACTOR_GUEST_IMPORTS);
export function validate_actor_guest_imports(module) {
  const imports = WebAssembly.Module.imports(module);
  if (imports.length > ACTOR_GUEST_IMPORTS.length) {
    throw new Error('E_ACTOR_IMPORT_COUNT');
  }
  const seen = new Set();
  for (const imp of imports) {
    if (imp.module !== 'atto' || !allowed.has(imp.name)) {
      throw new Error('E_ACTOR_IMPORT_NAME');
    }
    if (imp.kind !== 'function') {
      throw new Error('E_ACTOR_IMPORT_KIND');
    }
    if (seen.has(imp.name)) {
      throw new Error('E_ACTOR_IMPORT_DUPLICATE');
    }
    seen.add(imp.name);
  }
  return imports;
}
