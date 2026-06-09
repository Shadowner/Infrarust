## Summary

<!-- What does this PR change, and why? -->

## Linked Issue

<!-- Closes #123, or "None" -->

## Checklist

- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` passes
- [ ] `cargo test --workspace` passes (and `cargo test -p infrarust-loader-wasm --features wasm` if the WASM loader is touched)
- [ ] Docs updated (`docs/v2`, README) if the change is user-facing
