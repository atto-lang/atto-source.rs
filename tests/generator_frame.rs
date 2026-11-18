//! Checks for bounded generator continuation layout, handle fencing,
//! and lifecycle semantics. These are ABI-contract tests, NOT evidence
//! that guest WebAssembly can execute generators yet.

use atto_compiler::diag::Pos;
use atto_compiler::generator_frame::{
    GeneratorFrameLayout, GeneratorHandle, GeneratorState, FRAME_HEADER_BYTES, MAX_GENERATOR_FRAME_BYTES, MAX_GUEST_ARENA_BYTES,
};

fn pos() -> Pos {
    Pos { line: 3, col: 7 }
}

#[test]
fn offsets_are_bounded_aligned_and_use_disjoint_parameter_local_delegate_regions() {
    let f = GeneratorFrameLayout::new(2, 3, 2, pos()).unwrap();
    assert_eq!(f.parameters_offset, FRAME_HEADER_BYTES);
    assert_eq!(f.locals_offset, 48);
    assert_eq!(f.delegates_offset, 72);
    assert_eq!(f.frame_bytes, 88);
    assert_eq!(f.parameter_offset(0), Some(32));
    assert_eq!(f.parameter_offset(1), Some(40));
    assert_eq!(f.parameter_offset(2), None);
    assert_eq!(f.local_offset(0), Some(48));
    assert_eq!(f.local_offset(2), Some(64));
    assert_eq!(f.local_offset(3), None);
    assert_eq!(f.delegate_offset(0), Some(72));
    assert_eq!(f.delegate_offset(1), Some(80));
    assert_eq!(f.delegate_offset(2), None);
    for offset in [f.parameters_offset, f.locals_offset, f.delegates_offset, f.frame_bytes] {
        assert_eq!(offset % 8, 0);
    }
}

#[test]
fn frame_size_and_arithmetic_overflow_fail_before_wasm_allocation() {
    for (p, l, d) in [(u32::MAX, 1, 1), (0, u32::MAX, 0), (0, 0, u32::MAX), (1024, 1024, 8192)] {
        let error = GeneratorFrameLayout::new(p, l, d, pos()).unwrap_err();
        assert_eq!(error.code, "E_GENERATOR_FRAME");
        assert_eq!(error.pos, pos());
    }
    assert!(GeneratorFrameLayout::new(32, 1024, 4096, pos()).is_ok());
    let huge = GeneratorFrameLayout::new(0, 0, MAX_GENERATOR_FRAME_BYTES / 8, pos());
    assert!(huge.is_err(), "header overhead must count toward frame size");
}

#[test]
fn generation_tag_prevents_stale_handle_alias_when_offset_is_reused() {
    let original = GeneratorHandle::new(0x100, 7, pos()).unwrap();
    assert_eq!(original.arena_offset(), 0x100);
    assert_eq!(original.generation(), 7);
    assert_eq!(GeneratorHandle::from_bits(original.bits(), pos()).unwrap(), original);
    let new = GeneratorHandle::new(original.arena_offset(), original.next_generation(pos()).unwrap(), pos()).unwrap();
    assert_ne!(original.bits(), new.bits(), "reused arena location must have distinct handle bits");
    assert_eq!(original.arena_offset(), new.arena_offset());

    for (offset, generation) in [(0, 1), (1, 1), (7, 1), (8, 0), (MAX_GUEST_ARENA_BYTES, 1)] {
        assert_eq!(
            GeneratorHandle::new(offset, generation, pos()).unwrap_err().code,
            "E_GENERATOR_FRAME"
        );
    }
    assert_eq!(GeneratorHandle::from_bits(0, pos()).unwrap_err().code, "E_GENERATOR_FRAME");
    let terminal = GeneratorHandle::new(8, u32::MAX, pos()).unwrap();
    assert_eq!(terminal.next_generation(pos()).unwrap_err().code, "E_GENERATOR_FRAME");
}

#[test]
fn generator_state_machine_disallows_reentrant_next_and_pull_after_close() {
    use GeneratorState::*;
    let running = Created.begin_pull(pos()).unwrap();
    assert_eq!(running, Running);
    assert_eq!(running.begin_pull(pos()).unwrap_err().code, "E_GENERATOR_STATE");
    let suspended = running.yield_item(pos()).unwrap();
    assert_eq!(suspended, Suspended);
    let running = suspended.begin_pull(pos()).unwrap();
    let done = running.finish(pos()).unwrap();
    assert_eq!(done, Done);
    assert_eq!(done.begin_pull(pos()).unwrap(), Done);
    assert_eq!(done.request_close(), Closed);
    assert_eq!(Closed.request_close(), Closed);
    assert_eq!(Closed.begin_pull(pos()).unwrap_err().code, "E_GENERATOR_STATE");
    assert_eq!(Suspended.yield_item(pos()).unwrap_err().code, "E_GENERATOR_STATE");
}

#[test]
fn running_generator_closure_requires_unwinding_before_reuse() {
    use GeneratorState::*;
    let closing = Running.request_close();
    assert_eq!(closing, Closing);
    assert_eq!(closing.request_close(), Closing);
    assert_eq!(closing.begin_pull(pos()).unwrap_err().code, "E_GENERATOR_STATE");
    assert_eq!(closing.yield_item(pos()).unwrap_err().code, "E_GENERATOR_STATE");
    assert_eq!(Closing.complete_unwind(pos()).unwrap(), Closed);
    assert_eq!(Done.complete_unwind(pos()).unwrap_err().code, "E_GENERATOR_STATE");
}
