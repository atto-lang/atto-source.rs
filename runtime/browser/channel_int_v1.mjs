// Worker-owned, typed Channel<int> host runtime contract v1.
//
// This is a real bounded FIFO scheduler *on the host*, not an AttoLang guest
// import. The compiler MUST continue to reject guest Channel<T>/nb readch
// until the versioned Result<Option<int>,IoError> Wasm representation exists.
// No child actor, network sender, or second Worker shares these handles.

const MIN_I64 = -(1n << 63n);
const MAX_I64 = (1n << 63n) - 1n;
const ok = value => ({ok: true, value});
const err = kind => ({ok: false, error: {kind}});

export class ChannelIntHostV1 {
  #channels = new Map();
  #tickets = new Map();
  #retired = new Set();
  #nextChannel = 1;
  #nextTicket = 1;
  #maxChannels;
  #maxTickets;
  #maxCapacity;

  constructor({maxChannels=64, maxTickets=1024, maxCapacity=1024}={}) {
    for (const n of [maxChannels, maxTickets, maxCapacity]) {
      if (!Number.isSafeInteger(n) || n<0 || n>65536) {
        throw new TypeError('invalid Channel<int> host quota');
      }
    }
    this.#maxChannels=maxChannels;
    this.#maxTickets=maxTickets;
    this.#maxCapacity=maxCapacity;
  }

  open(capacity) {
    if (!Number.isSafeInteger(capacity) || capacity<0 || capacity>this.#maxCapacity)
      return err('InvalidCapacity');
    if (this.#channels.size>=this.#maxChannels || this.#nextChannel>0x7fffffff)
      return err('Quota');
    const handle=this.#nextChannel++;
    this.#channels.set(handle,{
      capacity, items:[], pending:[], closed:false, handle
    });
    return ok(handle);
  }

  send(handle, value) {
    const channel=this.#channels.get(handle);
    if (!channel) return err('InvalidHandle');
    if (typeof value!=='bigint' || value<MIN_I64 || value>MAX_I64)
      return err('InvalidValue');
    if (channel.closed) return err('Closed');
    // A pending registration always gets the next admitted value first.
    if (channel.pending.length) {
      const ticket=this.#tickets.get(channel.pending.shift());
      if (!ticket || ticket.settled) throw Error('broken channel admission invariant');
      this.#settle(ticket,ok({some:true,value}));
      return ok({admitted:true});
    }
    if (channel.items.length>=channel.capacity) return err('Full');
    channel.items.push(value);
    return ok({admitted:true});
  }

  nb_read(handle) {
    const channel=this.#channels.get(handle);
    if (!channel) return err('InvalidHandle');
    if (this.#tickets.size>=this.#maxTickets || this.#nextTicket>0x7fffffff)
      return err('Quota');
    const ticketId=this.#nextTicket++;
    const ticket={id:ticketId,channel:handle,settled:false,claimed:false,
      outcome:null,notify:null};
    this.#tickets.set(ticketId,ticket);
    // Drain queued values even after close; only then report EOF.
    if (channel.items.length) {
      this.#settle(ticket,ok({some:true,value:channel.items.shift()}));
    } else if (channel.closed) {
      this.#settle(ticket,ok({some:false}));
    } else {
      channel.pending.push(ticketId);
    }
    return ok(ticketId);
  }

  // Claim a Future synchronously, then await its completion. Two concurrent
  // awaiters cannot both claim one ticket. No busy spin or speculative read.
  await_read(ticketId) {
    const ticket=this.#tickets.get(ticketId);
    if (!ticket) return Promise.resolve(err(this.#retired.has(ticketId)
      ? 'AlreadyConsumed' : 'InvalidHandle'));
    if (ticket.claimed) return Promise.resolve(err('AlreadyConsumed'));
    ticket.claimed=true;
    if (ticket.settled) {
      this.#retire(ticket);
      return Promise.resolve(ticket.outcome);
    }
    return new Promise(resolve => {ticket.notify=resolve});
  }

  cancel(ticketId) {
    const ticket=this.#tickets.get(ticketId);
    if (!ticket) return err(this.#retired.has(ticketId)
      ? 'AlreadyConsumed' : 'InvalidHandle');
    if (ticket.settled) return err('AlreadySettled');
    const channel=this.#channels.get(ticket.channel);
    const idx=channel.pending.indexOf(ticketId);
    if (idx<0) throw Error('broken pending read invariant');
    channel.pending.splice(idx,1);
    this.#settle(ticket,err('Cancelled'));
    return ok({cancelled:true});
  }

  close(handle) {
    const channel=this.#channels.get(handle);
    if (!channel) return err('InvalidHandle');
    if (channel.closed) return ok({alreadyClosed:true});
    channel.closed=true;
    // No queue can coexist with pending read tickets. If this ever
    // changes, pending results must be delivered FIFO before EOF.
    if (channel.pending.length && channel.items.length)
      throw Error('broken close ordering invariant');
    while (channel.pending.length) {
      const ticket=this.#tickets.get(channel.pending.shift());
      if (!ticket || ticket.settled) throw Error('broken close invariant');
      this.#settle(ticket,ok({some:false}));
    }
    return ok({alreadyClosed:false});
  }

  // Final owner-controlled drop. Closing is NOT equivalent to destroying
  // the channel: buffered items must remain retrievable until explicitly
  // drained. Outstanding ticket owners also prohibit quota reclamation.
  release(handle) {
    const channel=this.#channels.get(handle);
    if (!channel) return err('InvalidHandle');
    if (!channel.closed) return err('NotClosed');
    if (channel.items.length || channel.pending.length ||
        [...this.#tickets.values()].some(ticket=>ticket.channel===handle)) {
      return err('Busy');
    }
    this.#channels.delete(handle);
    return ok({released:true});
  }

  #settle(ticket,outcome) {
    if (ticket.settled) throw Error('Future settled twice');
    ticket.settled=true;
    ticket.outcome=outcome;
    if (ticket.notify) {
      const done=ticket.notify;
      ticket.notify=null;
      this.#retire(ticket);
      done(outcome);
    }
  }

  #retire(ticket) {
    if (!this.#tickets.delete(ticket.id)) throw Error('Future retired twice');
    this.#retired.add(ticket.id);
    // Bounded tombstones, solely to return a helpful AlreadyConsumed
    // diagnostic; tickets and channel handles themselves never recycle.
    if (this.#retired.size>4096) {
      this.#retired.delete(this.#retired.values().next().value);
    }
  }

  // Debug metrics are host-only, never authoritative guest capability data.
  snapshot(handle) {
    const c=this.#channels.get(handle);
    if (!c) return err('InvalidHandle');
    return ok(Object.freeze({
      queued:c.items.length,pending:c.pending.length,
      closed:c.closed,capacity:c.capacity,outstanding:this.#tickets.size
    }));
  }
}
