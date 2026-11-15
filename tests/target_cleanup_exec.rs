//! Executed native cleanup-path tests. These are a restricted Rust runtime
//! witness; no out-of-line Atto receiver is executable in the guest backend.
use std::sync::{mpsc, Arc, Barrier};
use std::thread;
use std::time::Duration;

use atto_compiler::lexer::lex;
use atto_compiler::target_cleanup::{
    require_backend_cleanup_proof, LeaseCleanupExit, ReceiverCleanupPlan,
};
use atto_compiler::target_cleanup_cfg::{
    lower_verified_cleanup_cfg, native_experiment::LeaseCell,
    CleanupExit, CleanupOp, VerifiedCleanupCfg,
};
use atto_compiler::target_syntax::parse_target_declarations;
use atto_compiler::target_typed::lower_target_signatures;

const INCREMENT: &str = r#"
declare actor Counter;
define actor Counter as let int count = 0; end
mtd (mut self: Counter) increment(int delta): void {
    self.handle_state(critical-write |s| -> { s.count = s.count + delta; });
}
"#;
const READ: &str = r#"
declare actor Counter;
define actor Counter as let int count = 0; end
mtd (Counter self) current(): int {
    return self.handle_state(critical-read |s| -> s.count);
}
"#;
const READ_PANIC: &str = r#"
declare actor Counter;
define actor Counter as let int count = 0; end
mtd (Counter self) next_count(): int {
    return self.handle_state(critical-read |s| -> s.count + 1);
}
"#;

fn plan(src:&str) -> ReceiverCleanupPlan {
    let headers=parse_target_declarations(&lex(src).unwrap()).unwrap();
    let mut typed=lower_target_signatures(&headers).unwrap();
    assert_eq!(typed.receiver_cleanup.len(),1);
    typed.receiver_cleanup.remove(0)
}
fn cfg(src:&str)->VerifiedCleanupCfg {
    lower_verified_cleanup_cfg(&plan(src)).unwrap()
}

#[test]
fn native_cleanup_cfg_has_checked_normal_panic_and_cancel_paths() {
    let plan=plan(INCREMENT);
    assert_eq!(plan.leases.len(),1);
    assert_eq!(plan.leases[0].cleanup.len(),2);
    assert_eq!(plan.leases[0].cleanup[1].exit,LeaseCleanupExit::CheckedPanic);
    let cfg=lower_verified_cleanup_cfg(&plan).unwrap();
    assert_eq!(cfg.lease_count(),1);
    assert_eq!(cfg.paths().len(),3);
    for exit in [CleanupExit::Normal, CleanupExit::CheckedPanic,
        CleanupExit::CooperativeCancel] {
        let path=cfg.paths().iter().find(|path|path.exit==exit).unwrap();
        assert_eq!(path.ops.len(),3);
        assert!(matches!(path.ops[0],CleanupOp::Acquire{lease:0,..}));
        assert_eq!(path.ops[1],CleanupOp::Release{lease:0});
        assert_eq!(path.ops[2],CleanupOp::Exit(exit));
    }
    assert_eq!(require_backend_cleanup_proof(&plan).unwrap_err().code,
        "E_UNSUPPORTED","experimental execution cannot forge the production proof");
    assert_eq!(atto_compiler::compile(INCREMENT).unwrap_err().code,"E_UNSUPPORTED");
}

#[test]
fn native_cleanup_cfg_rejects_missing_or_forged_release_obligations() {
    let mut broken=plan(INCREMENT);
    broken.leases[0].cleanup.pop();
    assert_eq!(lower_verified_cleanup_cfg(&broken).unwrap_err().code,"E_CLEANUP");
    let mut broken=plan(INCREMENT);
    broken.leases[0].cleanup[0].lease=22;
    assert_eq!(lower_verified_cleanup_cfg(&broken).unwrap_err().code,"E_CLEANUP");
    let mut broken=plan(INCREMENT);
    broken.leases[0].cleanup[1].exit=LeaseCleanupExit::Normal;
    assert_eq!(lower_verified_cleanup_cfg(&broken).unwrap_err().code,"E_CLEANUP");
    let mut broken=plan(INCREMENT);
    broken.backend_cleanup_proven=true;
    assert_eq!(lower_verified_cleanup_cfg(&broken).unwrap_err().code,"E_CLEANUP");
    let mut broken=plan(INCREMENT);
    broken.leases[0].cleanup[0].backend_release_proven=true;
    assert_eq!(lower_verified_cleanup_cfg(&broken).unwrap_err().code,"E_CLEANUP");
    let mut broken=plan(INCREMENT);
    broken.leases[0].receiver=1234;
    assert_eq!(lower_verified_cleanup_cfg(&broken).unwrap_err().code,"E_CLEANUP");
}

