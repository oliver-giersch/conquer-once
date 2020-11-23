# Releases

## Release `0.2.0`

- fixes internal soundness issues (e.g. one similar to
  https://github.com/rust-lang/rust/pull/65719)
- renames `OnceCell::new` to `OnceCell::uninit` and `OnceCell::initialized` to
  `OnceCell::new`
- adds some convenience trait implementations and tightens some trait bounds
  (breaking)
- improved internal code structure and code re-usage, better facilitates
  monomorphic code usage

### Release `0.2.1`

- updates docs to explicitly specify the concrete synchronization guarantees

## Release `0.3.0`

- mostly internal refactorings
- adds the `noblock` module containing types which only allow non-blocking
  initialization
- `raw` is renamed to `doc` and hidden from documentation, but linked to from
  within the docs
- the `Spin` marker type is no longer exported

### Release `0.3.1`

- fixes an apparent regression, requiring a new compiler than the stated 1.36.0