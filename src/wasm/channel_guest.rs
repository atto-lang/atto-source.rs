//! Checked Channel<int> v1 guest Wasm lowering, only for typed compiler IR.
//! All operations return a checked ADT; no EOF sentinel or raw host pointers.
use super::*;

impl Emit<'_> {
    fn channel_alloc(&mut self,local:u32) {
        self.i32c(16);self.call(self.l.helper(H_ALLOC));self.set(local);
    }
    fn channel_store_tag(&mut self,ptr:u32,tag:i32) {
        self.get(ptr);self.i32c(tag);self.store_i32(0);
    }
    fn channel_store_error(&mut self,ptr:u32,code:u32) {
        self.get(ptr);self.get(code);self.store_i32(8);
    }
    fn channel_status_adt(&mut self,status:u32,result:u32) {
        self.channel_alloc(result);
        self.get(result);self.get(status);self.i32c(0);self.op(0x47); // err tag iff status nonzero
        self.store_i32(0);
        self.channel_store_error(result,status);
        self.get(result);
    }
    pub(super) fn channel_open(&mut self,capacity:i32,scratch:u32,status:u32,result:u32) {
        self.channel_alloc(scratch);
        self.i32c(capacity);self.get(scratch);self.call(self.l.import(Import::ChannelOpen));
        self.set(status);
        self.channel_alloc(result);
        self.get(result);self.get(status);self.i32c(0);self.op(0x47); // Err=1, Ok=0
        self.store_i32(0);
        self.get(result);
        self.get(status);self.op(0x45); // success
        self.open(0x04,I32,Frame::Other);
        self.get(scratch);self.load_i32(0); // valid host handle only on success
        self.op(0x05);self.get(status); // checked IoError status
        self.close();
        self.store_i32(8);
        self.get(result);
    }
    pub(super) fn channel_read(&mut self,channel:&TE,scratch:u32,status:u32) {
        self.channel_alloc(scratch);
        self.expr(channel);self.get(scratch);
        self.call(self.l.import(Import::ChannelNbRead));self.set(status);
        self.get(status);self.op(0x45); // no admission error?
        self.open(0x04,I32,Frame::Other);
        self.get(scratch);self.load_i32(0);
        self.op(0x05);
        // Negative encoded IoError is an immediately settled Future; never
        // collides with monotonically increasing positive host ticket IDs.
        self.i32c(0);self.get(status);self.op(0x6b); // i32.sub
        self.close();
    }
    pub(super) fn channel_send(&mut self,channel:&TE,value:&TE,status:u32,result:u32) {
        self.expr(channel);self.expr(value);
        self.call(self.l.import(Import::ChannelSend));self.set(status);
        self.channel_status_adt(status,result);
    }
    pub(super) fn channel_finish(&mut self,channel:&TE,operation:Import,status:u32,result:u32) {
        self.expr(channel);self.call(self.l.import(operation));self.set(status);
        self.channel_status_adt(status,result);
    }
    pub(super) fn channel_await(&mut self,ticket:&TE,scratch:u32,status:u32,result:u32,option:u32) {
        // Scratch is a checked 16-byte host output in guest-owned memory.
        self.channel_alloc(scratch);
        self.expr(ticket);self.set(status);
        self.get(status);self.i32c(0);self.op(0x48); // ticket < 0: immediately failed admission
        self.open(0x04,0x40,Frame::Other);
        self.channel_store_tag(scratch,2);
        self.get(scratch);self.i32c(0);self.get(status);self.op(0x6b); // -ticket
        self.store_i32(4);
        self.op(0x05);
        self.get(status);self.get(scratch);
        self.call(self.l.import(Import::ChannelAwaitRead));self.set(status);
        self.get(status);self.i32c(0);self.op(0x47); // status!=0
        self.open(0x04,0x40,Frame::Other);
        self.channel_store_tag(scratch,2);
        self.get(scratch);self.get(status);self.store_i32(4);
        self.close();
        self.close();
        self.channel_alloc(result);
        self.get(scratch);self.load_i32(0);self.i32c(2);self.op(0x46); // Err tag?
        self.open(0x04,0x40,Frame::Other);
        self.channel_store_tag(result,1);
        self.get(result);self.get(scratch);self.load_i32(4);self.store_i32(8);
        self.op(0x05);
        self.channel_store_tag(result,0);
        self.channel_alloc(option);
        self.get(option);self.get(scratch);self.load_i32(0);self.op(0x45); // Some=1, None=0
        self.store_i32(0);
        self.get(option);self.get(scratch);self.load_i64(8);self.store_i64(8);
        self.get(result);self.get(option);self.store_i32(8);
        self.close();
        self.get(result);
    }
}
