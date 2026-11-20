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
  #maxPendingSends;
  #pendingSendCount = 0;

  constructor({maxChannels=64, maxTickets=1024, maxCapacity=1024, maxPendingSends=1024}={}) {
    for (const n of [maxChannels, maxTickets, maxCapacity, maxPendingSends]) {
      if (!Number.isSafeInteger(n) || n<0 || n>65536) {
        throw new TypeError('invalid Channel<int> host quota');
      }
    }
    this.#maxChannels=maxChannels;
    this.#maxTickets=maxTickets;
    this.#maxCapacity=maxCapacity;
    this.#maxPendingSends=maxPendingSends;
  }

  open(capacity) {
    if (!Number.isSafeInteger(capacity) || capacity<0 || capacity>this.#maxCapacity)
      return err('InvalidCapacity');
    if (this.#channels.size>=this.#maxChannels || this.#nextChannel>0x7fffffff)
      return err('Quota');
    const handle=this.#nextChannel++;
    this.#channels.set(handle,{
      capacity, items:[], pending:[], waitingSends:[], closed:false, handle
    });
    return ok(handle);
  }

  send(handle, value) {
    const channel=this.#channels.get(handle);
    if (!channel) return err('InvalidHandle');
    if (typeof value!=='bigint' || value<MIN_I64 || value>MAX_I64)
      return err('InvalidValue');
    if (channel.closed) return err('Closed');
    // A previously blocked sender must get admission first. Nonparking sends
    // cannot jump over an older writer when capacity becomes available.
    if (channel.waitingSends.length) return err('Full');
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

  // One checked suspending send. No busy wait, carrier parking or retries:
  // admission linearizes at the same FIFO boundary as send(). The Promise
  // settles only when the message is admitted or an error/cancellation wins.
  // Abort is optional for owner-controlled cancellation and is never a guest
  // capability token. Quotas bound all outstanding writer registrations.
  send_async(handle, value, {signal}={}) {
    // Cancellation observed before the linearization point admits nothing.
    if (signal?.aborted) return Promise.resolve(err('Cancelled'));
    const immediate=this.send(handle,value);
    if (immediate.ok || immediate.error.kind!=='Full')
      return Promise.resolve(immediate);
    if (signal?.aborted) return Promise.resolve(err('Cancelled'));
    if (this.#pendingSendCount>=this.#maxPendingSends) return Promise.resolve(err('Quota'));
    const channel=this.#channels.get(handle);
    if (!channel || channel.closed)
      return Promise.resolve(err(channel?'Closed':'InvalidHandle'));
    return new Promise(resolve=>{
      const entry={value,resolve,signal,onAbort:null,settled:false};
      if (signal) {
        entry.onAbort=()=>{
          if (entry.settled) return;
          const index=channel.waitingSends.indexOf(entry);
          if (index<0) throw Error('broken pending send cancellation invariant');
          channel.waitingSends.splice(index,1);
          this.#settleSend(entry,err('Cancelled'));
        };
        signal.addEventListener('abort',entry.onAbort,{once:true});
      }
      channel.waitingSends.push(entry);
      this.#pendingSendCount++;
      // Abort may race the listener registration before enqueue completes.
      if (signal?.aborted) entry.onAbort();
    });
  }

  #settleSend(entry,outcome) {
    if (entry.settled) throw Error('pending channel send settled twice');
    entry.settled=true;
    this.#pendingSendCount--;
    if (entry.signal && entry.onAbort)
      entry.signal.removeEventListener('abort',entry.onAbort);
    entry.resolve(outcome);
  }

  // A consumed slot admits at most one queued writer. Zero-capacity channels
  // admit only when a reader has registered; no phantom buffering.
  #admitOldestWriter(channel) {
    if (!channel.waitingSends.length) return;
    const entry=channel.waitingSends[0];
    if (channel.pending.length) {
      const reader=this.#tickets.get(channel.pending.shift());
      if (!reader || reader.settled) throw Error('broken pending read/write invariant');
      channel.waitingSends.shift();
      this.#settle(reader,ok({some:true,value:entry.value}));
      this.#settleSend(entry,ok({admitted:true}));
    } else if (channel.items.length<channel.capacity) {
      channel.waitingSends.shift();
      channel.items.push(entry.value);
      this.#settleSend(entry,ok({admitted:true}));
    }
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
      // Replenish the newly vacated slot with the oldest waiting writer.
      this.#admitOldestWriter(channel);
    } else if (channel.waitingSends.length && !channel.closed) {
      // The capacity-zero rendezvous path cannot buffer the writer's item.
      channel.pending.push(ticketId);
      this.#admitOldestWriter(channel);
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
    // Closing revokes every writer waiting for admission. Their items were
    // never accepted and must not become visible to any future reader.
    while (channel.waitingSends.length) {
      const pending=channel.waitingSends.shift();
      this.#settleSend(pending,err('Closed'));
    }
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
    if (channel.items.length || channel.pending.length || channel.waitingSends.length ||
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
