//! Fixed-profile host resource calculation; not guest lowering or dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Affinity { None, Sticky, Pinned }
#[derive(Clone, Copy)]
pub struct Policy {
    pub serial: bool,
    pub carrier_cap: i32,
    pub default_carriers: i32,
    pub live_stack_slots: i64,
    pub heap_limit: i64,
    pub stack_limit: i64,
    pub mailbox_messages_limit: i64,
    pub mailbox_byte_charge: i64,
}
#[derive(Clone, Copy)]
pub struct Defaults {
    pub affinity: Affinity,
    pub heap_bytes: i64,
    pub stack_bytes: i64,
    pub mailbox_messages: i64,
}
#[derive(Clone, Copy, Default)]
pub struct Request {
    pub carriers: Option<i32>,
    pub affinity: Option<Affinity>,
    pub heap_bytes: Option<i64>,
    pub stack_bytes: Option<i64>,
    pub mailbox_messages: Option<i64>,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    pub carrier_cap: i32,
    pub affinity: Affinity,
    pub stack_bytes: i64,
    pub heap: i64,
    pub stacks: i64,
    pub mailbox_bytes: i64,
    pub messages: i64,
}
pub fn resolve(p: Policy, d: Defaults, r: Request) -> Result<Plan, &'static str> {
    if p.carrier_cap <= 0 || p.default_carriers <= 0 || p.default_carriers > p.carrier_cap
        || p.live_stack_slots <= 0 || p.heap_limit <= 0 || p.stack_limit <= 0
        || p.mailbox_messages_limit <= 0 || p.mailbox_byte_charge <= 0
        || (p.serial && (p.carrier_cap != 1 || p.default_carriers != 1)) {
        return Err("E_SPAWN_POLICY");
    }
    let checked = |cap, affinity, heap, stack: i64, messages| {
        if cap <= 0 || cap > p.carrier_cap || (affinity == Affinity::Pinned && cap != 1) {
            return Err("E_SPAWN_POLICY");
        }
        if heap <= 0 || heap > p.heap_limit || stack <= 0 || stack > p.stack_limit
            || messages <= 0 || messages > p.mailbox_messages_limit {
            return Err("E_SPAWN_QUOTA");
        }
        let stacks = stack.checked_mul(p.live_stack_slots).ok_or("E_SPAWN_QUOTA")?;
        Ok(Plan { carrier_cap: cap, affinity, stack_bytes: stack, heap, stacks,
            mailbox_bytes: p.mailbox_byte_charge, messages })
    };
    checked(p.default_carriers, d.affinity, d.heap_bytes, d.stack_bytes, d.mailbox_messages)?;
    checked(r.carriers.unwrap_or(p.default_carriers), r.affinity.unwrap_or(d.affinity),
        r.heap_bytes.unwrap_or(d.heap_bytes), r.stack_bytes.unwrap_or(d.stack_bytes),
        r.mailbox_messages.unwrap_or(d.mailbox_messages))
}

#[cfg(test)]
mod tests {
    use super::*;
    const P: Policy = Policy { serial:false, carrier_cap:4, default_carriers:2,
        live_stack_slots:8, heap_limit:1000, stack_limit:100, mailbox_messages_limit:50,
        mailbox_byte_charge:4096 };
    const D: Defaults = Defaults { affinity:Affinity::Sticky, heap_bytes:500,
        stack_bytes:50, mailbox_messages:10 };
    #[test]
    fn defaults_and_narrowing() {
        assert_eq!(resolve(P,D,Request::default()), Ok(Plan{carrier_cap:2,affinity:Affinity::Sticky,
            stack_bytes:50,heap:500,stacks:400,mailbox_bytes:4096,messages:10}));
        assert_eq!(resolve(P,D,Request{carriers:Some(1),affinity:Some(Affinity::Pinned),
            heap_bytes:Some(300),stack_bytes:Some(20),mailbox_messages:Some(3)}),
            Ok(Plan{carrier_cap:1,affinity:Affinity::Pinned,stack_bytes:20,
                heap:300,stacks:160,mailbox_bytes:4096,messages:3}));
    }
    #[test]
    fn policy_denials() {
        for cap in [0,-1,5] { assert_eq!(resolve(P,D,Request{carriers:Some(cap),..Default::default()}),Err("E_SPAWN_POLICY")); }
        assert_eq!(resolve(P,D,Request{affinity:Some(Affinity::Pinned),..Default::default()}),Err("E_SPAWN_POLICY"));
        let serial=Policy{serial:true,carrier_cap:1,default_carriers:1,..P};
        assert_eq!(resolve(serial,D,Request::default()).unwrap().carrier_cap,1);
        assert_eq!(resolve(serial,D,Request{carriers:Some(2),..Default::default()}),Err("E_SPAWN_POLICY"));
        assert_eq!(resolve(Policy{carrier_cap:2,..serial},D,Request::default()),Err("E_SPAWN_POLICY"));
        assert_eq!(resolve(Policy{mailbox_byte_charge:0,..P},D,Request::default()),Err("E_SPAWN_POLICY"));
    }
    #[test]
    fn resource_denials_and_signed_boundary() {
        for heap in [0,-1,1001] { assert_eq!(resolve(P,D,Request{heap_bytes:Some(heap),..Default::default()}),Err("E_SPAWN_QUOTA")); }
        assert_eq!(resolve(P,D,Request{stack_bytes:Some(101),..Default::default()}),Err("E_SPAWN_QUOTA"));
        assert_eq!(resolve(P,D,Request{mailbox_messages:Some(51),..Default::default()}),Err("E_SPAWN_QUOTA"));
        assert_eq!(resolve(P,Defaults{heap_bytes:0,..D},Request{heap_bytes:Some(10),..Default::default()}),Err("E_SPAWN_QUOTA"));
        let huge=Policy{live_stack_slots:i64::MAX,..P};
        assert_eq!(resolve(huge,D,Request::default()),Err("E_SPAWN_QUOTA"));
        assert_eq!(resolve(huge,Defaults{stack_bytes:1,..D},Request::default()).unwrap().stacks,i64::MAX);
    }
}
