# cargo lambda watch

The watch subcommand emulates the AWS Lambda control plane API. Run this command at the root of a Rust workspace and cargo-lambda will use cargo-watch to hot compile changes in your Lambda functions. Use flag `--no-reload` to avoid hot compilation.

::: warning
This command works best with the **[Lambda Runtime version 0.5.1](https://crates.io/crates/lambda_runtime/0.5.1)**. Previous versions of the runtime are likely to crash with serialization errors.
:::

```
cargo lambda watch
```

The function is not compiled until the first time that you try to execute it. See the [invoke](/commands/invoke) command to learn how to execute a function. Cargo will run the command `cargo run --bin FUNCTION_NAME` to try to compile the function. `FUNCTION_NAME` can be either the name of the package if the package has only one binary, or the binary name in the `[[bin]]` section if the package includes more than one binary.

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
cargo lambda watch \
  --env-vars FOO=BAR,BAZ=QUX \
  http-lambda
```

The flag `--env-var` allows you to pass several variables in the command line with the format `KEY=VALUE`. This flag overrides the previous one, and cannot be combined.

```
cargo lambda watch \
  --env-var FOO=BAR --env-var BAZ=QUX \
  http-lambda
```

The flag `--env-file` will read the variables from a file and add them to the function during the deploy. Each variable in the file must be in a new line with the same `KEY=VALUE` format:

```
cargo lambda watch --env-file .env http-lambda
```

## Function URLs

The emulator server includes support for [Lambda function URLs](https://docs.aws.amazon.com/lambda/latest/dg/lambda-urls.html) out of the box. Since we're working locally, these URLs are under the `/lambda-url` path instead of under a subdomain. The function that you're trying to access through a URL must respond to Request events using [lambda_http](https://crates.io/crates/lambda_http/), or raw `ApiGatewayV2httpRequest` events.

You can create functions compatible with this feature by running `cargo lambda new --http FUNCTION_NAME`.

To access a function via its HTTP endpoint, start the watch subcommand `cargo lambda watch`, then send requests to the endpoint `http://localhost:9000/lambda-url/FUNCTION_NAME`. You can add any additional path after the function name, or any query parameters.

## Enabling features

You can pass a list of features separated by comma to the `watch` command to load them during run:

```
cargo lambda watch --features feature-1,feature-2
```


## Debug with breakpoints

You have two options to debug your application, set breakpoints, and step through your code using a debugger like GDB or LLDB.

The first option is to let `cargo-lambda` start your function and manually attach your debugger to the newly created process that hosts your function.
Suppose the flag `--no-reload` is not set and you modify and save the function's source code.
In that case, `cargo-lambda` automatically terminates the function's process, rebuilds the executable and restarts it.
If the flag `--no-reload` is set, you must manually restart `cargo-lambda` watch.
In both cases, the debugger has to be reattached to the new process.

The second option is to let `cargo-lambda` provide the lambda runtime API for your function by setting the flag `--only-lambda-apis` and manually starting the lambda function from your IDE in debug mode.
This way, the debugger is attached to the new process automatically by your IDE.
When you modify your function's source code, let your IDE rebuild and relaunch the function and reattach the debugger to the new process.
The drawback of the second option is that essential environment variables are not provided automatically to your function by `cargo-lambda` but have to be configured in your IDE's launch configuration.
If you provide a function name when you invoke the function, you must replace `@package-bootstrap@` with that name.

```
 AWS_LAMBDA_FUNCTION_VERSION=1
 AWS_LAMBDA_FUNCTION_MEMORY_SIZE=4096
 AWS_LAMBDA_RUNTIME_API=http://[::]:9000/.rt/@package-bootstrap@
 AWS_LAMBDA_FUNCTION_NAME=@package-bootstrap@
```

These environment variables are also mentioned as info messages in the log output by `cargo-lambda`.

## Release mode

You can also run your code in release mode if needed when the emulator is loaded:

```
cargo lambda watch --release
```