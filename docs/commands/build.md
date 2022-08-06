# cargo lambda build

Within a Rust project that includes a `Cargo.toml` file, run the `cargo lambda build` command to natively compile your Lambda functions in the project.
The resulting artifacts such as binaries or zips, will be placed in the `target/lambda` directory.
This is an example of the output produced by this command:

```
❯ tree target/lambda
target/lambda
├── delete-product
│   └── bootstrap
├── dynamodb-streams
│   └── bootstrap
├── get-product
│   └── bootstrap
├── get-products
│   └── bootstrap
└── put-product
    └── bootstrap

5 directories, 5 files
```

## Output Format

By default, cargo-lambda produces a binary artifact for each Lambda functions in the project.
However, you can configure cargo-lambda to produce a ready to upload zip artifact.

The `--output-format` parameter controls the output format, the two current options are `zip` and `binary` with `binary` being the default.

Example usage to create a zip.

```
cargo lambda build --output-format zip
```

## Architectures

By default, cargo-lambda compiles the code for Linux X86-64 architectures, you can compile for Linux ARM architectures by providing the right target:

```
cargo lambda build --target aarch64-unknown-linux-gnu
```

ℹ️ Starting in version 0.6.2, you can use the shortcut `--arm64` to compile your functions for Linux ARM architectures:

```
cargo lambda build --arm64
```

## Compilation Profiles

By default, cargo-lambda compiles the code in `debug` mode. If you want to change the profile to compile in `release` mode, you can provide the right flag.

```
cargo lambda build --release
```

When you compile your code in release mode, cargo-lambda will strip the binaries from all debug symbols to reduce the binary size.

## Extensions

cargo-lambda can also build Lambda Extensions written in Rust. If you want to build a extension, use the flag `--extension` to put the output under `target/lambda/extensions`, so you don't mix extensions and functions.

```
cargo lambda build --release --extension
```

If you want to create a zip file with the structure that AWS Lambda expects to find extensions in, add the `--output-format` flag to the previous command, and cargo-lambda will zip the extensions directory with your extension inside.

```
cargo lambda build --release --extension --output-format zip
```

## How does it work?

cargo-lambda uses [Zig](https://ziglang.org) and [cargo-zigbuild](https://crates.io/crates/cargo-zigbuild)
to compile the code for the right architecture. If Zig is not installed in your host machine, the first time that your run cargo-lambda, it will guide you through some installation options. If you run cargo-lambda in a non-interactive shell, the build process will fail until you install that dependency.
