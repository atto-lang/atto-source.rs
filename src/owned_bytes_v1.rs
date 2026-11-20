//! Host-only candidate for sealed OwnedBytes. No Atto guest lowering/linking.
//! Requires trusted caller identity, quota admission and future TenantVM integration.
#![allow(dead_code)]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Owner { tenant: u64, actor: u64, generation: u64 }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fault { Owner, Bounds, Value, Quota, Allocation, Retired }
#[derive(Clone, Copy, Debug)]
pub(crate) enum Endian { Little, Big }
pub(crate) struct OwnedBytes { owner: Owner, data: Vec<u8>, retired: bool }

impl OwnedBytes {
    pub fn allocate_zeroed(owner: Owner, size: i64, limit: usize) -> Result<Self, Fault> {
        if owner.tenant == 0 || owner.actor == 0 || owner.generation == 0 { return Err(Fault::Owner); }
        let size = usize::try_from(size).map_err(|_| Fault::Quota)?;
        if size > limit { return Err(Fault::Quota); }
        let mut data = Vec::new();
        data.try_reserve_exact(size).map_err(|_| Fault::Allocation)?;
        data.resize(size, 0);
        Ok(Self { owner, data, retired: false })
    }
    fn owner_check(&self, caller: Owner) -> Result<(), Fault> {
        if self.retired { return Err(Fault::Retired); }
        if caller != self.owner { return Err(Fault::Owner); }
        Ok(())
    }
    pub fn length(&self, caller: Owner) -> Result<usize, Fault> {
        self.owner_check(caller)?;
        Ok(self.data.len())
    }
    fn check(&self, caller: Owner, offset: i64, width: usize) -> Result<usize, Fault> {
        self.owner_check(caller)?;
        let index = usize::try_from(offset).map_err(|_| Fault::Bounds)?;
        if width > self.data.len() || index > self.data.len() - width { return Err(Fault::Bounds); }
        Ok(index)
    }
    pub fn read(&self, caller: Owner, offset: i64, width: usize, endian: Endian)
        -> Result<i64, Fault>
    {
        if !matches!(width, 1 | 2 | 4) { return Err(Fault::Value); }
        let start = self.check(caller, offset, width)?;
        let mut output = 0u64;
        for i in 0..width {
            let shift = 8 * match endian { Endian::Little => i, Endian::Big => width - i - 1 };
            output |= u64::from(self.data[start + i]) << shift;
        }
        Ok(output as i64)
    }
    pub fn write(&mut self, caller: Owner, offset: i64, width: usize,
                 value: i64, endian: Endian) -> Result<(), Fault>
    {
        if !matches!(width, 1 | 2 | 4) { return Err(Fault::Value); }
        let start = self.check(caller, offset, width)?;
        if value < 0 || (value as u64) > ((1u64 << (width * 8)) - 1) {
            return Err(Fault::Value);
        }
        // Fully validate before any mutation; no allocation or suspension.
        for i in 0..width {
            let shift = 8 * match endian { Endian::Little => i, Endian::Big => width - i - 1 };
            self.data[start + i] = (((value as u64) >> shift) & 255) as u8;
        }
        Ok(())
    }
    pub fn retire(&mut self, caller: Owner) -> Result<(), Fault> {
        self.owner_check(caller)?;
        self.data.fill(0);
        self.retired = true;
        Ok(())
    }
}
impl Drop for OwnedBytes {
    fn drop(&mut self) { self.data.fill(0); }
}
#[cfg(test)]
mod tests {
    use super::*;
    const A: Owner = Owner { tenant: 1, actor: 2, generation: 3 };
    const B: Owner = Owner { tenant: 9, actor: 2, generation: 3 };
    #[test] fn roundtrip_and_endian() {
        for order in [Endian::Little, Endian::Big] {
            let mut b = OwnedBytes::allocate_zeroed(A, 8, 8).unwrap();
            b.write(A, 1, 4, 0x12345678, order).unwrap();
            assert_eq!(b.read(A, 1, 4, order), Ok(0x12345678));
            assert_eq!(b.read(A, 5, 1, order), Ok(0));
            let expected: &[u8] = match order {
                Endian::Little => &[0x78, 0x56, 0x34, 0x12],
                Endian::Big => &[0x12, 0x34, 0x56, 0x78],
            };
            assert_eq!(&b.data[1..5], expected);
        }
    }
    #[test] fn fail_atomic_and_bad_offsets() {
        let mut b = OwnedBytes::allocate_zeroed(A, 8, 8).unwrap();
        b.write(A, 0, 4, 0x12345678, Endian::Little).unwrap();
        let original = b.data.clone();
        for offset in [-1, 5, i64::MAX, i64::MIN] {
            assert_eq!(b.write(A, offset, 4, 42, Endian::Little), Err(Fault::Bounds));
            assert_eq!(b.data, original);
        }
        for value in [-1, 4294967296, i64::MAX] {
            assert_eq!(b.write(A, 0, 4, value, Endian::Big), Err(Fault::Value));
            assert_eq!(b.data, original);
        }
    }
    #[test] fn owner_quota_retire() {
        assert!(matches!(OwnedBytes::allocate_zeroed(A, -1, 8), Err(Fault::Quota)));
        assert!(matches!(OwnedBytes::allocate_zeroed(A, 9, 8), Err(Fault::Quota)));
        let mut b = OwnedBytes::allocate_zeroed(A, 4, 8).unwrap();
        assert_eq!(b.read(B, 0, 1, Endian::Little), Err(Fault::Owner));
        assert_eq!(b.write(B, 0, 1, 1, Endian::Little), Err(Fault::Owner));
        b.retire(A).unwrap();
        assert_eq!(b.data, vec![0; 4]);
        assert_eq!(b.read(A, 0, 1, Endian::Little), Err(Fault::Retired));
        assert_eq!(b.retire(A), Err(Fault::Retired));
        let z = OwnedBytes::allocate_zeroed(A, 0, 0).unwrap();
        assert_eq!(z.length(A), Ok(0));
        assert_eq!(z.read(A, 0, 1, Endian::Little), Err(Fault::Bounds));
    }
    #[test] fn stress_100k_roundtrips() {
        let mut b = OwnedBytes::allocate_zeroed(A, 12, 12).unwrap();
        let mut seed = 0x9e3779b97f4a7c15u64;
        for _ in 0..100_000 {
            seed ^= seed << 7;
            seed ^= seed >> 9;
            seed ^= seed << 8;
            let v = (seed & 0xffff_ffff) as i64;
            let order = if seed & 1 == 0 { Endian::Little } else { Endian::Big };
            b.write(A, 3, 4, v, order).unwrap();
            assert_eq!(b.read(A, 3, 4, order), Ok(v));
        }
    }
}
