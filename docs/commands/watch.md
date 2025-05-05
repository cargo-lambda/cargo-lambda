<script setup>
import SystemMessage from '../components/SystemMessage.vue'
</script>

# Cargo Lambda Watch

The watch subcommand emulates the AWS Lambda control plane API. Run this command at the root of a Rust workspace and cargo-lambda will use cargo-watch to hot compile changes in your Lambda functions.

```
cargo lambda watch
```

The function is not compiled until the first time that you try to execute it. See the [invoke](/commands/invoke) command to learn how to execute a function. Cargo will run the command `cargo run --bin FUNCTION_NAME` to try to compile the function. `FUNCTION_NAME` can be either the name of the package if the package has only one binary, or the binary name in the `[[bin]]` section if the package includes more than one binary.

The following video shows how you can use this subcommand to develop functions locally:

<iframe width="560" height="315" src="https://www.youtube.com/embed/Rf1VewhIrqM" title="YouTube video player" frameborder="0" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen></iframe>

## Environment variables

If you need to set environment variables for your function to run, you can specify them in the metadata section of your Cargo.toml file.

Use the section `package.metadata.lambda.env` to set global variables that will applied to all functions in your package:

```toml
[package]
name = "basic-lambda"

[package.metadata.lambda.env]
RUST_LOG = "debug"
MY_CUSTOM_ENV_VARIABLE = "custom value"
```

If you have more than one function in the same package, and you want to set specific variables for each one of them, you can use a section named after each one of the binaries in your package, `package.metadata.lambda.bin.BINARY_NAME`:

```toml
[package]
name = "lambda-project"

[package.metadata.lambda.env]
RUST_LOG = "debug"

[package.metadata.lambda.bin.get-product.env]
GET_PRODUCT_ENV_VARIABLE = "custom value"

[package.metadata.lambda.bin.add-product.env]
ADD_PRODUCT_ENV_VARIABLE = "custom value"

[[bin]]
name = "get-product"
path = "src/bin/get-product.rs"

[[bin]]
name = "add-product"
path = "src/bin/add-product.rs"
```

You can also set environment variables on a workspace

```toml
[workspace.metadata.lambda.env]
RUST_LOG = "debug"

[workspace.metadata.lambda.bin.get-product.env]
GET_PRODUCT_ENV_VARIABLE = "custom value"
```

These behave in the same way, package environment variables will override workspace settings, the order of precedence is:

1) Package Binary
2) Package Global
3) Workspace Binary
4) Workspace Global

You can also use the flag `--env-vars` to add environment variables. This flag supports a comma separated list of values:

```
cargo lambda watch --env-vars FOO=BAR,BAZ=QUX
```

The flag `--env-var` allows you to pass several variables in the command line with the format `KEY=VALUE`. This flag overrides the previous one, and cannot be combined.

```
cargo lambda watch --env-var FOO=BAR --env-var BAZ=QUX
```

The flag `--env-file` will read the variables from a file and add them to the function during the deploy. Each variable in the file must be in a new line with the same `KEY=VALUE` format:

```
cargo lambda watch --env-file .env
```

## Function URLs

