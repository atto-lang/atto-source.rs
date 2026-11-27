# Historical native-host tests

These unchanged tests require the separate legacy `oreslang_source_rs` frontend and `oresc` native AOT executable. They are not runnable targets of the browser-only Cargo manifest. Their browser counterparts exercise `atto_compiler::compile` and `attoc` with the browser source byte limit and actual Wasm output. No native AOT qualification is inferred from browser tests. Restore these native tests in their owning native-host package when integrating that package.
