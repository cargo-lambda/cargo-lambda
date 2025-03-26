Thanks for your interest in contributing to this project.

These are some basic instructions to build and test the project.

## Organization

This project uses Cargo workspaces to organize its codebase. If you run `cargo build` or `cargo test` in the root of the repository, you'll be running it against all the crates that create this project.

We use Cargo Clippy and Cargo Fmt to keep the project formatted and follow best practices. You can run `make fmt` and `make clippy` to invoke those tools with the project's configuration. 

## Building

If you want to compile a release version for testing locally, you can run `make install`. This will compile your current project, and will put the `cargo-lambda` binary in `$CARGO_HOME/bin`.

## Testing

This project uses Cargo Test for unit tests, you can find those in individual files. We also use Cargo's own integration test harness to write integration tests. You can find those under `crates/cargo-lambda-cli/tests`. Integration tests run along the regular test suite when you execute `cargo test`.