# Cargo Lambda New

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

Cargo Lambda can also download custom templates from other public GitHub repositories by using the `--template` with the repository url, for example:

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

### Private template repositories

If you want to use a template that's in a private repository, Cargo Lambda uses the same method as `git clone` to download the repository. This means that you need to have access to the repository and that you need to have the credentials to access it configured in your machine.

To download a private repository with SSH, you need to have the SSH key configured in your machine. You can use the same SSH URLs as you use with `git clone`.

```sh
cargo lambda new \
    --template git@github.com:cargo-lambda/custom-template.git \
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

You can also use Liquid variables in file paths themselves. This allows you to dynamically generate file paths based on template variables:

```sh
cargo lambda new \
    --template https://github.com/calavera/custom-template \
    --render-var ci_provider=.github \
    new-project
```

For example, a template containing a file path like `{{ci_provider}}/workflows/build.yml` would be rendered as `.github/workflows/build.yml` with the above command.


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

## Custom templates

Cargo Lambda allows you to create custom templates with interactive prompts to collect user input during project creation. To create a custom template, add a `CargoLambda.toml` file to your template repository with the following structure. The template files can be placed either in the root directory or in a subdirectory called `template`. Cargo Lambda will automatically detect and use the `template` subdirectory if it exists. The `CargoLambda.toml` file can be located in the root directory or in the `template` directory. If you don't want to add it to the final project directory, it's recommended to place it in the root directory, and put the template files in the `template` directory.

```toml
[template]
# Disable the default interactive prompts that cargo-lambda shows
disable_default_prompts = true

# Specify which files should be processed by the Liquid template engine
render_files = [
    "Cargo.toml",
    "README.md",
    "src/main.rs"
]

# Process all files in the template with Liquid (overrides render_files)
render_all_files = true

# Files to ignore when copying the template
ignore_files = [
    "README.md"
]

# Files to conditionally render based on a promptvariable
[template.render_conditional_files] 
".github" = { var = "github_actions", value = true }

# Define custom interactive prompts
[template.prompts]
project_description = { message = "What is the description of your project?", default = "My Lambda" }
enable_tracing = { message = "Would you like to enable tracing?", default = false }
runtime = { message = "Which runtime would you like to use?", choices = ["provided.al2023", "provided.al2"], default = "provided.al2023" }
architecture = { message = "Which architecture would you like to target?", choices = ["x86_64", "arm64"], default = "x86_64" }
memory = { message = "How much memory (in MB) would you like to allocate?", default = "128" }
timeout = { message = "What timeout (in seconds) would you like to set?", default = "3" }
github_actions = { message = "Would you like to add GitHub Actions CI/CD support?", default = false }
```

## Configuration Options

- `disable_default_prompts`: When set to `true`, disables Cargo Lambda's built-in prompts
- `render_files`: List of files that should be processed by the Liquid template engine
- `render_all_files`: When `true`, all files in the template will be processed by Liquid
- `render_conditional_files`: Table of files that should be conditionally rendered based on a prompt variable
- `ignore_files`: List of files that should not be copied to the new project
- `prompts`: Table of interactive prompts to collect user input

### Prompt Configuration

Each prompt can have the following properties:

- `name`: Variable name to use in templates (required)
- `message`: Question to display to the user (required)
- `default`: Default value if user doesn't provide input (optional)
- `choices`: Array of valid options for the user to choose from (optional)

The values collected from these prompts are available in your template files through Liquid variables. For example:

```toml
[package]
name = "{{ project_name }}"
description = "{{ project_description }}"
```

```rust
#[function_name = "{{ project_name }}"]
#[tracing(enable = {{ enable_tracing }})]
pub async fn handler() -> Result<()> {
    // ...
}
```

To use your custom template:

```sh
cargo lambda new \
    --template https://github.com/your-org/custom-template \
    new-project
```

If you want to skip the interactive prompts, use the `--no-interactive` flag:

```sh
cargo lambda new \
    --template https://github.com/your-org/custom-template \
    --no-interactive \
    new-project
```
