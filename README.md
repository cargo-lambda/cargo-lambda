# cargo-lambda

[![crates.io][crate-image]][crate-link]
[![Documentation][doc-image]][doc-link]
[![Build Status][build-image]][build-link]

cargo-lambda is a [Cargo](https://doc.rust-lang.org/cargo/) subcommand to help you work with AWS Lambda.

This subcommand compiles AWS Lambda functions natively and prepares the compiled binaries to upload them
to AWS Lambda with other echosystem tools, like [SAM Cli](https://github.com/aws/aws-sam-cli) or the [AWS CDK](https://github.com/aws/aws-cdk).

## Usage

Install this subcommand on your host machine with Cargo itself:

```
cargo install cargo-lambda
```

Within a Rust project that includes a `Cargo.toml` file, run the `cargo lambda build` command to compile your
Lambda functions present in the project. The resulting binary, or binaries, will be placed in the `target/lambda` directory. This is an example of what the output if this command is:

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

By default, cargo-lambda compiles the code for Linux X86-64 architectures, you can compile for Linux ARM architectures by providing the right target:

```
cargo lambda build --target aarch64-unknown-linux-gnu
```

By default, cargo-lambda compiles the code in `debug` mode. If you want to change the profile to compile in `release` mode, you can provide the right flag.

```
cargo lambda build --release
```

When you compile your code in release mode, cargo-lambda will strip the binaries from all debug symbols to reduce the binary size.

## How does cargo-lambda work?

cargo-lambda uses [Zig](https://ziglang.org) and [cargo-zigbuild](https://crates.io/crates/cargo-zigbuild)
to compile the code for the right architecture. If those dependencies are not installed in your host machine, the first time that your run cargo-lambda, it will guide you through some installation options. If you run cargo-lambda in a non-interactive shell, the build process will fail until you install those dependencies.


[//]: # (badges)

[crate-image]: https://img.shields.io/crates/v/cargo-lambda.svg
[crate-link]: https://crates.io/crates/cargo-lambda
[doc-image]: https://docs.rs/cargo-lambda/badge.svg
[doc-link]: https://docs.rs/cargo-lambda
[build-image]: https://github.com/calavera/cargo-lambda/workflows/Build/badge.svg
[build-link]: https://github.com/calavera/cargo-lambda/actions?query=workflow%3ACI+branch%3Amain