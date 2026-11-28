//! Actual owned scalar-record memory for a staged collection-layout runtime.
//!
//! `I64RecordArray` materializes AoS records, SoA field columns and AoSoA
//! blocks. This is an executable RUST storage kernel, NOT Atto-compiled guest
//! execution. Source-level Array remains fail-closed until type checking,
//! lowering, ownership and guest-Wasm emission are integrated.

use crate::collection_layout_plan::{Plan, PlanError, ScalarField, Storage};

/// 16 Wasm pages: upper bound only. A guest emitter must also budget other data.
const MAX_STORAGE_BYTES: u64 = 16 * 65_536;
const MAX_FIELDS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageError {
    Plan(PlanError),
    Capacity,
    Shape,
    Bounds,
    Overflow,
}
impl From<PlanError> for StorageError {
    fn from(value: PlanError) -> Self { Self::Plan(value) }
}

/// For the first storage profile every nominal struct field is an i64.
/// A single aligned allocation contains ALL active cells, with the physical
/// order determined by Plan. No detached row copies or aliased field vectors.
#[derive(Clone, Debug)]
pub struct I64RecordArray {
    plan: Plan,
    words: Vec<i64>,
}
impl I64RecordArray {
    pub fn new(layout: Storage, field_count: usize, len: u64) -> Result<Self, StorageError> {
        // Prevent an untrusted field_count from allocating before Plan checks it.
        if field_count == 0 || field_count > MAX_FIELDS {
            return Err(StorageError::Plan(PlanError::InvalidField));
        }
        let plan = Plan::new(layout, vec![ScalarField { size: 8, align: 8 }; field_count], len)?;
        if plan.total_bytes > MAX_STORAGE_BYTES { return Err(StorageError::Capacity); }
        let count = usize::try_from(plan.total_bytes / 8).map_err(|_| StorageError::Capacity)?;
        let mut words = Vec::new();
        words.try_reserve_exact(count).map_err(|_| StorageError::Capacity)?;
        words.resize(count, 0);
        Ok(Self { plan, words })
    }

    /// Transpose evaluated logical rows exactly once in row-major input order.
    /// The caller retains responsibility for evaluating guest expressions once.
    pub fn from_rows(layout: Storage, field_count: usize, rows: &[Vec<i64>]) -> Result<Self, StorageError> {
        if rows.iter().any(|r| r.len() != field_count) { return Err(StorageError::Shape); }
        let len = u64::try_from(rows.len()).map_err(|_| StorageError::Capacity)?;
        let mut array = Self::new(layout, field_count, len)?;
        for (row, values) in rows.iter().enumerate() {
            for (field, value) in values.iter().enumerate() {
                array.set(row as u64, field, *value)?;
            }
        }
        Ok(array)
    }

    pub fn plan(&self) -> &Plan { &self.plan }
    pub fn len(&self) -> u64 { self.plan.len }
    pub fn is_empty(&self) -> bool { self.plan.len == 0 }

