Thanks for your interest in contributing to this project.

These are some basic instructions to build and test the project.

## Organization

This project uses Cargo workspaces to organize its codebase. If you run `cargo build` or `cargo nextest run` in the root of the repository, you'll be running it against all the crates that create this project.

We use Cargo Clippy and Cargo Fmt to keep the project formatted and follow best practices. You can run `make fmt` and `make clippy` to invoke those tools with the project's configuration. 

## Building

If you want to compile a release version for testing locally, you can run `make install`. This will compile your current project, and will put the `cargo-lambda` binary in `$CARGO_HOME/bin`.

## Testing

This project uses unit tests in individual files and integration tests under `crates/cargo-lambda-cli/tests`. All tests are run using [cargo-nextest](https://nexte.st/), which provides faster parallel execution and better test reporting.

### Running Tests

First, install cargo-nextest if you haven't already:

```bash
make install-nextest
# or
cargo install cargo-nextest --locked
```

Then run the test suite:

```bash
# Run all tests
make test
# or
cargo nextest run --all-features
```

### Test Performance Tips

For faster test iteration during development:

- Use `cargo nextest run --package cargo-lambda-cli` to run only the CLI tests
- Use `cargo nextest run <test_name>` to run a specific test
- The test suite uses a shared build cache to avoid recompiling dependencies across tests
- First test run will be slower as it warms the cache; subsequent runs are much faster

The project includes a `.config/nextest.toml` configuration file that optimizes test execution with parallel execution, retries, and timeout settings.