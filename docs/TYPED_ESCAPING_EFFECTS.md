# Typed escaping effect rows — integrated browser subset

Integration base: post-#296 main with the #275 `trap -> Result<_,Error>`
IR, first-class Error handle identity and cleanup/recovery rules. The
`audit_cleanup_call_effects` fixed-point analysis is the single source
of truth for escaping ordinary throw and raise in the executable browser
profile. It traverses arguments, calls, recursive call groups, traps,
handlers, unwind registrations and structural reachability.

An explicit type list is an upper bound; an omitted kind in an otherwise
explicit declaration is forbidden. `@Throws()`, `@Raises()`,
`throws()`, and `raises()` are **explicit empty rows**. The AST records
per-kind declaration presence separately from allowed nominal Error names.
`@Throws(Error)` and `throws(Error)` normalize to the same data.
Existing unannotated functions retain inferred compatibility semantics.

The nominal representation retains names in three type sets; the browser
compiler currently admits **only the built-in Error** and rejects any
subclass name without type resolution and a matching runtime representation.
Explicit `@Panics()` cannot yet certify total absence of fatal traps,
allocation failure, call-depth quota or actor supervisor failures; accepting
the syntax is not proof of panic-freedom.

`trap` discharges only ordinary throw; it preserves Result.Ok/Err payloads
and Error identity, leaving raise and fatal panic untouched. `?` propagates
typed Result/Option failure, not an exception, and routes through defers
and finally. Cleanup callbacks cannot leak throw/raise; ordinary catch
outranks callee recover.

**Not yet implemented/claimed:** general nominal Error subclasses; typed
higher-order effect parameters; static cancellation/async and supervisor
CONTROL effect discharge; full CFG precision and full panics bound proof;
other AttoLang backends. Keep these fail-closed until verified.
