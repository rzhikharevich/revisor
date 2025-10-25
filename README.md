# revisor

A runit-inspired daemon supervisor.

## Recipe for Small Builds

```sh
cargo +nightly build -Z build-std=std,panic_abort -Z build-std-features=optimize_for_size --profile=small
```
