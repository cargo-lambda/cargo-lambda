# Multi Process Watcher

## Problem

Currently, each new function that is invoked is spawned with its own `Watchexec` watcher. This is a viable approach, as it allows new functions to be invoked easily and within their own subsystems. The difficulty with this approach is a lack of control over when which function is rebuilt (i.e. when `cargo run --binary {binary_name}`). When multiple functions are invoked at the same time multiple processes may try to acquire locks on the build directory, potentially causing deadlock and halting of the recompilation process.

## Proposed Changes

To avoid this deadlock problem and to enable more fine grained control over which function is spawned when, this PR proposes to run all function processes/commands under the same `Watchexec` watcher. This way function build/rebuilds may be performed in sequence and in such a way that deadlock is avoided as best as possible.

## Reasoning

## Design

#### API Changes

#### Risks

## Drawbacks

## Open Questions

## Progress

- [ ] Spawn multiple processes from one watcher
  - [ ] Single watcher spawn all functions
  - [ ] Garbage collection for dead processes
- [ ] Filter events for specific functions
  - [ ] Filter for file paths