    fn word_index(&self, row: u64, field: usize) -> Result<usize, StorageError> {
        let offset = self.plan.offset(row, field).ok_or(StorageError::Bounds)?;
        usize::try_from(offset / 8).map_err(|_| StorageError::Bounds)
    }
    pub fn get(&self, row: u64, field: usize) -> Result<i64, StorageError> {
        let index = self.word_index(row, field)?;
        self.words.get(index).copied().ok_or(StorageError::Bounds)
    }
    pub fn set(&mut self, row: u64, field: usize, value: i64) -> Result<(), StorageError> {
        let index = self.word_index(row, field)?;
        *self.words.get_mut(index).ok_or(StorageError::Bounds)? = value;
        Ok(())
    }
    pub fn add_checked(&mut self, row: u64, field: usize, delta: i64) -> Result<(), StorageError> {
        let index = self.word_index(row, field)?;
        let slot = self.words.get_mut(index).ok_or(StorageError::Bounds)?;
        let result = slot.checked_add(delta).ok_or(StorageError::Overflow)?;
        *slot = result;
        Ok(())
    }
    pub fn row(&self, row: u64) -> Result<Row<'_>, StorageError> {
        if row >= self.len() { return Err(StorageError::Bounds); }
        Ok(Row { owner: self, row })
    }
    pub fn row_mut(&mut self, row: u64) -> Result<RowMut<'_>, StorageError> {
        if row >= self.len() { return Err(StorageError::Bounds); }
        Ok(RowMut { owner: self, row })
    }
    pub fn column(&self, field: usize) -> Result<Column<'_>, StorageError> {
        if field >= self.plan.fields.len() { return Err(StorageError::Bounds); }
        Ok(Column { owner: self, field })
    }
    pub fn column_mut(&mut self, field: usize) -> Result<ColumnMut<'_>, StorageError> {
        if field >= self.plan.fields.len() { return Err(StorageError::Bounds); }
        Ok(ColumnMut { owner: self, field })
    }

    /// Internal inspection only. Not a safe AttoLang-level raw-storage API.
    pub fn physical_words(&self) -> &[i64] { &self.words }

    fn soa_range(&self, field: usize) -> Option<std::ops::Range<usize>> {
        if !matches!(self.plan.layout, Storage::Soa) { return None; }
        let begin = usize::try_from(self.plan.field_offsets.get(field)?.checked_div(8)?).ok()?;
        let len = usize::try_from(self.plan.len).ok()?;
        let end = begin.checked_add(len)?;
        (end <= self.words.len()).then_some(begin..end)
    }
}

/// Host-side owned Array slot: shallow copying duplicates only the outer
/// slot and retains the same allocation. Every alias loses write authority
/// until the last competing slot has detached or dropped. This is a local
/// Rust reference model for future guest lowering, not an actor-transfer
/// capability or a guest ABI. No implicit copy-on-write.
#[derive(Debug)]
pub struct I64ArraySlot {
    storage: std::rc::Rc<I64RecordArray>,
    replaceable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArraySlotError {
    ReadOnly,
    FixedSlot,
    Shape,
    Storage(StorageError),
}
impl From<StorageError> for ArraySlotError {
    fn from(value: StorageError) -> Self { Self::Storage(value) }
}

impl I64ArraySlot {
    pub fn new(storage: I64RecordArray, replaceable: bool) -> Self {
        Self { storage: std::rc::Rc::new(storage), replaceable }
    }

    /// Explicit independent outer slot, retaining exactly one backing identity.
    /// Rust's Rc is deliberately local (not Send/Sync); no Weak or Rc escapes.
    pub fn shallow_copy(&self) -> Self {
        Self { storage: std::rc::Rc::clone(&self.storage), replaceable: self.replaceable }
    }

    /// Explicit deep allocation: construct independent bounded storage first,
    /// then copy its complete physical contents, including any layout padding.
    pub fn deep_copy(&self) -> Result<Self, ArraySlotError> {
        let source = &*self.storage;
        let mut owned = I64RecordArray::new(source.plan.layout, source.plan.fields.len(), source.len())?;
        owned.words.copy_from_slice(&source.words);
        Ok(Self::new(owned, self.replaceable))
    }

    pub fn len(&self) -> u64 { self.storage.len() }
    pub fn is_empty(&self) -> bool { self.storage.is_empty() }
    pub fn get(&self, row: u64, field: usize) -> Result<i64, ArraySlotError> {
        self.storage.get(row, field).map_err(Into::into)
    }

    /// Keep the old allocation and its ownership intact on denial. The
    /// caller supplies a separately owned array; no arbitrary alias injection.
    /// A replacement may change logical length but not the physical schema.
    pub fn replace(&mut self, fresh: I64RecordArray) -> Result<(), ArraySlotError> {
        if !self.replaceable { return Err(ArraySlotError::FixedSlot); }
        if self.storage.plan.layout != fresh.plan.layout ||
            self.storage.plan.fields != fresh.plan.fields {
            return Err(ArraySlotError::Shape);
        }
        self.storage = std::rc::Rc::new(fresh);
        Ok(())
    }

    pub fn set(&mut self, row: u64, field: usize, value: i64) -> Result<(), ArraySlotError> {
        std::rc::Rc::get_mut(&mut self.storage)
            .ok_or(ArraySlotError::ReadOnly)?.set(row, field, value)?;
        Ok(())
    }

