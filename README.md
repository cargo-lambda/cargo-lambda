# cargo-lambda

[![crates.io][crate-image]][crate-link]
[![Build Status][build-image]][build-link]

cargo-lambda is a [Cargo](https://doc.rust-lang.org/cargo/) subcommand to help you work with AWS Lambda.

The [build](#build) subcommand compiles AWS Lambda functions natively and produces artifacts which you can then upload to AWS Lambda or use with other echosystem tools, like [SAM Cli](https://github.com/aws/aws-sam-cli) or the [AWS CDK](https://github.com/aws/aws-cdk).

The [start](#start) subcommand boots a development server that emulates interations with the AWS Lambda control plane. This subcommand also reloads your Rust code as you work on it.

The [invoke](#invoke) subcommand sends request to the control plane emulator to test and debug interactions with your Lambda functions.

## Installation

Install this subcommand on your host machine with Cargo itself:

```
cargo install cargo-lambda
```

## Usage

### Build

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

#### Build - Output Format

By default, cargo-lambda produces a binary artifact for each Lambda functions in the project.
However, you can configure cargo-lambda to produce a ready to upload zip artifact.

The `--output-format` paramters controls the output format, the two current options are `zip` and `binary` with `binary` being the default.

Example usage to create a zip.

```
cargo lambda build --output-format zip
```

#### Build - Architectures

By default, cargo-lambda compiles the code for Linux X86-64 architectures, you can compile for Linux ARM architectures by providing the right target:

```
cargo lambda build --target aarch64-unknown-linux-gnu
```

#### Build - Compilation Profiles

By default, cargo-lambda compiles the code in `debug` mode. If you want to change the profile to compile in `release` mode, you can provide the right flag.

```
cargo lambda build --release
```

When you compile your code in release mode, cargo-lambda will strip the binaries from all debug symbols to reduce the binary size.

#### Build - How does it work?

cargo-lambda uses [Zig](https://ziglang.org) and [cargo-zigbuild](https://crates.io/crates/cargo-zigbuild)
to compile the code for the right architecture. If Zig is not installed in your host machine, the first time that your run cargo-lambda, it will guide you through some installation options. If you run cargo-lambda in a non-interactive shell, the build process will fail until you install that dependency.

### Start

The start subcommand emulates the AWS Lambda control plane API. Run this command at the root of a Rust workspace and cargo-lambda will use cargo-watch to hot compile changes in your Lambda functions.

```
cargo lambda start
```

### Invoke

The invoke subcomand helps you send requests to the control plane emulator. To use this subcommand, you have to provide the name of the Lambda function that you want to invoke, and the payload that you want to send. When the control plane emulator receives the request, it will compile your Lambda function and handle your request.

### Invoke - Ascii data

The `--ascii-data` flag allows you to send a payload directly from the command line:

```
cargo lambda invoke basic-lambda --ascii-data '{"command": "hi"}'
```

### Invoke - File data

The `--file-data` flag allows you to read the payload from a file in your file system:

```
cargo lambda invoke basic-lambda --file-data examples/my-payload.json
```

### Invoke - Example data

The `--example-data` flag allows you to fetch an example payload from the [aws-lambda-events repository](https://github.com/LegNeato/aws-lambda-events/), and use it as your request payload. For example, if you want to use the [example-apigw-request.json](https://github.com/LegNeato/aws-lambda-events/blob/master/aws_lambda_events/src/generated/fixtures/example-apigw-request.json) payload, you have to pass the name `apigw-request` into this flag:

```
cargo lambda invoke http-lambda --example-data apigw-request
```

After the first download, these examples are cached in your home directory, under `.cargo/lambda/invoke-fixtures`.

[//]: # (badges)

[crate-image]: https://img.shields.io/crates/v/cargo-lambda.svg
[crate-link]: https://crates.io/crates/cargo-lambda
[build-image]: https://github.com/calavera/cargo-lambda/workflows/Build/badge.svg
[build-link]: https://github.com/calavera/cargo-lambda/actions?query=workflow%3ACI+branch%3Amain
