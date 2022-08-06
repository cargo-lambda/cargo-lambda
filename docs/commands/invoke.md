# cargo lambda invoke

The invoke subcomand helps you send requests to the control plane emulator, as well as remote functions.

If your Rust project only includes one function, in the package's main.rs file, you can invoke it by sending the data that you want to process, without extra arguments. For example:

```
$ cargo lambda invoke --data-ascii '{"command": "hi"}'
```

If your project includes more than one function, or the binary has a different name than the package, you must provide the name of the Lambda function that you want to invoke, and the payload that you want to send. If you don't know how to find your function's name, it can be in two places:

- If your Cargo.toml file includes a `[package]` section, and it does **not** include a `[[bin]]` section, the function's name is in the `name` attribute under the `[package]` section.
- If your Cargo.toml file includes one or more `[[bin]]` sections, the function's name is in the `name` attribute under the `[[bin]]` section that you want to compile.

In the following example, `basic-lambda` is the function's name as indicated in the package's `[[bin]]` section:

```
$ cargo lambda invoke basic-lambda --data-ascii '{"command": "hi"}'
```

Cargo-Lambda compiles functions on demand when they receive the first invocation. It's normal that the first invocation takes a long time if your code has not compiled with the host compiler before. After the first compilation, Cargo-Lambda will re-compile your code every time you make a change in it, without having to send any other invocation requests.

## Ascii data

The `--data-ascii` flag allows you to send a payload directly from the command line:

```
cargo lambda invoke basic-lambda --data-ascii '{"command": "hi"}'
```

## File data

The `--data-file` flag allows you to read the payload from a file in your file system:

```
cargo lambda invoke basic-lambda --data-file examples/my-payload.json
```

## Example data

The `--data-example` flag allows you to fetch an example payload from the [aws-lambda-events repository](https://github.com/LegNeato/aws-lambda-events/), and use it as your request payload. For example, if you want to use the [example-apigw-request.json](https://github.com/LegNeato/aws-lambda-events/blob/master/aws_lambda_events/src/generated/fixtures/example-apigw-request.json) payload, you have to pass the name `apigw-request` into this flag:

```
cargo lambda invoke http-lambda --data-example apigw-request
```

After the first download, these examples are cached in your home directory, under `.cargo/lambda/invoke-fixtures`.

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
