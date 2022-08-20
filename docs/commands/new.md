# cargo lambda new

The `new` command creates new Rust packages with a basic scheleton to help you start writing AWS Lambda functions with Rust. This command will create this package in a new sub-directory inside the directory where it's invoked. Run `cargo lambda new PACKAGE-NAME` to generate your new package.

This command uses templates packed as zip files, or from local directories. The [default template](https://github.com/cargo-lambda/default-template) supports HTTP Lambda functions, as well as functions that receive events defined in the [aws_lambda_events crate](https://crates.io/crates/aws-lambda-events). You can provide your own template using the `--template` flag.

The files `Cargo.toml`, `README.md`, and `src/main.rs` in the template are parsed with [Liquid](https://crates.io/crates/liquid) to dynamically render different files based on a series of global variables. You can see all the variables in [the source code](https://github.com/cargo-lambda/cargo-lambda/blob/main/crates/cargo-lambda-new/src/lib.rs#L167-L178).

After creating a new package, you can use the [build](#build) command to compile the source code.

## Extensions

You can also use this subcommand to create new Lambda Extension projects. Use the flag `--extension` to create the right project:

```sh
cargo lambda new --extension extension-project
```

### Logs extensions

If you want to build a Lambda Logs extension, add the `--logs` to the previous command. Cargo Lambda will create the scaffolding for a Logs extension:

```sh
cargo lambda new --extension --logs logs-project
```