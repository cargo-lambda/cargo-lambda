<script setup>
import SystemMessage from '../components/SystemMessage.vue'
</script>

# Getting Started

This section will help you build a Rust function for AWS Lambda from scratch. If you already have an existent function, and would like to build it for AWS Lambda, start from Step 4.

## Step 1: Install Cargo Lambda

<ClientOnly>
<SystemMessage>
<template v-slot:win>
You can use <a href="https://learn.microsoft.com/en-us/windows/package-manager/">WinGet</a> to install Cargo Lambda on Windows. Run the following command:

```sh
winget install CargoLambda.CargoLambda
```
</template>
<template v-slot:mac>
You can use <a href="https://brew.sh/">Homebrew</a> to install Cargo Lambda on macOS and Linux. Run the following command on your terminal install it:

```sh
brew install cargo-lambda/tap/cargo-lambda
```
</template>
<template v-slot:linux>
You can use <a href="https://curl.se/">Curl</a> to install Cargo Lambda on Linux:

```sh
curl -fsSL https://cargo-lambda.info/install.sh | sh
```
</template>
</SystemMessage>
</ClientOnly>

See all the ways that you can [install Cargo Lambda](/guide/installation) in your system if you need other options.

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

The [invoke](/commands/invoke) subcommand can send JSON payloads to the function running locally. Payloads are deserialized into strongly typed Rust structs, and the invoke call will fail if the payload doesn't have the right shape.

If you're starting with a basic function that only receives events with a `command` field in the payload, you can invoke them with the following command:

```sh
cargo lambda invoke --data-ascii "{ \"command\": \"hi\" }"
```

If you're starting an HTTP function, you can access it with your browser from the local endpoint: `http://localhost:9000/`. You can also invoke HTTP functions with the `invoke` subcommand, the payload to send will depend if this function receives calls from Amazon API Gateway, Amazon Elastic Load Balancer, or Lambda Function URLs.

If your function integrates with Amazon API Gateway, you can use one of the payload examples that we provide by using the `--data-example` flag:

```sh
cargo lambda invoke http-lambda --data-example apigw-request
```

Read more about the example flag in the [Invoke documentation](/commands/invoke.html#example-data).

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
            - name: Install Rust toolchain with Cargo Lambda
              uses: moonrepo/setup-rust@v1
              with:
                bins: cargo-lambda
            - name: Install Zig toolchain
              uses: mlugg/setup-zig@v1
              with:
                # Note: make sure you are using a recent version of zig (the one below isn't kept in sync with new releases)
                zig-version: 0.14.0
            # Add your build steps below
```

## AWS CDK Support

You can build and deploy Rust functions with Cargo Lambda and the AWS CDK with the [constructs in the Cargo Lambda CDK repository](https://github.com/cargo-lambda/cargo-lambda-cdk).
