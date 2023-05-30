<script setup>
import SystemMessage from '../components/SystemMessage.vue'
</script>

# Getting Started

This section will help you build a Rust function for AWS Lambda from scratch. If you already have an existent function, and would like to build it for AWS Lambda, start from Step 4.

## Step 1: Install Cargo Lambda

<ClientOnly>
<SystemMessage>
<template v-slot:win>
You can use <a href="https://scoop.sh/">Scoop</a> to install Cargo Lambda on Windows. Run the following commands to add our bucket, and install it:

```sh
scoop bucket add cargo-lambda https://github.com/cargo-lambda/scoop-cargo-lambda
scoop install cargo-lambda/cargo-lambda
```
</template>
<template v-slot:mac>
You can use <a href="https://brew.sh/">Homebrew</a> to install Cargo Lambda on macOS and Linux. Run the following commands on your terminal to add our tap, and install it:

```sh
brew tap cargo-lambda/cargo-lambda
brew install cargo-lambda
```
</template>
<template v-slot:linux>
You can use <a href="https://pypi.org/">PyPI</a> to install Cargo Lambda on Linux:

```sh
pip3 install cargo-lambda
```
</template>
</SystemMessage>
</ClientOnly>

See all the ways that you can use to [install Cargo Lambda](/guide/installation) in your system.

## Step 2: Create a new project

The [new](/commands/new) subcommand will help you create a new project with a default template. When that's done, change into the new directory:

```sh
cargo lambda new new-lambda-project \
    && cd new-lambda-project
```

::: tip
Add the flag `--http` if you want to automatically generate an HTTP function that integrates with Amazon API Gateway, Amazon Elastic Load Balancer, and AWS Lambda Function URLs.
:::

## Step 3: Serve the function locally for testing

Run the Lambda emulator built in with the [watch](/commands/watch) subcommand:

```sh
cargo lambda watch
```

## Step 4: Test your function

The [invoke](/commands/invoke) subcommand can send payloads to the function running locally:

```sh
cargo lambda invoke --data-ascii "{ \"command\": \"hi\" }"
```

If you're testing an HTTP function, you can access it with your browser from the local endpoint: `http://localhost:9000/lambda-url/new-lambda-project`.

## Step 5: Build the function to deploy it on AWS Lambda

Use the [build](/commands/build) subcommand to compile your function for Linux systems:

```sh
cargo lambda build --release
```

::: tip
Add the flag `--arm64` if you want to use Graviton processors on AWS Lambda
:::

Check out the [build](/commands/build) subcommand docs to learn how to compile multiple functions in the same project.

## Step 6: Deploy the function on AWS Lambda

Use the [deploy](/commands/deploy) subcommand to upload your function to AWS Lambda. This subcommand requires AWS credentials in your system.

```sh
cargo lambda deploy
```

::: info
A default execution role for this function will be created when you execute this command. Use the flag `--iam-role` if you want to use a predefined IAM role.
:::

## Debugging

Use the flag `--verbose` with any subcommand to enable tracing instrumentation. You can also enable instrumentation with the following environment variable `RUST_LOG=cargo_lambda=trace`.

## GitHub Actions

If you want to use Cargo Lambda in a GitHub Action workflow, you can use one of the predefined actions that download release binaries from GitHub Releases.

The following example shows the steps to install Rust, Zig, and Cargo Lambda on a Linux x86-64 workflow:

```yaml
jobs:
    build:
        runs-on: ubuntu-latest
        steps:
            - name: Install Rust toolchain
              uses: dtolnay/rust-toolchain@stable
            - name: Install Zig toolchain
              uses: korandoru/setup-zig@v1
              with:
                zig-version: 0.10.0
            - name: Install Cargo Lambda
              uses: jaxxstorm/action-install-gh-release@v1.9.0
              with:
                repo: cargo-lambda/cargo-lambda
                tag: v0.14.0 # Remove this if you want to grab always the latest version
                platform: linux # Other valid options: 'windows' or 'darwin'
                arch: x86_64 # Other valid options for linux: 'aarch64'
            # Add your build steps below
```

## AWS CDK Support

You can build and deploy Rust functions with Cargo Lambda and the AWS CDK with the [constructs in the Cargo Lambda CDK repository](https://github.com/cargo-lambda/cargo-lambda-cdk).
