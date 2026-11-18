# Keyed actor registry from compiled AttoLang — executable Rust/Wasm slice

**Status:** additive browser compiler/host prototype. Not a distributed directory,
not a public library design, not backend parity. Stacked on compiler PR #297
(typed one-entrypoint actors) and browser host PR #64 (local registry).

## Source syntax

```atto
define serial actor Queue as
    receive(msg: int): void {
        match msg over
            on 40 -> { stdio.println(40); }
            on _ -> { stdio.println(msg); }
        end
        return;
    }
end

pub fnc main(): int {
    val q = ActorRegistry.get_or_create<Queue>("tenant/vhost", "payments");
    q.send(40);
    return 0;
}
```

This is an explicitly activating, compiler-recognized **standard-library
intrinsic**, not a new keyword or general generic-call implementation. It
returns the already-checked `ActorRef<Queue>` type, with the existing typed
`send` method. The two key components must be *string literals* for this
initial profile; other argument counts, generic type names, and sender types
fail without emitting a Wasm guest.

## Wasm import contract

`atto.actor_get_or_create(i32 actor_type, i64 namespace_descriptor,
i64 application_key_descriptor) -> i32 actor_slot`.

Each i64 string descriptor uses the existing guest string ABI: upper 32 bits
unsigned byte offset into the main instance's exported linear memory and lower
32 bits unsigned UTF-8 byte length. The trusted host MUST decode with fatal
UTF-8 validation, verify bounds without integer overflow, reject zero lengths
and aggregate UTF-8 keys above 4,096 bytes, verify actor type against the
`atto.actors` table and its configured admission policy, and reject calls
from inside an actor. The host MUST allocate via its generation-safe
`createLocalActorRegistry` with exact typed key identity, not a fresh spawn
per lookup. **Do not silently fall back** to `actor_spawn` on registry failure.

The returned `i32` is the *existing* browser ABI actor slot for this
narrow profile. The compiled code tracks a nominal `ActorRef<A>`, not an
arbitrary integer; runtime liveness and mailbox admission remain separately
validated. This is not yet an unforgeable network capability or
`VirtualActorRef<A>`: it is a transient local activation reference.

## Safety boundary

- `ActorRegistry.get_or_create` is explicit activation. A virtual reference
  lookup (`ref_for`) must remain non-activating.
- Broker queue declarations, routing permissions, tenant authority and
  durable leases are NOT implied by a string key.
- Calls from actor `receive` are rejected pending verified nested spawning
  and supervision.
- Synchronous main-owned guest calls only, bounded by the existing actor
  count (16) and per-actor mailbox limit (10,000). Send returns on admission.
- The host's `ActorRef` slot will be invalid after retirement; no automatic
  redirection or exactly-once retry is asserted.
- Typed ADT variant payloads such as `QueueMessage.Publish(value)` and their
  cross-actor wire codecs remain NOT_EXECUTED in this profile. Canonical
  `on` statement match is tested using an `int` message, not advertised as
  compiled tagged ADT messages.

## Required validation

Actual native-vs-Wasm compiler equivalence, positive keyed guest execution,
missing `on` rejection with no guest artifact, wrong send-type rejection,
namespace isolation, malformed host string bounds, and exact-tree CI receipt.
Releasing a compiler artifact or claiming multi-backend parity is separate.
