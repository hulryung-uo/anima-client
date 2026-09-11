# Contributing to Anima

Useful contributions include reproducible gameplay bugs, installation results,
shard compatibility reports, documentation, and protocol or renderer fixes.
You do not need to write Rust to help.

## Report a problem

[Open a bug report](https://github.com/hulryung-uo/anima-client/issues/new?template=bug_report.yml)
with the Anima version, OS/architecture, server software, steps to reproduce,
expected behavior, and actual behavior. A short recording or screenshot helps.
Remove account passwords, tokens, private server addresses, and personal data.
Distinguish a verified result from something you expect to work.

For installation help, read [Getting started](docs/GETTING_STARTED.md).

## Change the code

1. Read [DESIGN.md](docs/DESIGN.md), the source of truth for architecture.
2. Check [CLASSICUO_GAPS.md](docs/CLASSICUO_GAPS.md) for existing verification.
3. Keep protocol/world logic in the core and rendering/UI outside it. Preserve
   the shared Observation/Action boundary.
4. Use the Rust version pinned in `rust-toolchain.toml` and run
   `scripts/check.sh` before submitting. See [testing notes](docs/TESTING.md).
5. Describe the concrete before/after behavior and how you verified it. Mention
   missing live-shard or real-data validation explicitly.

For renderer tests, read [web/test/README.md](web/test/README.md) first. Browser
scripts share a global scope; testing each file's syntax separately is not enough.

Do not commit UO game data or credentials. The repository's code is dual-licensed
under MIT / Apache-2.0; game assets are outside that license.
