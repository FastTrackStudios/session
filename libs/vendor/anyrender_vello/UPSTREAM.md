Source: crates.io `anyrender_vello` 0.12.0 (dioxuslabs/anyrender), the
published tarball with the registry's own bookkeeping files dropped.
License: MIT OR Apache-2.0 — theirs, not ours, and it stays that way.

Vendored for three changes to `window_renderer.rs`, all found by
measuring an arrangement window at 2560x1440 and all reversible from
the environment so the next machine can re-measure rather than trust
these numbers:

- **The GPU barrier is gone.** After `present`, upstream calls
  `poll(wait_indefinitely())`, which blocks the thread until the GPU has
  finished everything it was handed. That is a hard fence once a frame:
  the CPU cannot start building frame N+1 until frame N has been
  rasterised, so a frame costs CPU + GPU instead of max(CPU, GPU).
  `PollType::Poll` does the same housekeeping — map callbacks, freeing
  what the queue released — without the fence. `FTS_GPU_BARRIER=1` puts
  the wait back for a driver that needs it.

- **`desired_maximum_frame_latency` is 1, not 2.** Two lets the CPU run a
  frame ahead, which is right for something that can always fill the
  pipeline; here it meant `get_current_texture` blocked until the queue
  drained, on the critical path. Acquire-and-present went from 6.7 ms at
  two to 2.2 ms at one, with the GPU render itself 1.3 ms either way.
  Three was worse again. `FTS_FRAME_LATENCY` overrides it.

- **`present_mode` is switchable** via `FTS_PRESENT`
  (`immediate` / `mailbox`, default `AutoVsync`), so vsync can be ruled
  in or out by measurement. Under `AutoVsync` a slow frame and a frame
  that is merely WAITING are the same number from outside.

Plus a one-shot surface-configuration line behind the crate's existing
opt-in `log_frame_times` feature, which is `println`-based upstream.

Drop the vendor if upstream takes the barrier and latency changes. See
the root `[patch.crates-io]` entry and issue #119.
