# Typed actor entrypoints — first browser execution slice

Every concrete executable actor must declare exactly **one** private, sealed
`receive` entrypoint. That rule applies to ordinary and `serial` actors:

```atto
define serial actor Counter as
    receive(msg: int): void {
        // An ordinary actor-local method can be called here.
        return;
    }
end
```

The direct message syntax `receive(msg: T)` is the preferred spelling; the
existing `receive(ActorMail<T> mail)` remains supported when a handler needs
`mail.value` or `mail.sequence`. Both are always a single parameter and
`void` result. Public or duplicate `receive` declarations are invalid.

## Actor kinds and current coverage

| Spelling | Frontend | Executable browser semantics |
| --- | --- | --- |
| `define actor` | Yes | Existing single-worker FIFO mailbox |
| `define serial actor` | Yes | Explicit serialized mailbox, one handler active |
| `define isoactor` | Recognized | `E_UNSUPPORTED`: isolation backend not proved |
| `define shared actor` | Recognized | `E_UNSUPPORTED`: shared state contract not proved |
| `define untrusted actor` | Recognized | `E_UNSUPPORTED`: sandbox policy not proved |

`serial actor` is **not** a new actor base class. It is a scheduling
modifier retained in the AST/typed IR and emitted in the browser
`atto.actor_modes` custom section: LEB128 actor count followed by a byte per
actor (0=default, 1=serial), indexed in `atto.actors` declaration order.
Consumers that do not implement other scheduling strategies may still execute
default actors serially; they must not silently enable parallel state mutation.

The first executable direct payload type is `int`; `ActorMail<bool>` remains
executable. `receive(msg: bool)`, tagged enum/ADT/object payloads,
`match`-based dispatch, ownership transfer, bounded async send/admission
receipts, suspend/await retention, and parallel default actor dispatch require
subsequent compiler and host ABI work, and are **not claimed as implemented**.
An unrecognized or unsupported kind or payload must fail without a Wasm artifact.

Do not treat browser single-worker execution as proof of native concurrency,
hard-real-time scheduling, isolation, or zero-copy actor messaging.
