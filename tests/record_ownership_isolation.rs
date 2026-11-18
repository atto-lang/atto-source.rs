//! Fail-closed Record ownership and actor-isolation contracts.
//! A rejected program must never produce a partial guest-Wasm artifact.
//! Deep graph cloning and Record actor payloads remain unimplemented.
use atto_compiler::compile;

fn reject(source: &str, expected: &str) {
    let err = compile(source).expect_err("unsupported ownership or actor transport emitted guest Wasm");
    assert_eq!(err.code, expected, "{source}\n{err}");
}

#[test]
fn record_copy_does_not_default_to_shallow_aliasing() {
    reject(
        "pub fnc main(): int { let Record r = {x: 42}; val clone = copy! r; return clone.x; }",
        "E_COPY",
    );
}

#[test]
fn record_share_does_not_allow_mutable_cross_owner_aliasing() {
    reject(
        "pub fnc main(): int { let Record r = {x: 42}; val alias = share! r; return alias.x; }",
        "E_SHARE",
    );
}

#[test]
fn taken_record_cannot_be_read_from_old_binding() {
    reject(
        "pub fnc main(): int { let Record r = {x: 42}; val moved = take! r; return r.x + moved.x; }",
        "E_MOVED",
    );
}

#[test]
fn mutable_record_borrow_cannot_write_to_immutable_closed_fields() {
    reject(
        "pub fnc main(): int { let Record r = {x: 42}; block { val writer = borrow! mut r; writer.x = 7; } return r.x; }",
        "E_BORROW",
    );
}

#[test]
fn borrowed_class_cannot_be_taken_while_loan_is_live() {
    reject(
        "define class Parcel as pub let int value = 42; end pub fnc main(): int {
            let parcel = new Parcel();
            val reader = borrow! parcel;
            val moved = take! parcel;
            return reader.value + moved.value;
        }",
        "E_BORROW",
    );
}

#[test]
fn class_copy_is_not_silently_changed_into_a_shallow_clone() {
    reject(
        "define class Parcel as pub let int value = 42; end pub fnc main(): int {
            let parcel = new Parcel();
            val duplicated = copy! parcel;
            return duplicated.value;
        }",
        "E_COPY",
    );
}

#[test]
fn actor_scalar_mailbox_rejects_record_object_messages() {
    reject(
        "define actor Worker as receive(ActorMail<int> mail): void { return; } end
         pub fnc main(): int {
            let Record r = {x: 42};
            val worker = spawn Worker();
            worker.send(r);
            return 0;
         }",
        "E_TYPE",
    );
}

#[test]
fn actor_record_mailbox_cannot_claim_unproven_graph_transport() {
    reject(
        "define actor Worker as receive(ActorMail<Record> mail): void { return; } end
         pub fnc main(): int { return 0; }",
        "E_UNSUPPORTED",
    );
}

#[test]
fn actor_private_mutable_state_is_not_publicly_readable() {
    reject(
        "define actor Worker as let int secret = 42;
            receive(ActorMail<int> mail): void { return; } end
         pub fnc main(): int { val worker = spawn Worker(); return worker.secret; }",
        "E_PRIVATE",
    );
}
