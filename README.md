# Exploring lending async iterators

This repository contains some code related to the [talk given during RustLab
2024](https://docs.google.com/presentation/d/1Z166168WqGkkWhTtoJPupw9jWJIfO0n8Ftxo45N4lQk/).

**Important notice**: in order to being able to compile the code, you need:
- a nightly compiler
- `RUSTFLAGS=-Zpolonius` environment variable

The nightly compiler is selected automatically (it is specified in the
`rust-toolchain.toml`), but you still need to set the environment variable when
running the compiler from CLI and for the `rust-analyzer` as well.

---

All the code presented is a proof of concept/proof of work for lending async
iterators. Some parts are missing because I did not have enough time to write a
totally exhaustive example, some other parts are impossible to write for
lending async iterators (i.e.: `collect`, `last`, `min`, `max`), and other
parts simply are very hard to make compile, at least without a lot of unsafe.
Hopefully in the future it should be possible to make more cases compile.
