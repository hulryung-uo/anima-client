# Sound loading and memory budgets

Current source, newer than the v0.7.0 installers. These changes improve the
sound reader and renderer's sound pipeline; they do not establish a whole-client
memory limit or complete the long-session gameplay audit.

## Behavior

The native sound reader opens the UOP directory and reads individual entries
through `LazyUopReader`. It no longer keeps the entire sound bank in a `Vec`.
`Sound.def` aliases, including storage indices above the packet's 16-bit ID
range, and explicit silence retain their previous behavior. WAV output remains
PCM 16-bit mono at 22050 Hz.

| Resource | Limit / policy |
| --- | --- |
| Native retained WAV payloads | 32 MiB, at most 512 entries, least recently used first |
| Native missing/silenced lookups | Share the 512-entry bound |
| One sound asset | At most 32 MiB as a WAV; raw entry declarations and actual zlib expansion are checked before serving |
| Browser retained decoded buffers | 128 MiB of Float32 samples and 512 entries, least recently used first |
| Browser simultaneous fetch/decode work | 4, including decoders that exceeded the caller deadline but have not actually finished |
| Browser loading deadline | 5 seconds, covering response, body and decoding |
| Browser failed lookup backoff | 60 seconds for 404, 1 second for other failures; at most 128 remembered failures |
| Simultaneously playing effects | 8, unchanged |

Browser response streams are counted as they arrive, so an absent or false
Content-Length cannot bypass the body bound. Engines without response streams
use an arrayBuffer fallback with a size check; the native endpoint independently
limits the WAV it serves.

Cached effects remain immediately available during a loading burst. Additional
uncached effects are skipped when all four work slots are occupied; they are
not queued for delayed playback. An effect whose initial load takes over one
second warms the cache but does not play its outdated event. A new request can
use that buffer immediately. Effects already outside hearing range or beyond
the active playback cap do not start more downloads.

Eviction removes the cache's reference; it does not stop a playing source.
A decoded buffer larger than the retention budget can still be used without
being cached. Active playback buffers, transient reads/decodes, allocator
overhead and other client assets are separate from the retained-byte budgets.
Mute, disabled effects, transport loss and session changes continue to cancel
pending playback without replaying it when sound or connectivity returns.

## Verification — 2026-09-13

The local stock `soundLegacyMUL.uop` was 168,385,870 bytes with 1,655 directory
entries. The largest entry was `beatFast.wav`; the resulting WAV was 21,187,628
bytes. Its size was checked before selecting the per-asset bound.

A standalone native probe linked the current asset library and the previous
sound reader from commit `3911641895405e28d433084de082e4269960888f`. In separate
processes, `/usr/bin/time -l` measured maximum resident memory:

| Sound-reader workload | Previous | Current |
| --- | ---: | ---: |
| Open bank only | 170,557,440 bytes | 2,146,304 bytes |
| Read IDs 0–4095 once | 389,726,208 bytes | 121,225,216 bytes |

A separate comparison checked complete `Option<Vec<u8>>` outputs for all 4,096
IDs: all matched, with 1,656 valid sounds including aliases. These are sound-reader
measurements on one Mac and one resource package, not application-wide memory
or audible gameplay measurements. Raw logs, source hashes and probe location
are recorded locally in `/tmp/anima-sound-memory-benchmark.json`.

The headless renderer probe used 600 fixture buffers representing 1 MiB each.
Retained sample bytes fell from 600 MiB to 128 MiB. A burst of 100 unique IDs
started 4 requests instead of 100. These measure cache/admission behavior with
fixture Web Audio buffers, not a browser heap profile. Reports are in
`/tmp/anima-audio-cache-before.json` and `/tmp/anima-audio-cache-after.json`.

`scripts/check.sh` completed with actual exit 0: 314 web tests / 1575 assertions,
23 Python tooling tests, Rust/WASM checks and 14 desktop tests (the ordinary gate
excludes two real-vault tests). Log: `/tmp/anima-audio-budgets-gate.log`.
Regressions cover LRU/byte/entry eviction, active-buffer lifetime, load coalescing,
physical decoder limits after timeout, stale retries, streamed body limits,
backoff, long stock-sized sounds, late-playback expiry and inaudible effects.
Native tests also cover lazy payload reads, aliases, silence and zlib expansion
beyond a false declared size.
Desktop CI now runs the native asset and connection library tests on both
macOS and Windows, in addition to desktop compilation and profile/vault checks.

The real-resource test was additionally run and passed:

```sh
cargo test -p anima-assets sound::tests::opens_real_sounds_and_builds_wav -- --ignored --nocapture
```

It requires the local game files and fails explicitly if the sound bank is
absent. Interactive listening, Windows runtime measurements and broader
texture/map/animation-cache audits remain open in CLIENT_READINESS.md.