#[test]
fn native_executable_lease_is_released_once_on_normal_panic_and_cancel() {
    let cfg=cfg(INCREMENT);
    let cell=LeaseCell::new(i64::MAX-1);
    assert_eq!(cell.checked_add(&cfg,0,1).unwrap().unwrap(),i64::MAX);
    assert_eq!(cell.release_count(),1);
    assert!(cell.try_write_available());
    assert!(cell.checked_add(&cfg,0,1).unwrap().is_err(),
        "checked overflow must unwind, not silently wrap");
    assert_eq!(cell.release_count(),2);
    assert!(cell.try_write_available(),"panic must not leak exclusive lease");
    assert_eq!(cell.cancel_while_exclusive(&cfg,0),Err("cancelled"));
    assert_eq!(cell.release_count(),3);
    assert!(cell.try_write_available(),"cooperative cancellation must release");
    assert_eq!(cell.with_shared(&cfg,0,|_|()).unwrap_err(),
        "unverified shared-read cleanup path");
}

#[test]
fn native_shared_readers_coexist_and_exclude_writer() {
    let cfg=Arc::new(cfg(READ));
    assert_eq!(cfg.paths().len(),2);
    let cell=Arc::new(LeaseCell::new(41));
    let gate=Arc::new(Barrier::new(3));
    let (tx,rx)=mpsc::channel();
    let mut handles=Vec::new();
    for _ in 0..2 {
        let cell=Arc::clone(&cell);
        let cfg=Arc::clone(&cfg);
        let gate=Arc::clone(&gate);
        let tx=tx.clone();
        handles.push(thread::spawn(move|| {
            cell.with_shared(&cfg,0,|value| {
                assert_eq!(value,41);
                tx.send(()).unwrap();
                gate.wait();
            }).unwrap();
        }));
    }
    drop(tx);
    for _ in 0..2 {
        rx.recv_timeout(Duration::from_secs(5))
            .expect("both readers must enter concurrently");
    }
    assert!(!cell.try_write_available(),"writer must be excluded by both reads");
    assert_eq!(cell.release_count(),0,
        "release count must not advance while either lease is still locked");
    gate.wait();
    for handle in handles { handle.join().unwrap(); }
    assert!(cell.try_write_available(),"both read leases must be released");
    assert_eq!(cell.release_count(),2);
    assert!(cell.checked_add(&cfg,0,1).is_err(),
        "read-only CFG may not authorize exclusive mutation");
}

#[test]
fn native_read_panic_releases_lease_and_is_not_production_actor_unwind() {
    let cfg=cfg(READ_PANIC);
    let cell=LeaseCell::new(5);
    assert!(cell.panic_while_shared(&cfg,0).unwrap().is_err());
    assert_eq!(cell.release_count(),1);
    assert!(cell.try_write_available());
    assert_eq!(atto_compiler::compile(READ_PANIC).unwrap_err().code,
        "E_UNSUPPORTED");
}

#[test]
fn native_lease_free_receiver_has_no_cleanup_paths_or_aot_permission() {
    let src="declare actor Stateless;
        mtd (Stateless self) identity(int n): int { return n; }";
    let p=plan(src);
    let cfg=lower_verified_cleanup_cfg(&p).unwrap();
    assert_eq!(cfg.lease_count(),0);
    assert!(cfg.paths().is_empty());
    assert!(require_backend_cleanup_proof(&p).is_ok());
    assert_eq!(atto_compiler::compile(src).unwrap_err().code,
        "E_UNSUPPORTED");
}