    pub fn add_checked(&mut self, row: u64, field: usize, delta: i64) -> Result<(), ArraySlotError> {
        std::rc::Rc::get_mut(&mut self.storage)
            .ok_or(ArraySlotError::ReadOnly)?.add_checked(row, field, delta)?;
        Ok(())
    }

    /// Lexical read projections retain Rust's borrow of the outer slot.
    pub fn row(&self, row: u64) -> Result<Row<'_>, ArraySlotError> {
        self.storage.row(row).map_err(Into::into)
    }
    pub fn column(&self, field: usize) -> Result<Column<'_>, ArraySlotError> {
        self.storage.column(field).map_err(Into::into)
    }
    pub fn row_mut(&mut self, row: u64) -> Result<RowMut<'_>, ArraySlotError> {
        std::rc::Rc::get_mut(&mut self.storage)
            .ok_or(ArraySlotError::ReadOnly)?.row_mut(row).map_err(Into::into)
    }
    pub fn column_mut(&mut self, field: usize) -> Result<ColumnMut<'_>, ArraySlotError> {
        std::rc::Rc::get_mut(&mut self.storage)
            .ok_or(ArraySlotError::ReadOnly)?.column_mut(field).map_err(Into::into)
    }
}

pub struct Row<'a> { owner: &'a I64RecordArray, row: u64 }
impl Row<'_> {
    pub fn get(&self, field: usize) -> Result<i64, StorageError> {
        self.owner.get(self.row, field)
    }
}
pub struct RowMut<'a> { owner: &'a mut I64RecordArray, row: u64 }
impl RowMut<'_> {
    pub fn get(&self, field: usize) -> Result<i64, StorageError> {
        self.owner.get(self.row, field)
    }
    pub fn set(&mut self, field: usize, value: i64) -> Result<(), StorageError> {
        self.owner.set(self.row, field, value)
    }
    pub fn add_checked(&mut self, field: usize, delta: i64) -> Result<(), StorageError> {
        self.owner.add_checked(self.row, field, delta)
    }
}
pub struct Column<'a> { owner: &'a I64RecordArray, field: usize }
impl Column<'_> {
    pub fn get(&self, row: u64) -> Result<i64, StorageError> {
        self.owner.get(row, self.field)
    }
    /// Only SoA has contiguous field storage; AoS/AoSoA are not rebranded.
    pub fn as_contiguous_slice(&self) -> Option<&[i64]> {
        self.owner.words.get(self.owner.soa_range(self.field)?)
    }
}
pub struct ColumnMut<'a> { owner: &'a mut I64RecordArray, field: usize }
impl ColumnMut<'_> {
    pub fn get(&self, row: u64) -> Result<i64, StorageError> {
        self.owner.get(row, self.field)
    }
    pub fn set(&mut self, row: u64, value: i64) -> Result<(), StorageError> {
        self.owner.set(row, self.field, value)
    }
    pub fn add_checked(&mut self, row: u64, delta: i64) -> Result<(), StorageError> {
        self.owner.add_checked(row, self.field, delta)
    }
    pub fn as_contiguous_slice_mut(&mut self) -> Option<&mut [i64]> {
        let range = self.owner.soa_range(self.field)?;
        self.owner.words.get_mut(range)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn particles() -> Vec<Vec<i64>> { vec![vec![10, 20, 1], vec![30, 40, 2]] }

    #[test]
    fn real_soa_columns_return_fifteen_through_borrowed_row_and_column() {
        let mut array = I64RecordArray::from_rows(Storage::Soa, 3, &particles()).unwrap();
        assert_eq!(array.physical_words(), &[10, 30, 20, 40, 1, 2]);
        array.row_mut(0).unwrap().add_checked(0, 5).unwrap();
        assert_eq!(array.column(0).unwrap().get(0), Ok(15));
        assert_eq!(array.column(0).unwrap().as_contiguous_slice(), Some(&[15, 30][..]));
        assert_eq!(array.physical_words(), &[15, 30, 20, 40, 1, 2]);
    }

    #[test]
    fn aos_is_row_major_and_columns_are_not_a_contiguous_slice() {
        let array = I64RecordArray::from_rows(Storage::Aos, 3, &particles()).unwrap();
        assert_eq!(array.physical_words(), &[10, 20, 1, 30, 40, 2]);
        assert_eq!(array.row(1).unwrap().get(2), Ok(2));
        assert!(array.column(0).unwrap().as_contiguous_slice().is_none());
    }

    #[test]
    fn aosoa_partial_tail_is_padded_but_never_addressable() {
        let rows = (1..=5).map(|x| vec![x, x * 10, x * 100]).collect::<Vec<_>>();
        let mut array = I64RecordArray::from_rows(Storage::Aosoa(4), 3, &rows).unwrap();
        assert_eq!(array.physical_words(), &[
            1, 2, 3, 4, 10, 20, 30, 40, 100, 200, 300, 400,
            5, 0, 0, 0, 50, 0, 0, 0, 500, 0, 0, 0
        ]);
        assert_eq!(array.row(4).unwrap().get(0), Ok(5));
        assert_eq!(array.get(5, 0), Err(StorageError::Bounds));
        assert!(array.column(0).unwrap().as_contiguous_slice().is_none());
        array.column_mut(1).unwrap().set(4, 70).unwrap();
        assert_eq!(array.row(4).unwrap().get(1), Ok(70));
    }

    #[test]
    fn mutable_column_projections_modify_same_storage() {
        let mut array = I64RecordArray::from_rows(Storage::Soa, 3, &particles()).unwrap();
        {
            let mut column = array.column_mut(0).unwrap();
            column.as_contiguous_slice_mut().unwrap()[0] = 77;
        }
        assert_eq!(array.row(0).unwrap().get(0), Ok(77));
    }

    #[test]
    fn rejects_shape_bounds_capacity_and_checked_arithmetic_overflow() {
        assert!(matches!(I64RecordArray::from_rows(Storage::Soa, 3, &[vec![1, 2]]), Err(StorageError::Shape)));
        let mut array = I64RecordArray::new(Storage::Soa, 2, 1).unwrap();
        assert_eq!(array.set(1, 0, 1), Err(StorageError::Bounds));
        assert_eq!(array.get(0, 2), Err(StorageError::Bounds));
        array.set(0, 0, i64::MAX).unwrap();
        assert_eq!(array.add_checked(0, 0, 1), Err(StorageError::Overflow));
        assert_eq!(array.get(0, 0), Ok(i64::MAX));
        assert!(matches!(I64RecordArray::new(Storage::Soa, 3, 200_000), Err(StorageError::Capacity)));
        assert!(matches!(I64RecordArray::new(Storage::Aosoa(0), 3, 1), Err(StorageError::Plan(PlanError::InvalidBlock))));
        assert!(matches!(I64RecordArray::new(Storage::Soa, usize::MAX, 1), Err(StorageError::Plan(PlanError::InvalidField))));
    }

    #[test]
    fn independent_copies_and_empty_arrays() {
        let mut original = I64RecordArray::from_rows(Storage::Soa, 3, &particles()).unwrap();
        let mut copy = original.clone();
        copy.set(0, 0, 99).unwrap();
        assert_eq!(original.get(0, 0), Ok(10));
        original.set(0, 0, -3).unwrap();
        assert_eq!(copy.get(0, 0), Ok(99));
        let empty = I64RecordArray::new(Storage::Aosoa(16), 3, 0).unwrap();
        assert!(empty.is_empty());
        assert_eq!(empty.get(0, 0), Err(StorageError::Bounds));
    }
}