The emulator server includes support for [Lambda function URLs](https://docs.aws.amazon.com/lambda/latest/dg/lambda-urls.html) out of the box. Since we're working locally, these URLs are under the `/lambda-url` path instead of under a subdomain. The function that you're trying to access through a URL must respond to Request events using [lambda_http](https://crates.io/crates/lambda_http/), or raw `ApiGatewayV2httpRequest` events.

You can create functions compatible with this feature by running `cargo lambda new --http FUNCTION_NAME`.

To access a function via its HTTP endpoint, start the watch subcommand `cargo lambda watch`, then send requests to the endpoint `http://localhost:9000`. You can add any additional path, or any query parameters.

::: warning
Your function MUST have the `apigw_http` feature enabled in the `lambda_http` dependency for Function URLs to work. The payload that AWS sends is only compatible with the `apigw_http` format, not with the `apigw_rest` format.
:::

### Multi-package projects

If your project includes several functions under the same package, you can access them using the function's name as the prefix in the request path `http://localhost:9000/lambda-url/FUNCTION_NAME`. You can also add any additional path after the function name, or any query parameters.

If only one binary package in your project is a Lambda function, you can specify the package or binary that you want to work with by using the `--package` and `--bin` flags. This way, only the function in the specified package will be available through the Function URL.

```
cargo lambda watch --package my-package
cargo lambda watch --bin my-binary
```

When you use these flags, only one function will be available through the Function URL, you can access it by using the root path `http://localhost:9000`.

You can also use the advanced routing feature to specify the routes for the function URLs. See the [Custom HTTP routes](/commands/watch#custom-http-routes) section for more information.

## Lambda response streaming

When you work with function URLs, you can stream responses to the client with [Lambda's support for Streaming Responses](https://aws.amazon.com/blogs/compute/introducing-aws-lambda-response-streaming/).

Start the watch command in a function that uses the Response Streaming API, like the [example function in the Runtime's repository](https://github.com/awslabs/aws-lambda-rust-runtime/tree/main/examples/basic-streaming-response):

```
cargo lambda watch
```

Then use cURL to send requests to the Lambda function. You'll see that the client starts printing the response as soon as it receives the first chunk of data, without waiting to have the complete response:

```
curl http://localhost:9000
```

## Enabling features

You can pass a list of features separated by comma to the `watch` command to load them during run:

```
cargo lambda watch --features feature-1,feature-2
```

## Debug with breakpoints

You have two options to debug your application, set breakpoints, and step through your code using a debugger like GDB or LLDB.

The first option is to let Cargo Lambda start your function and manually attach your debugger to the newly created process that hosts your function. This option automatically terminates the function's process, rebuilds the executable and restarts it when your code changes. The debugger must be reattached to the process when the function every time the function boots.

The second option is to let Cargo Lambda provide the Lambda runtime APIs for your function by setting the flag `--only-lambda-apis`, and manually starting the lambda function from your IDE in debug mode. This way, the debugger is attached to the new process automatically by your IDE. When you modify your function's source code, let your IDE rebuild and relaunch the function and reattach the debugger to the new process.

The drawback of the second option is that essential environment variables are not provided automatically to your function by Cargo Lambda, but have to be configured in your IDE's launch configuration. If you provide a function name when you invoke the function, you must replace `_` with that name.

<ClientOnly>
<SystemMessage>
<template v-slot:win>
In PowerShell, you can export these variables with the following commands:

```
$env:AWS_LAMBDA_FUNCTION_VERSION="1"
$env:AWS_LAMBDA_FUNCTION_MEMORY_SIZE="4096"
$env:AWS_LAMBDA_RUNTIME_API="http://127.0.0.1:9000/.rt"
$env:AWS_LAMBDA_FUNCTION_NAME="_"
```
</template>

<template v-slot:mac>
In your terminal, you can export these variables with the following commands:

```
export AWS_LAMBDA_FUNCTION_VERSION=1
export AWS_LAMBDA_FUNCTION_MEMORY_SIZE=4096
export AWS_LAMBDA_RUNTIME_API=http://[::]:9000/.rt
export AWS_LAMBDA_FUNCTION_NAME=_
```
</template>

<template v-slot:linux>
In your terminal, you can export these variables with the following commands:

```
export AWS_LAMBDA_FUNCTION_VERSION=1
export AWS_LAMBDA_FUNCTION_MEMORY_SIZE=4096
export AWS_LAMBDA_RUNTIME_API=http://[::]:9000/.rt
export AWS_LAMBDA_FUNCTION_NAME=_
```
</template>

<template v-slot:fallback>
In your terminal, you can export these variables with the following commands:

```
export AWS_LAMBDA_FUNCTION_VERSION=1
export AWS_LAMBDA_FUNCTION_MEMORY_SIZE=4096
export AWS_LAMBDA_RUNTIME_API=http://[::]:9000/.rt
export AWS_LAMBDA_FUNCTION_NAME=_
```
</template>
</SystemMessage>
</ClientOnly>

These environment variables are also mentioned as info messages in the log output by `cargo-lambda`.

## Ignore changes

If you want to run the emulator without hot reloading the function every time there is a change in the code, you can use the flag `--ignore-changes`:

```
cargo lambda watch --ignore-changes
```

## Release mode

You can also run your code in release mode if needed when the emulator is loaded:

```
cargo lambda watch --release
```

## Working with extensions

You can boot extensions locally that can be associated to a function running under the `watch` command.

In the terminal where your Lambda function code lives, run Cargo Lambda as usual `cargo lambda watch`.

In the terminal where your Lambda extension code lives, export the runtime api endpoint as an environment variable, and run your extension with `cargo run`:

<ClientOnly>
<SystemMessage>
<template v-slot:win>
In PowerShell, you can do that with the following commands:

```
$env:AWS_LAMBDA_RUNTIME_API="http://127.0.0.1:9000/.rt"
cargo run
```
</template>

<template v-slot:mac>
In your terminal, you can do that with the following commands:

```
export AWS_LAMBDA_RUNTIME_API=http://[::]:9000/.rt
cargo run
```
</template>

<template v-slot:linux>
In your terminal, you can do that with the following commands:

```
export AWS_LAMBDA_RUNTIME_API=http://[::]:9000/.rt
cargo run
```
</template>

<template v-slot:fallback>
In your terminal, you can do that with the following commands:

```
export AWS_LAMBDA_RUNTIME_API=http://[::]:9000/.rt
cargo run
```
</template>
</SystemMessage>
</ClientOnly>

This will make your extension to send requests to the local runtime to register the extension and subscribe to events. If your extension subscribes to `INVOKE` events, it will receive an event every time you invoke your function locally. If your extension subscribes to `SHUTDOWN` events, it will receive an event every time the function is recompiled after code changes.

::: warning
At the moment Log and Telemetry extensions don't receive any data from the local runtime.
:::

The following video shows you how to use the watch subcommand with Lambda extensions:

<iframe width="560" height="315" src="https://www.youtube.com/embed/z2sv41ukHTE" title="YouTube video player" frameborder="0" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen></iframe>

## Graceful shutdown support

The watch subcommand will propagate `SIGTERM` and `SIGINT` signals to your function, with a delay to allow for graceful shutdown logic to fire.

This makes a best-effort attempt to replicate signal handling in AWS Lambda, including allowed shutdown delay, but does not strictly match AWS orchestration. Note that graceful shutdown is only enabled in AWS if extensions are registered. This means that if your function has no registered extensions, it will only receive a `SIGKILL`.

One easy way to trigger a graceful shutdown in a Posix shell is by using `Control-C` in the session
running your `cargo lambda watch` process.

Some caveats:
- Graceful shutdown signals are only propagated to the function's process, not that of external extensions.
- Graceful shutdown signals are not sent properly on hot reload, only direct signaling to the `cargo watch` process.
- No guarantees are made around sequencing of runtime shutdown and extensions receiving `SHUTDOWN` events.

For more information on graceful shutdown handling, refer to:
- [AWS documentation](https://docs.aws.amazon.com/lambda/latest/dg/lambda-runtime-environment.html#runtimes-lifecycle-shutdown)
- [AWS Samples repository](https://github.com/aws-samples/graceful-shutdown-with-aws-lambda)

## TLS support

The watch subcommand supports TLS connections to the runtime if you want to send requests to the runtime securely.

To enable TLS, you need to provide a TLS certificate and key. You can use the `--tls-cert` and `--tls-key` flags to specify the path to the certificate and key files. The certificate and key files must be in PEM format.

```
cargo lambda watch --tls-cert cert.pem --tls-key key.pem
```

If the root CA file is not specified, the local CA certificates on your system will be used to verify the TLS connection. You can use the `--tls-ca` flag to specify a custom root CA file.

```
cargo lambda watch --tls-cert cert.pem --tls-key key.pem --tls-ca ca.pem
```

If you always want to use TLS, you can place the certificate and key files in the global configuration directory as defined by XDG_CONFIG_HOME. Cargo Lambda will automatically look for
those files in a subdirectory called `cargo-lambda`. The file names must be `cert.pem`, `key.pem`, and `ca.pem` respectively.

```
tree $HOME/.config/cargo-lambda
/home/david/.config/cargo-lambda
├── cert.pem
└── key.pem

1 directory, 2 files

```

::: tip
We recommend using [mkcert](https://github.com/FiloSottile/mkcert) to generate the TLS certificate and key files for development purposes.
:::

## Custom HTTP routes

You can add custom HTTP routes to the emulator by setting the `routes` field in the `watch` section of your Cargo.toml file. This is useful if you have several functions in your package and you want to access them using paths without the `/lambda-url` prefix.

This configuration can be managed at the workspace level when you have more than one function in your workspace, or at the package level if you want to separate the routes for each package. Routes at the package level will override the ones in the workspace.

Cargo Lambda uses [Matchit](https://crates.io/crates/matchit/) to match the HTTP routes to the functions. The syntax to specify the route paths is similar to the one used by the [Axum router](https://docs.rs/axum/latest/axum/routing/index.html).

Each route is a key-value pair where the key is the path and the value is either a string with the function name, or a table with the HTTP method to match and the function name.

### Workspace level

This configuration is applied to all functions in your workspace.

```toml
[workspace.metadata.lambda.watch.router]
"/get-product/{id}" = "get-product"
"/add-product" = "add-product"
"/users" = [
    { method = "GET", function = "get-users" },
    { method = "POST", function = "add-user" }
]
```

### Package level

This configuration is applied to a function in a package. It will be merged with the workspace level configuration if it exists.

```toml
[package.metadata.lambda.watch.router]
"/products" = "handle-products"
```

## Ignore files from hot reloading

Cargo Lambda supports ignore files and directories to avoid hot reloading when certain files are modified. This is useful to avoid unnecessary recompilations when the files are not relevant to the function.

The ignore files are discovered from the following sources:

- Git ignore rules (`.gitignore`)
- Files in the system using the keywords `CARGO_LAMBDA` and `cargo-lambda`:
  - `$HOME/.cargo-lambda/ignore`
  - `$XDG_CONFIG_HOME/cargo-lambda/ignore`
  - `$APPDATA/cargo-lambda/ignore`
  - `$USERPROFILE/.cargo-lambda/ignore`

  - `$HOME/.CARGO_LAMBDA/ignore`
  - `$XDG_CONFIG_HOME/CARGO_LAMBDA/ignore`
  - `$APPDATA/CARGO_LAMBDA/ignore`
  - `$USERPROFILE/.CARGO_LAMBDA/ignore`
- A file named `.cargolambdaignore` in the root of the project.
- A file specified in the `CARGO_LAMBDA_IGNORE_FILE` environment variable.

The ignore files are merged together and used to create glob patterns that are used to match the files that will be ignored.

The syntax of the ignore files is the same as the one used by [Git](https://git-scm.com/docs/gitignore).

```
*.rs
*.toml
*.lock
static/**
```
