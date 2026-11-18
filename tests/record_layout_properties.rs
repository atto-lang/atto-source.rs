//! Reproducible property-style Record/Struct interning tests.
//! Logical host shape IDs are not a packed guest ABI or transport serialization.
use atto_compiler::{
    collection_types::LiteralType as T,
    compile,
    record_struct_layout::{decide_access, Error, FieldDecision, FieldType, LayoutInterner},
};

fn named(name: &str, ty: T) -> FieldType {
    (name.to_owned(), ty)
}

fn orders<F: FnMut(&[usize])>(order: &mut [usize], start: usize, f: &mut F) {
    if start == order.len() {
        f(order);
        return;
    }
    for j in start..order.len() {
        order.swap(start, j);
        orders(order, start + 1, f);
        order.swap(start, j);
    }
}

fn shuffled(seed: &mut u64, len: usize) -> Vec<usize> {
    let mut order = (0..len).collect::<Vec<_>>();
    for i in (1..len).rev() {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        order.swap(i, (*seed as usize) % (i + 1));
    }
    order
}

#[test]
fn exhaustive_six_field_orders_have_one_shape_and_one_inverse_slot_mapping() {
    let source = [
        named("", T::Int),
        named("a", T::Bool),
        named("cafe\u{301}", T::String),
        named("café", T::Float),
        named("x-y", T::Int),
        named("🚀", T::Bool),
    ];
    let mut interner = LayoutInterner::new();
    let first = interner.closed_record(&source).unwrap();
    let canonical = interner.shape(first.shape_id).unwrap().fields.clone();
    let mut perm = [0, 1, 2, 3, 4, 5];
    let mut n = 0;
    orders(&mut perm, 0, &mut |positions| {
        let fields = positions.iter().map(|&i| source[i].clone()).collect::<Vec<_>>();
        let plan = interner.closed_record(&fields).unwrap();
        assert_eq!(plan.shape_id, first.shape_id);
        assert_eq!(interner.shape(plan.shape_id).unwrap().fields, canonical);
        for (i, (name, ty)) in fields.iter().enumerate() {
            let slot = plan.source_to_slot[i];
            assert_eq!(canonical[slot], (name.clone(), ty.clone()));
            assert_eq!(interner.shape(plan.shape_id).unwrap().slot(name), Some(slot));
            assert_eq!(decide_access(&T::Record(canonical.clone()), name), FieldDecision::StaticSlot(slot));
        }
        n += 1;
    });
    assert_eq!(n, 720);
    assert_eq!(interner.len(), 1);
}

#[test]
fn seeded_permutations_preserve_type_identity_without_runtime_randomness() {
    let names = ["k0", "k1", "k2", "k3", "k4", "k5", "k6", "k7"];
    let types = [T::Int, T::Bool, T::String, T::Float];
    let mut seed = 0xa771_2026_d15a_b1e5_u64;
    let mut interner = LayoutInterner::new();
    for case in 0..256 {
        let baseline = names
            .iter()
            .enumerate()
            .map(|(i, name)| named(name, types[(i + case) % types.len()].clone()))
            .collect::<Vec<_>>();
        let indices = shuffled(&mut seed, baseline.len());
        let fields = indices.iter().map(|&i| baseline[i].clone()).collect::<Vec<_>>();
        let first = interner.closed_record(&baseline).unwrap();
        let permuted = interner.closed_record(&fields).unwrap();
        assert_eq!(first.shape_id, permuted.shape_id);
        for (i, &index) in indices.iter().enumerate() {
            assert_eq!(permuted.source_to_slot[i], index);
            assert_eq!(fields[i], baseline[index]);
        }
        let mut changed = baseline.clone();
        changed[0].1 = if changed[0].1 == T::Int { T::Bool } else { T::Int };
        assert_ne!(first.shape_id, interner.closed_record(&changed).unwrap().shape_id);
    }
}

#[test]
fn all_five_field_nominal_orders_preserve_declared_slots_and_nominal_identity() {
    let fields = [
        named("a", T::Int),
        named("b", T::Bool),
        named("c", T::String),
        named("d", T::Float),
        named("e", T::Int),
    ];
    let mut interner = LayoutInterner::new();
    let mut perm = [0, 1, 2, 3, 4];
    let mut seen = 0;
    let mut structural_id = None;
    orders(&mut perm, 0, &mut |indices| {
        let source = indices.iter().map(|&i| fields[i].clone()).collect::<Vec<_>>();
        let name = format!("pkg::Struct{seen}");
        let nominal = interner.nominal_struct(&name, &source).unwrap();
        assert_eq!(interner.shape(nominal.shape_id).unwrap().fields, source);
        assert_eq!(nominal.source_to_slot, [0, 1, 2, 3, 4]);
        let record = interner.closed_record(&source).unwrap();
        if let Some(existing) = structural_id {
            assert_eq!(record.shape_id, existing);
        } else {
            structural_id = Some(record.shape_id);
        }
        assert_ne!(record.shape_id, nominal.shape_id);
        seen += 1;
    });
    assert_eq!(seen, 120);
    assert_eq!(interner.len(), 121);
}

#[test]
fn invalid_shapes_never_mutate_the_registry_or_poison_valid_nominals() {
    let mut interner = LayoutInterner::with_shape_limit(2);
    let fields = [named("x", T::Int), named("y", T::Bool)];
    let first = interner.closed_record(&fields).unwrap().shape_id;
    for _ in 0..64 {
        assert_eq!(
            interner.closed_record(&[named("x", T::Int), named("x", T::Bool)]),
            Err(Error::DuplicateField)
        );
        assert_eq!(interner.closed_record(&[named("child", T::OpenRecord)]), Err(Error::UnresolvedType));
        assert_eq!(
            interner.nominal_struct("pkg::S", &[named("not-valid", T::Int)]),
            Err(Error::InvalidStructField)
        );
        assert_eq!(interner.closed_record(&fields).unwrap().shape_id, first);
    }
    let nominal = interner.nominal_struct("pkg::S", &[named("x", T::Int)]).unwrap();
    assert_ne!(nominal.shape_id, first);
    assert_eq!(
        interner.nominal_struct("pkg::S", &[named("x", T::Bool)]),
        Err(Error::ChangedNominalDefinition)
    );
    assert_eq!(interner.closed_record(&[named("z", T::Int)]), Err(Error::TooManyShapes));
    assert_eq!(interner.len(), 2);
}

#[test]
fn twenty_four_guest_compilations_are_deterministic_for_all_four_field_orders() {
    let fields = [("a", 10), ("b", 11), ("c", 12), ("d", 9)];
    let mut perm = [0, 1, 2, 3];
    let mut seen = 0;
    orders(&mut perm, 0, &mut |indices| {
        let init = indices
            .iter()
            .map(|&i| format!("{}: {}", fields[i].0, fields[i].1))
            .collect::<Vec<_>>()
            .join(", ");
        let source = format!("pub fnc main(): int {{ let Record r = {{{init}}}; return r.a + r[\"b\"] + r.c + r[\"d\"]; }}");
        let wasm = compile(&source).unwrap_or_else(|err| panic!("{source}\n{err}"));
        assert!(wasm.starts_with(b"\0asm\x01\0\0\0"));
        assert_eq!(wasm, compile(&source).unwrap());
        seen += 1;
    });
    assert_eq!(seen, 24);
}