#[cfg(test)]
mod array_slot_tests {
    use super::*;

    fn storage(layout: Storage, values: &[i64]) -> I64RecordArray {
        I64RecordArray::from_rows(layout, 1,
            &values.iter().map(|x| vec![*x]).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn shared_array_detachment_is_symmetric_for_every_layout() {
        for layout in [Storage::Aos, Storage::Soa, Storage::Aosoa(4)] {
            let mut a = I64ArraySlot::new(storage(layout, &[20, 7, 8]), true);
            let mut b = a.shallow_copy();
            let mut c = b.shallow_copy();
            assert_eq!(a.set(0, 0, 21), Err(ArraySlotError::ReadOnly));
            assert_eq!(b.add_checked(0, 0, 1), Err(ArraySlotError::ReadOnly));
            assert!(matches!(c.row_mut(0), Err(ArraySlotError::ReadOnly)));
            assert!(matches!(a.column_mut(0), Err(ArraySlotError::ReadOnly)));
            b.replace(storage(layout, &[20, 9])).unwrap();
            b.set(0, 0, 22).unwrap();
            assert_eq!(a.get(0, 0), Ok(20));
            assert_eq!(c.get(0, 0), Ok(20));
            assert_eq!(b.get(0, 0), Ok(22));
            assert_eq!(a.len(), 3);
            assert_eq!(b.len(), 2);
            assert_eq!(a.set(0, 0, 9), Err(ArraySlotError::ReadOnly));
            assert_eq!(c.set(0, 0, 9), Err(ArraySlotError::ReadOnly));
            a.replace(storage(layout, &[30])).unwrap();
            a.set(0, 0, 31).unwrap();
            c.row_mut(0).unwrap().set(0, 32).unwrap();
            assert_eq!(c.get(0, 0), Ok(32));
            assert_eq!(a.get(0, 0), Ok(31));
            assert_eq!(b.get(0, 0), Ok(22));
        }
    }

    #[test]
    fn deep_copy_and_scope_release_do_not_mutate_shared_storage() {
        let mut a = I64ArraySlot::new(storage(Storage::Soa, &[20, 1]), true);
        {
            let b = a.shallow_copy();
            let mut deep = b.deep_copy().unwrap();
            deep.column_mut(0).unwrap().as_contiguous_slice_mut().unwrap()[0] = 22;
            assert_eq!(a.get(0, 0), Ok(20));
            assert_eq!(b.get(0, 0), Ok(20));
            assert_eq!(deep.get(0, 0), Ok(22));
            assert_eq!(a.set(0, 0, 5), Err(ArraySlotError::ReadOnly));
        }
        a.add_checked(0, 0, 2).unwrap();
        assert_eq!(a.row(0).unwrap().get(0), Ok(22));
    }

    #[test]
    fn denied_replacements_do_not_release_aliases_and_fixed_slots_stay_fixed() {
        let mut a = I64ArraySlot::new(storage(Storage::Aos, &[20]), true);
        let mut b = a.shallow_copy();
        assert_eq!(b.replace(storage(Storage::Soa, &[30])), Err(ArraySlotError::Shape));
        let wrong_schema = I64RecordArray::from_rows(Storage::Aos, 2, &[vec![1, 2]]).unwrap();
        assert_eq!(b.replace(wrong_schema), Err(ArraySlotError::Shape));
        assert_eq!(a.set(0, 0, 7), Err(ArraySlotError::ReadOnly));
        assert_eq!(b.get(0, 0), Ok(20));
        drop(b);
        a.set(0, 0, 7).unwrap();
        let mut fixed = I64ArraySlot::new(storage(Storage::Aos, &[1]), false);
        let mut fixed_copy = fixed.shallow_copy();
        assert_eq!(fixed.replace(storage(Storage::Aos, &[2])), Err(ArraySlotError::FixedSlot));
        assert_eq!(fixed_copy.replace(storage(Storage::Aos, &[3])), Err(ArraySlotError::FixedSlot));
        drop(fixed_copy);
        fixed.set(0, 0, 4).unwrap();
        assert_eq!(fixed.get(0, 0), Ok(4));
    }

    #[test]
    fn bounds_and_overflow_do_not_mutate_owned_storage() {
        let mut a = I64ArraySlot::new(storage(Storage::Aosoa(4), &[i64::MAX]), true);
        assert_eq!(a.set(1, 0, 42), Err(ArraySlotError::Storage(StorageError::Bounds)));
        assert_eq!(a.add_checked(0, 0, 1), Err(ArraySlotError::Storage(StorageError::Overflow)));
        assert_eq!(a.get(0, 0), Ok(i64::MAX));
        a.replace(storage(Storage::Aosoa(4), &[])).unwrap();
        assert!(a.is_empty());
        assert_eq!(a.get(0, 0), Err(ArraySlotError::Storage(StorageError::Bounds)));
        assert_eq!(a.deep_copy().unwrap().len(), 0);
    }
}


/// Host-only reference model for scalar Map ownership. Copying a Map
/// duplicates its entry table; copying a containing field retains its
/// identity. This is not guest lowering or an Atto contextual allocator.
pub mod scalar_map_ownership {
    use std::{collections::BTreeMap, rc::Rc};
    const MAX_ENTRIES: usize = 64;
    const MAX_KEY_BYTES: usize = 256;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum MapError { Capacity, KeySize, DuplicateKey, ReadOnly, FixedSlot }

    #[derive(Debug, Default)]
    pub struct ScalarMap { entries: BTreeMap<String, i64> }

    impl ScalarMap {
        pub fn from_entries(entries: &[(&str, i64)]) -> Result<Self, MapError> {
            if entries.len() > MAX_ENTRIES { return Err(MapError::Capacity); }
            let mut result = Self::default();
            for &(key, value) in entries {
                if result.entries.contains_key(key) { return Err(MapError::DuplicateKey); }
                result.insert(key, value)?;
            }
            Ok(result)
        }
        pub fn len(&self) -> usize { self.entries.len() }
        pub fn is_empty(&self) -> bool { self.entries.is_empty() }
        pub fn get(&self, key: &str) -> Option<i64> { self.entries.get(key).copied() }
        pub fn insert(&mut self, key: &str, value: i64) -> Result<Option<i64>, MapError> {
            if key.len() > MAX_KEY_BYTES { return Err(MapError::KeySize); }
            if let Some(old) = self.entries.get_mut(key) {
                return Ok(Some(std::mem::replace(old, value)));
            }
            if self.entries.len() >= MAX_ENTRIES { return Err(MapError::Capacity); }
            self.entries.insert(key.to_owned(), value);
            Ok(None)
        }
        pub fn remove(&mut self, key: &str) -> Option<i64> { self.entries.remove(key) }
        pub fn copy_entries(&self) -> Self { Self { entries: self.entries.clone() } }
    }

    /// Local, non-Send field slot. No backing Rc, Weak, or mutable reference
    /// escapes this API. Replacement never mutates the retained allocation.
    #[derive(Debug)]
    pub struct MapFieldSlot {
        map: Rc<ScalarMap>,
        replaceable: bool,
        referent_mutable: bool,
    }
    impl MapFieldSlot {
        pub fn new(map: ScalarMap, replaceable: bool, referent_mutable: bool) -> Self {
            Self { map: Rc::new(map), replaceable, referent_mutable }
        }
        pub fn shallow_copy(&self) -> Self {
            Self { map: Rc::clone(&self.map), replaceable: self.replaceable,
                referent_mutable: self.referent_mutable }
        }
        pub fn deep_copy(&self) -> Self {
            Self::new(self.map.copy_entries(), self.replaceable, self.referent_mutable)
        }
        pub fn get(&self, key: &str) -> Option<i64> { self.map.get(key) }
        pub fn len(&self) -> usize { self.map.len() }
        pub fn is_empty(&self) -> bool { self.map.is_empty() }
        pub fn replace(&mut self, fresh: ScalarMap) -> Result<(), MapError> {
            if !self.replaceable { return Err(MapError::FixedSlot); }
            self.map = Rc::new(fresh);
            Ok(())
        }
        fn unique_mut(&mut self) -> Result<&mut ScalarMap, MapError> {
            if !self.referent_mutable { return Err(MapError::ReadOnly); }
            Rc::get_mut(&mut self.map).ok_or(MapError::ReadOnly)
        }
        pub fn insert(&mut self, key: &str, value: i64) -> Result<Option<i64>, MapError> {
            self.unique_mut()?.insert(key, value)
        }
        pub fn remove(&mut self, key: &str) -> Result<Option<i64>, MapError> {
            Ok(self.unique_mut()?.remove(key))
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn initial() -> ScalarMap { ScalarMap::from_entries(&[("old", 20)]).unwrap() }

        #[test]
        fn direct_map_copy_has_independent_presence_and_values() {
            let mut a = ScalarMap::from_entries(&[("old", 1)]).unwrap();
            let mut b = a.copy_entries();
            assert_eq!(a.insert("left", 10), Ok(None));
            assert_eq!(b.insert("right", 20), Ok(None));
            assert_eq!(a.insert("left", 20), Ok(Some(10)));
            assert_eq!(b.insert("right", 22), Ok(Some(20)));
            assert_eq!(a.get("right"), None);
            assert_eq!(b.get("left"), None);
            assert_eq!(a.get("left").unwrap() + b.get("right").unwrap(), 42);
            assert_eq!(a.remove("old"), Some(1));
            assert_eq!(b.get("old"), Some(1));
        }

        #[test]
        fn containing_copy_denies_all_shared_mutations() {
            let mut a = MapFieldSlot::new(initial(), true, true);
            let mut b = a.shallow_copy();
            let mut c = b.shallow_copy();
            for slot in [&mut a, &mut b, &mut c] {
                assert_eq!(slot.insert("old", 99), Err(MapError::ReadOnly));
                assert_eq!(slot.insert("new", 99), Err(MapError::ReadOnly));
                assert_eq!(slot.remove("old"), Err(MapError::ReadOnly));
                assert_eq!(slot.get("old"), Some(20));
                assert_eq!(slot.get("new"), None);
                assert_eq!(slot.len(), 1);
            }
            b.replace(initial()).unwrap();
            b.insert("old", 22).unwrap();
            assert_eq!(a.get("old").unwrap() + b.get("old").unwrap() + c.get("old").unwrap(), 62);
            assert_eq!(a.insert("new", 99), Err(MapError::ReadOnly));
            assert_eq!(c.insert("new", 99), Err(MapError::ReadOnly));
            drop(c);
            assert_eq!(a.insert("new", 99), Ok(None));
            assert_eq!(b.get("new"), None);
        }

        #[test]
        fn slot_and_referent_mutability_are_independent() {
            let mut fixed = MapFieldSlot::new(initial(), false, true);
            let mut alias = fixed.shallow_copy();
            assert_eq!(fixed.replace(initial()), Err(MapError::FixedSlot));
            assert_eq!(alias.replace(initial()), Err(MapError::FixedSlot));
            assert_eq!(fixed.insert("old", 7), Err(MapError::ReadOnly));
            drop(alias);
            assert_eq!(fixed.insert("old", 7), Ok(Some(20)));
            let mut readonly = MapFieldSlot::new(initial(), true, false);
            readonly.replace(initial()).unwrap();
            assert_eq!(readonly.insert("old", 7), Err(MapError::ReadOnly));
            assert_eq!(readonly.remove("old"), Err(MapError::ReadOnly));
        }

        #[test]
        fn deep_copy_preserves_existing_shared_denial() {
            let mut a = MapFieldSlot::new(initial(), true, true);
            let b = a.shallow_copy();
            let mut c = b.deep_copy();
            c.insert("old", 22).unwrap();
            assert_eq!(a.get("old"), Some(20));
            assert_eq!(b.get("old"), Some(20));
            assert_eq!(c.get("old"), Some(22));
            assert_eq!(a.insert("old", 42), Err(MapError::ReadOnly));
            drop(b);
            assert_eq!(a.insert("old", 42), Ok(Some(20)));
            assert_eq!(c.get("old"), Some(22));
        }

        #[test]
        fn bounded_keys_capacity_and_duplicate_denials() {
            assert!(matches!(ScalarMap::from_entries(&[("a", 1), ("a", 2)]), Err(MapError::DuplicateKey)));
            let mut a = ScalarMap::default();
            assert_eq!(a.insert(&"é".repeat(MAX_KEY_BYTES / 2 + 1), 1), Err(MapError::KeySize));
            for i in 0..MAX_ENTRIES { a.insert(&i.to_string(), i as i64).unwrap(); }
            assert_eq!(a.insert("overflow", 1), Err(MapError::Capacity));
            assert_eq!(a.insert("0", 42), Ok(Some(0)));
            assert_eq!(a.len(), MAX_ENTRIES);
            assert_eq!(a.get("0"), Some(42));
            assert_eq!(a.get("overflow"), None);
            assert_eq!(a.remove("1"), Some(1));
            assert_eq!(a.insert("new", 22), Ok(None));
            assert_eq!(a.len(), MAX_ENTRIES);
        }
    }
}
