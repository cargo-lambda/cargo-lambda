# Multi Process Watcher

## Problem

Currently, each new function that is invoked is spawned with its own `Watchexec` watcher. This is a viable approach, as it allows new functions to be invoked easily and within their own subsystems. The difficulty with this approach is a lack of control over when which function is rebuilt (i.e. when `cargo run --binary {binary_name}`). When multiple functions are invoked at the same time multiple processes may try to acquire locks on the build directory, potentially causing deadlock and halting of the recompilation process.

## Proposed Changes

To avoid this deadlock problem and to enable more fine grained control over which function is spawned when, this PR proposes to run all function processes/commands under the same `Watchexec` watcher. This way function builds/rebuilds may be performed in sequence, or in another other order of choosing.

## Reasoning

By gaining a higher degree of control over the execution and scheduling of Lambda recompilation we can prevent deadlocking the build processes. Making the necessary changes to the `cargo-lambda-watcher` crate has become easier with the recent release of [watchexec](https://crates.io/crates/watchexec) version `3.0.0`. The recent release introduces a new API for `action_handler`s, giving users the ability to spawn and control multiple processes under one `Watchexec`.

## Design

#### API Changes

There would be no changes to the external API of `cargo-lambda-watcher` caused by this change.

#### Risks

Running multiple processes/functions under a single `Watchexec` requires a higher degree of fine tuning and an additional level of care when implementing the `action_handler`.

## Drawbacks

## Open Questions

- Does finer grain control allow deadlocks to be prevented completely?
- Could the current implementation of `cargo-lambda-watcher` be adjusted in a simpler way to achieve the same results?
- Is this change important enough to justify an overhaul of `cargo-lambda-watcher`'s Lambda scheduler?

## Progress

- [x] Spawn multiple processes from one watcher
  - [x] Single watcher spawn all functions
  - [ ] Garbage collection for dead processes
- [ ] Filter events for specific functions
  - [ ] Filter for file paths
- [ ] Thorough logging.
- [ ] Graceful handling of the `Watchexec` exiting.
