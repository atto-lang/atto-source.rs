use atto_compiler::compile;

fn read_u32(bytes: &[u8], cursor: &mut usize) -> u32 {
    let mut result = 0u32;
    for shift in (0..35).step_by(7) {
        let byte = *bytes.get(*cursor).expect("truncated wasm LEB");
        *cursor += 1;
        result |= ((byte & 0x7f) as u32) << shift;
        if byte & 0x80 == 0 { return result; }
    }
    panic!("invalid wasm LEB")
}

fn section<'a>(wasm: &'a [u8], wanted: u8) -> Option<&'a [u8]> {
    assert_eq!(&wasm[..8], b"\0asm\x01\0\0\0");
    let mut pos = 8;
    while pos < wasm.len() {
        let id = wasm[pos];
        pos += 1;
        let len = read_u32(wasm, &mut pos) as usize;
        let end = pos + len;
        assert!(end <= wasm.len());
        if id == wanted { return Some(&wasm[pos..end]); }
        pos = end;
    }
    None
}

fn exported_memory(wasm: &[u8]) -> bool {
    let Some(exports) = section(wasm, 7) else { return false };
    let mut pos = 0;
    let n = read_u32(exports, &mut pos);
    for _ in 0..n {
        let len = read_u32(exports, &mut pos) as usize;
        let name = &exports[pos..pos + len];
        pos += len;
        let kind = exports[pos]; pos += 1;
        let index = read_u32(exports, &mut pos);
        if name == b"memory" && kind == 2 && index == 0 { return true; }
    }
    false
}

#[test]
fn scalar_actor_modules_export_their_own_bounded_wasm32_memory() {
    let source = r#"
define actor Counter as
  let int total = 0;
  receive(ActorMail<int> mail): void {
    self.total = self.total + mail.value;
    return;
  }
end
pub fnc main(): int {
  val counter = spawn Counter();
  counter.send(1);
  return 0;
}
"#;
    let wasm = compile(source).expect("scalar actor fixture must compile");
    assert!(exported_memory(&wasm), "every actor module must export its private memory");
    let mem = section(&wasm, 5).expect("actor must have a memory section");
    assert_eq!(mem, &[1, 1, 1, 1], "one memory, min=1 page, max=1 page");
}

#[test]
fn scalar_program_without_actors_does_not_gain_an_unneeded_memory() {
    let wasm = compile("pub fnc main(): int { return 42; }").unwrap();
    assert!(!exported_memory(&wasm));
    assert!(section(&wasm, 5).is_none());
}

#[test]
fn empty_array_iteration_still_exports_memory_for_unreachable_load_instruction() {
    // The typed array loop contains an i64.load in its body even when a
    // statically empty descriptor prevents the loop from entering that body.
    // An empty data segment must not silently suppress the Wasm memory.
    let wasm = compile(
        "pub fnc main(): int { val xs: Array<int> = [];          for (int n : xs) { return n; } return 7; }",
    ).expect("empty array guest must compile");
    assert!(exported_memory(&wasm), "empty array loop requires memory");
    assert!(section(&wasm, 5).is_some(), "Wasm memory section must exist");
    assert!(section(&wasm, 11).is_some(), "empty array sentinel data must exist");
}

#[test]
fn nonempty_readonly_array_exports_bounded_module_memory() {
    let wasm = compile(
        "pub fnc main(): int { val xs: Array<int> = [1,2,3];          let sum = 0; for (int n : xs) { sum = sum + n; } return sum; }",
    ).expect("array range-for must compile");
    assert!(exported_memory(&wasm));
    let memory = section(&wasm, 5).expect("array needs one Wasm memory");
    assert_eq!(memory, &[1, 1, 1, 1], "one bounded Wasm page");
}
