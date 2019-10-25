# Releases

## Release `0.3.0`

- fixes internal soundness issues (similar to https://github.com/rust-lang/rust/pull/65719)
- renames `OnceCell::new` to `OnceCell::uninit` and `OnceCell::initialized` to `OnceCell::new`
- adds some convenience trait implementations and tightens some trait bounds (breaking)

