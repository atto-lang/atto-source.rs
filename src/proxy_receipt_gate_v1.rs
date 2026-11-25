//! Trusted-runtime proxy ACK stage gate (v1).
//! This module does not authenticate frames, send IPC, implement the Atto
//! language API, or claim compiled guest parity. A trusted transport must
//! verify destination/provider authority before calling apply_verified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiptIdentityV1 {
    pub protocol_major: u32,
    pub tenant: u64,
    pub caller_incarnation: u64,
    pub target_actor: u64,
    pub target_incarnation: u64,
    pub association_generation: u64,
    pub placement_epoch: u64,
    pub message_id: u64,
    pub attempt: u64,
}
impl ReceiptIdentityV1 {
    fn valid(self) -> bool {
        self.protocol_major == 1 && self.tenant != 0
            && self.caller_incarnation != 0 && self.target_actor != 0
            && self.target_incarnation != 0 && self.association_generation != 0
            && self.placement_epoch != 0
            && self.message_id != 0 && self.attempt != 0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptEventV1 {
    Admitted, Started, Processed, DurableInbox, DurableResult,
    Rejected, Failed, Indeterminate,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReceiptStageV1 { Dispatched, Admitted, Started, Processed }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptOutcomeV1 { Pending, Rejected, Failed, Indeterminate }

#[derive(Debug)]
pub struct ProxyReceiptGateV1 {
    expected: ReceiptIdentityV1,
    stage: ReceiptStageV1,
    outcome: ReceiptOutcomeV1,
    durable_inbox: bool,
    durable_result: bool,
}
impl ProxyReceiptGateV1 {
    pub fn new(expected: ReceiptIdentityV1) -> Self {
        Self { expected, stage: ReceiptStageV1::Dispatched,
            outcome: ReceiptOutcomeV1::Pending, durable_inbox: false,
            durable_result: false }
    }
    pub fn stage(&self) -> ReceiptStageV1 { self.stage }
    pub fn outcome(&self) -> ReceiptOutcomeV1 { self.outcome }
    pub fn destination_admitted(&self) -> bool { self.stage >= ReceiptStageV1::Admitted }
    pub fn durable_inbox(&self) -> bool { self.durable_inbox }
    pub fn durable_result(&self) -> bool { self.durable_result }

    /// Caller obligation: authenticate and authorize observed receipt against
    /// the real target mailbox/turn or durable provider first. This gate only
    /// checks correlation and stage semantics. It is NOT the verification step.
    pub fn apply_verified(&mut self, observed: ReceiptIdentityV1,
                          event: ReceiptEventV1) -> bool {
        if !self.expected.valid() || observed != self.expected
            || self.outcome != ReceiptOutcomeV1::Pending { return false; }
        match event {
            ReceiptEventV1::Admitted => {
                self.stage = self.stage.max(ReceiptStageV1::Admitted); true
            }
            ReceiptEventV1::Started => {
                if self.stage < ReceiptStageV1::Admitted { return false; }
                self.stage = self.stage.max(ReceiptStageV1::Started); true
            }
            ReceiptEventV1::Processed => {
                if self.stage < ReceiptStageV1::Started { return false; }
                self.stage = ReceiptStageV1::Processed; true
            }
            ReceiptEventV1::DurableInbox => {
                if self.stage < ReceiptStageV1::Admitted { return false; }
                self.durable_inbox = true; true
            }
            ReceiptEventV1::DurableResult => {
                if self.stage != ReceiptStageV1::Processed { return false; }
                self.durable_result = true; true
            }
            ReceiptEventV1::Rejected => {
                if self.stage != ReceiptStageV1::Dispatched { return false; }
                self.outcome = ReceiptOutcomeV1::Rejected; true
            }
            ReceiptEventV1::Failed => {
                if self.stage == ReceiptStageV1::Dispatched
                    || self.stage == ReceiptStageV1::Processed { return false; }
                self.outcome = ReceiptOutcomeV1::Failed; true
            }
            ReceiptEventV1::Indeterminate => {
                if self.stage == ReceiptStageV1::Processed { return false; }
                self.outcome = ReceiptOutcomeV1::Indeterminate; true
            }
        }
    }
}
