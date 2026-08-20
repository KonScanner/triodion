# Contributing to triodion

triodion is a small fork with a single maintainer. Contributions are welcome, and
the process is deliberately short.

Your contributions are licensed under both MIT and Apache 2.0, the same as the
project.

## Before you start

-   Small changes (a bug fix, a doc fix, a test): open a pull request directly.
-   Larger changes (a new dataset, a new interface, a dependency change): open an
    issue first, so we can agree on the approach before you write the code.
-   A draft pull request is a good way to show work in progress.

Replies can take a few days. One person reviews everything.

## Reporting a bug

Open an issue and include:

-   The `triodion` version, and confirmation that it is current
-   Your platform (Linux, macOS, Windows)
-   The exact command or python code you ran
-   The chain and the RPC provider
-   The smallest reproduction you can make

See [this guide][mcve] on how to write a minimal, complete, and verifiable
example.

## Requesting a feature

Open an issue. Describe what you want and why. If another tool already does it,
give a link.

## Checks

Run these four commands before you open a pull request. CI runs the same ones.

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo +nightly fmt --all
```

`+nightly` is required for `fmt`, not decorative. `rustfmt.toml` sets
nightly-only options, and `rust-toolchain.toml` pins the stable channel. A bare
`cargo fmt` ignores those options and then disagrees with CI.

## Tests

Every fix and every feature needs a test.

-   Unit tests for one function with one clear task.
-   Integration tests for behavior that crosses modules. Copy the style of the
    existing tests.
-   A test that forks a chain must have "fork" in its name.

## Commits

Group each logical change into its own commit. Squash the checkpoint commits that
do not stand alone. There is no limit on the number of commits in a pull request.

## Pull requests

The pull request template asks for three things: Overview, Reasoning, and Tests.
Fill in all three. Expect review comments, and expect requests for changes. Only
an incremental improvement is needed to merge. A follow-up pull request can
continue the work.

## Code of Conduct

This project follows the [Rust Code of Conduct][rust-coc].

[rust-coc]: https://github.com/rust-lang/rust/blob/master/CODE_OF_CONDUCT.md
[mcve]: https://stackoverflow.com/help/mcve
