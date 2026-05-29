+++
title = "Scheduling & resilience"
description = "architect::schedule retry/repeat policies and the architect::platform clock they run on."
weight = 60
+++

`architect::schedule` is a small, composable retry/repeat engine — the
Effect `Schedule` idea ported to plain `async`, no `Effect<A,E,R>` monad.
It stands on `architect::platform`, a `Clock` + `spawn` abstraction that
papers over the native (tokio) ↔ wasm (browser timers) split.

Both are opt-in. Enable the `schedule` feature (it pulls `platform`):

```toml
architect = { version = "…", features = ["schedule"] }
```

## The model

A [`Schedule`] is a **value** describing a sequence of delays. It does no
I/O — its one operation, `next(attempt) -> Option<Decision>`, is pure and
attempt-based, so the policy is trivially unit-testable:

```rust
use architect::Schedule;
use std::time::Duration;

// 200ms, 400ms, 800ms … (±20% jitter), capped at 5s, at most 5 retries.
let mut policy = Schedule::exponential(Duration::from_millis(200))
    .max_delay(Duration::from_secs(5))
    .jittered()
    .take(5);

let first = policy.next(1).unwrap().delay; // ~200ms
assert!(first >= Duration::from_millis(160) && first <= Duration::from_millis(240));
```

`Some(decision)` → wait `decision.delay`, then attempt again. `None` →
stop. That's the whole contract.

### Constructors

| Constructor | Behaviour |
| --- | --- |
| `Schedule::never()` | one attempt, no retry |
| `Schedule::recurs(n)` | up to `n` immediate (zero-delay) recurrences |
| `Schedule::spaced(d)` | fixed delay `d`, forever |
| `Schedule::exponential(base)` | `base`, `base·2`, `base·4`, … |
| `Schedule::exponential_factor(base, f)` | `base·fⁿ` |
| `Schedule::linear(base)` | `base`, `base·2`, `base·3`, … |

### Combinators (builder style)

| Combinator | Effect |
| --- | --- |
| `.max_delay(cap)` | clamp every delay to `cap` |
| `.take(n)` | stop after `n` recurrences |
| `.jittered()` / `.jittered_by(frac)` | scale by a deterministic factor in `[1±frac]` (default ±20%) |
| `.map_delay(f)` | transform each delay (the escape hatch for true RNG, floors, …) |
| `.and_then(next)` | run `self` to exhaustion, then `next` (attempts rebased) |
| `.intersect(other)` | recur only while **both** recur; delay = the larger |
| `.union(other)` | recur while **either** recurs; delay = the smaller |

Jitter is **deterministic** (a splitmix64 of the attempt number), so it's
reproducible and needs no `getrandom` — which keeps `Schedule` wasm-clean.
Reach for `.map_delay` when you genuinely want entropy.

## The drivers

`retry` / `repeat` run a plain `async FnMut() -> Result<T, E>` under a
schedule:

- **`retry`** — re-run while it returns `Err`. Returns the first `Ok`, or
  the last `Err` once the schedule is exhausted.
- **`repeat`** — re-run while it returns `Ok`. Returns the last `Ok` once
  exhausted, or the first `Err` that interrupts it.

```rust,ignore
use architect::{schedule, Schedule};
use std::time::Duration;

let client = schedule::retry(
    || async { ExampleRepoClient::establish(&link).await },
    Schedule::exponential(Duration::from_millis(200))
        .max_delay(Duration::from_secs(5))
        .jittered()
        .take(5),
)
.await?;
```

Both default to the real [`SystemClock`]. The `_with` variants
(`retry_with` / `repeat_with`) take a `&Clock`, which is how tests inject a
deterministic one.

## The payoff: resilient clients

This is the direct answer to the "client connect is fragile" thread. The
example app wraps its vox connect in `schedule::retry`
([`examples/app/ui/src/client.rs`](https://git.starcommand.live/codywright/architect/src/branch/main/examples/app/ui/src/client.rs)),
so a just-booting or briefly-flaky server is tolerated instead of failing
the first frame:

```rust,ignore
async fn establish_ws<C: vox_core::FromVoxSession>(url: &str) -> Result<C, String> {
    architect::schedule::retry(
        || establish_ws_once::<C>(url),
        architect::Schedule::exponential(Duration::from_millis(200))
            .max_delay(Duration::from_secs(5))
            .jittered()
            .take(5),
    )
    .await
}
```

Because the policy's clock is platform-portable, the same code is resilient
on **both** targets — `tokio::time` on desktop, browser timers on web.

## The platform layer

`architect::platform` is the foundation `schedule` waits on, and a useful
primitive in its own right:

| Item | Role |
| --- | --- |
| `Clock` (`now` + async `sleep`) | the trait the drivers are generic over |
| `SystemClock` | the real clock — `tokio::time` / browser `setTimeout` |
| `TestClock` | deterministic; time only moves on `advance()` |
| `sleep` / `now` / `spawn` (free fns) | portable primitives — native (tokio) / wasm (`spawn_local`) |
| `Instant` | re-export of `web_time::Instant` (monotonic on both targets) |

It centralizes the native↔wasm cfg-split that the rest of the crate
(`resource`, `local`, `axum_ws`) would otherwise each repeat.

## Testing scheduled code

Inject a `TestClock` and drive time by hand — sleeps resolve the instant
you advance past their deadline, so retry/backoff loops run **instantly and
deterministically**, with zero wall-clock flakiness:

```rust,ignore
use architect::platform::{Clock, TestClock};

let clock = TestClock::new();
let fut = schedule::retry_with(&clock, op, Schedule::exponential(/* … */));
// poll `fut`, then `clock.advance(Duration::from_millis(200))` to release
// the first backoff sleep, and so on — no real time elapses.
```

For policy logic itself you don't even need a clock: `Schedule::next` is
pure, so assert the delay sequence directly (this is how `schedule`'s own
unit tests cover `exponential`, `max_delay`, `intersect`, `and_then`, …).
