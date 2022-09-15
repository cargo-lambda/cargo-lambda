# cargo lambda new

The `new` command creates new Rust packages with a basic skeleton to help you start writing AWS Lambda functions with Rust. This command will create this package in a new sub-directory inside the directory where it's invoked. Run `cargo lambda new PACKAGE-NAME` to generate your new package.

This command uses templates packed as zip files, or from local directories. The [default template](https://github.com/cargo-lambda/default-template) supports HTTP Lambda functions, as well as functions that receive events defined in the [aws_lambda_events crate](https://crates.io/crates/aws-lambda-events). You can provide your own template using the `--template` flag.

The files `Cargo.toml`, `README.md`, and `src/main.rs` in the template are parsed with [Liquid](https://crates.io/crates/liquid) to dynamically render different files based on a series of global variables. You can see all the variables in [the source code](https://github.com/cargo-lambda/cargo-lambda/blob/main/crates/cargo-lambda-new/src/lib.rs#L167-L178).

After creating a new package, you can use the [build](/commands/build) command to compile the source code.

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

## Templates

Cargo Lambda uses template repositories as scaffolding for new projects. You can see the [default template for functions](https://github.com/cargo-lambda/default-template) and the [default template for extensions](https://github.com/cargo-lambda/default-extension-template) in GitHub.

Cargo Lambda can also download custom templates from other GitHub repositories by using the `--template` with the repository url, for example:

```sh
cargo lambda new \
    --template https://github.com/calavera/custom-template \
    new-project
```

The `--template` flag also accepts routes to specific repository branches and tags:

```sh
cargo lambda new \
    --template https://github.com/calavera/custom-template/branch/stable \
    new-project
```

```sh
cargo lambda new \
    --template https://github.com/calavera/custom-template/tag/v0.1.0 \
    new-project
```

### Template rendering

Cargo Lambda uses [Liquid](https://shopify.github.io/liquid/) to render files from a given template.

By default, only a few files in a template are rendered by Liquid, the rest are copied into the new project's directory as they are. These are the files rendered by Liquid by default:

- README.md
- Cargo.toml
- src/main.rs
- src/lib.rs
- src/bin/*.rs

If you want to render additional files you can use the flag `--render-file` with a path relative to the root of the directory where the project is created:

```sh
cargo lambda new \
    --template https://github.com/calavera/custom-template \
    --render-file package.json \
    --render-file lib/cdk-stack.ts \
    new-project
```

### Template variables

When you create a new project, Cargo Lambda adds several variables to the template engine that you can use in any file that's rendered by Liquid.

These are the variables for function templates:

- project_name: The name of the project and package.
- bin_name: The name of the main binary to compile if it's different than the project name.
- http_function: Whether the function is an http function.
- http_feature: the lambda event feature type that integrates with an http function.
- event_type: the Rust event type that the function receives.
- event_type_feature: the lambda event feature name in the aws_lambda_events crate.
- event_type_import: The Rust import statement that the function uses.

These are the variables for extension templates:

- project_name: The name of the project and package.
- bin_name: The name of the main binary to compile if it's different than the project name.
- logs: Whether the extension is a Logs extension or not.

You can add additional variables to render by a template with the flag `--render-var`. This flag takes variables in the format `KEY=VALUE`:

```sh
cargo lambda new \
    --template https://github.com/calavera/custom-template \
    --render-var CDK_VERSION=2 \
    --render-var CDK_STACK=lambda-stack.ts \
    new-project
```

### Ignore files

By default, Cargo Lambda will ignore the `.git` directory and the `LICENSE` file in the template repository. If you want to ignore additional files in a new project, you can use the flag `--ignore-file`:

```sh
cargo lambda new \
    --template https://github.com/calavera/custom-template \
    --ignore-file package-lock.json \
    --ignore-file yarn-lock.json \
    new-project
```

## Interactive options

Cargo Lambda's `new` subcommand displays several interactive questions for the default templates to work. If you have a custom template and you want to skip these questions, you can use the flag `--no-interactive`:

```sh
cargo lambda new \
    --template https://github.com/calavera/custom-template \
    --no-interactive \
    new-project
```
