# Cargo Lambda Invoke

The `invoke` subcommand helps you send requests to the local Lambda emulator, as well as remote functions. Before you use this command, you need to start the local emulator in a different terminal by calling the [Watch subcommand](/commands/watch).

The `invoke` subcommand sends raw JSON payloads to your Rust functions. The Rust Runtime for Lambda transforms those payloads into the Rust struct that your function receives. If the payload doesn't match the layout of the defined struct, the runtime will return a Serde deserialization error.

Rust functions implemented with `lambda_http` require the HTTP calls to be wrapped into the right JSON payloads. This is because AWS Lambda doesn't support HTTP calls natively. Services like Amazon API Gateway, Amazon Load Balancer, or Lambda Function URLs, receive the incoming HTTP calls and translate them to JSON payloads. `lambda_http` performs the opposite translation, so you can work with HTTP primitives.

You can find many examples of JSON payloads in the [AWS Lambda Events repository](https://github.com/calavera/aws-lambda-events/tree/main/src/fixtures). You can copy them directly, or use the [--data-example flag](/commands/invoke.html#example-data) to load them on demand.

The following video shows you how to use this subcommand:

<iframe width="560" height="315" src="https://www.youtube.com/embed/2MAuMihVlO0" title="YouTube video player" frameborder="0" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen></iframe>

## Basic JSON calls

If your Rust project only includes one function, in the package's main.rs file, you can invoke it by sending the data that you want to process, without extra arguments. For example:

```
cargo lambda invoke --data-ascii "{ \"command\": \"hi\" }"
```

## Multi function support

If your project includes more than one function, or the binary has a different name than the package, you must provide the name of the Lambda function that you want to invoke, and the payload that you want to send. If you don't know how to find your function's name, it can be in two places:

- If your Cargo.toml file includes a `[package]` section, and it does **not** include a `[[bin]]` section, the function's name is in the `name` attribute under the `[package]` section.
- If your Cargo.toml file includes one or more `[[bin]]` sections, the function's name is in the `name` attribute under the `[[bin]]` section that you want to compile.

In the following example, `basic-lambda` is the function's name as indicated in the package's `[[bin]]` section:

```
cargo lambda invoke basic-lambda --data-ascii "{ \"command\": \"hi\" }"
```

Cargo-Lambda compiles functions on demand when they receive the first invocation. It's normal that the first invocation takes a long time if your code has not compiled with the host compiler before. After the first compilation, Cargo-Lambda will re-compile your code every time you make a change in it, without having to send any other invocation requests.

## Ascii data

The `--data-ascii` flag allows you to send a payload directly from the command line:

```
cargo lambda invoke basic-lambda --data-ascii "{ \"command\": \"hi\" }"
```

## File data

The `--data-file` flag allows you to read the payload from a file in your file system:

```
cargo lambda invoke basic-lambda --data-file examples/my-payload.json
```

## Example data

The `--data-example` flag allows you to fetch an example payload from the [aws-lambda-events repository](https://github.com/awslabs/aws-lambda-rust-runtime/tree/main/lambda-events), and use it as your request payload. For example, if you want to use the [example-apigw-request.json](https://github.com/awslabs/aws-lambda-rust-runtime/tree/main/lambda-events/src/fixtures/example-apigw-request.json) payload, you have to pass the name `apigw-request` into this flag:

```
cargo lambda invoke http-lambda --data-example apigw-request
```

After the first download, these examples are cached (in your system's user-local cache) in your home directory, under `cargo-lambda/invoke-fixtures`.

If you don't want to cache the example or want to ignore the file in the cache, add the flag `--skip-cache` to the command:

```
cargo lambda invoke http-lambda --data-example apigw-request --skip-cache
```

## Remote

The `--remote` flag allows you to send requests to a remote function deployed on AWS Lambda. This flag assumes that your AWS account has permission to call the `lambda:invokeFunction` operation. You can specify the region where the function is deployed, as well as any credentials profile that the command should use to authenticate you:

```
cargo lambda invoke --remote --data-example apigw-request http-lambda
```

## Output format

The `--output-format` flag allows you to change the output formatting between plain text and pretty-printed JSON formatting. By default, all function outputs are printed as text.

```
cargo lambda invoke --remote --data-example apigw-request --output-format json http-lambda
```
