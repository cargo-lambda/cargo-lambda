# Configuring Cargo Lambda

Across all the documentation, you'll notice that most of the examples are using flags to configure the behavior of Cargo Lambda. This is because Cargo Lambda is designed to be used as a command line tool, and it's easy to use flags to configure the behavior of the tool.

However, Cargo Lambda also allows you to configure the behavior of the CLI through configuration files and environment variables. All options available via flags in the CLI for the `build`, `deploy`, and `watch` commands are also available in the configuration files and environment variables. The options for `new`, `init`, `invoke`, and `system` commands are not available in the configuration files and environment variables.

## Loading order

Cargo Lambda loads the configuration in the following order:

1. Environment variables that start with `CARGO_LAMBDA_`, if any.
2. Global `CargoLambda.toml` file, if it exists in the current directory, or if the `--global` option is used to specify a path location.
3. Cargo workspace metadata, if any.
4. Cargo package metadata, if any.
5. CLI flags, if any.

Each one of these sources overrides the values of the previous ones. For example, if you set the `memory` option in a configuration file, and then you also specify the `--memory` option on the command line, the value specified on the command line will override the value specified in the configuration file.

### Merge array behavior

Some configuration options, like `env`, `include`, and `router`, are arrays. By default, these arrays override the values from the previous sources. However, in some cases, you might want to merge the arrays instead of overriding them. This behavior can be changed by using the `--admerge` flag in the CLI. This option is only available through the CLI, and through the `CARGO_LAMBDA_ADMERGE` environment variable. When this option is enabled, array values from the CLI flags will merge with the values from the configuration files.

## Environment variables

Environment variables are always loaded first, any configuration files loaded after that, or flags in the CLI will override the values from the environment variables.

Environment variables start with `CARGO_LAMBDA_`, and are always in the format of `CARGO_LAMBDA_<SUBCOMMAND>.<OPTION>`. The valid subcommands are `build`, `deploy`, and `watch`. For example, the `memory` option in the `deploy` section would be `CARGO_LAMBDA_DEPLOY.MEMORY`. The `release` option in the `build` section would be `CARGO_LAMBDA_BUILD.RELEASE`.

:::tip

Environment variables that represent lists of options, like `features`, `layers`, or `tags`, MUST be wrapped in square brackets. For example, `CARGO_LAMBDA_BUILD.FEATURES` must be `CARGO_LAMBDA_BUILD.FEATURES="[lambda, env]"`.

:::

## Global configuration files

The configuration file is a TOML file named `CargoLambda.toml` that is usually located in the root of your project. You can specify a different location using the `--global` option.

```sh
cargo lambda build --global /path/to/config.toml
```

### Configuration contexts

You can also specify a context to load the configuration for. This is useful if you want to have different configurations for different environments, like `dev`, `staging`, and `production` inside the same global configuration file. Given the following `CargoLambda.toml` file:

```toml
# loaded when no context is specified
[deploy]
memory = 64

# loaded when the dev profile is selected
[dev.deploy]
memory = 128

# loaded when the staging profile is selected
[staging.deploy]
memory = 256

# loaded when the production profile is selected
[production.deploy]
memory = 512
```

You can load the configuration for the `dev` context by running the following command:

```sh
cargo lambda deploy --context dev
```

You can also specify the context using the `CARGO_LAMBDA_CONTEXT` environment variable.

## Build configuration

The build configuration is used to configure the build process for the Lambda function. This is the configuration that is used when you run the `cargo lambda build` command.

The build configuration supports the following options:

- `output_format`: The format to produce the compile Lambda into. Acceptable values are `Binary` or `Zip`.
- `lambda_dir`: Directory where the final lambda binaries will be located.
- `arm64`: Shortcut for `--target aarch64-unknown-linux-gnu`. When set to `true`, builds for ARM64 architecture.
- `x86_64`: Shortcut for `--target x86_64-unknown-linux-gnu`. When set to `true`, builds for x86_64 architecture.
- `extension`: Whether the code that you're building is a Lambda Extension. Set to `true` to build as an extension.
- `internal`: Whether an extension is internal or external. Only valid when `extension` is `true`.
- `flatten`: Put a bootstrap file in the root of the lambda directory. Use the name of the compiled binary to choose which file to move.
- `skip_target_check`: Whether to skip the target check. Set to `true` to skip the target check.
- `compiler`: The compiler to use to build the Lambda function.
- `disable_optimizations`: Whether to disable all default release optimizations.
- `include`: Option to add one or more files and directories to include in the output ZIP file (only works with --output-format=zip).
- `quiet`: Whether to disable all log messages.
- `jobs`: The number of parallel jobs to use when building the Lambda function.
- `keep_going`: Whether to continue building the Lambda function even if there are errors.
- `profile`: The profile to use when building the Lambda function.
- `features`: The features to enable when building the Lambda function.
- `all_features`: Whether to enable all features when building the Lambda function.
- `no_default_features`: Whether to disable the `default` feature when building the Lambda function.
- `target`: The target triple to build the Lambda function for.
- `target_dir`: The directory where the build artifacts will be located.
- `message_format`: The format to use for the build messages.
- `verbose`: Whether to enable verbose output.
- `color`: Whether to enable color output.
- `frozen`: Whether to require Cargo.lock and cache are up to date.
- `locked`: Whether to require Cargo.lock is up to date.
- `offline`: Whether to run without accessing the network.
- `override`: Override a configuration value (unstable).
- `config`: Override a configuration value (unstable).
- `unstable_flags`: Unstable (nightly-only) flags to Cargo, see 'cargo -Z help' for details.
- `timings`: Timing output formats (unstable) (comma separated): html, json.
- `manifest_path`: Path to Cargo.toml.
- `release`: Build artifacts in release mode, with optimizations.
- `ignore_rust_version`: Ignore `rust-version` specification in packages.
- `unit_graph`: Output build graph in JSON (unstable).
- `packages`: Package to build (see `cargo help pkgid`).
- `workspace`: Build all packages in the workspace.
- `exclude`: Exclude packages from the build.
- `lib`: Build only this package's library.
- `bin`: Build only the specified binary.
- `bins`: Build all binaries.
- `example`: Build only the specified example.
- `examples`: Build all examples.
- `test`: Build only the specified test target.
- `tests`: Build all tests.
- `bench`: Build only the specified bench target.
- `benches`: Build all benches.
- `all_targets`: Build all targets.

Example configuration:

```toml
[build]
output_format = "zip"
lambda_dir = "dist"
arm64 = true
include = ["README.md"]
quiet = true
```

## Deploy configuration

The deploy configuration is used to configure the deploy process for the Lambda function. This is the configuration that is used when you run the `cargo lambda deploy` command.

The deploy configuration supports the following options:

- `remote_config`: The remote configuration to use for the deploy. It includes the following options:
    - `profile`: The AWS profile to use for authorization.
    - `region`: The AWS region to deploy the Lambda function to.
    - `alias`: The AWS Lambda alias to associate the function to.
    - `retry_attempts`: The number of attempts to try failed operations.
    - `endpoint_url`: The custom endpoint URL to target.
- `enable_function_url`: Whether to enable function URL for this function.
- `disable_function_url`: Whether to disable function URL for this function.
- `memory`: The memory allocated for the function.
- `timeout`: The timeout for the function.
- `tracing`: The tracing mode with X-Ray.
- `role`: The IAM role associated with the function.
- `layer`: The Lambda Layer ARN to associate the deployed function with.
- `tracing`: The tracing mode with X-Ray.
- `role`: The IAM role associated with the function.
- `layer`: The Lambda Layer ARN to associate the deployed function with.
- `runtime`: The Lambda runtime to deploy the function with.
- `description`: A description for the new function version.
- `log_retention`: The retention policy for the function's log group.
- `env_var`: The environment variables to set for the function.
- `env_file`: The environment file to read the environment variables from.
- `vpc`: The VPC configuration to use for the function. It includes the following options:
    - `subnet_ids`: The subnet IDs to associate the deployed function with a VPC.
    - `security_group_ids`: The security group IDs to associate the deployed function.
    - `ipv6_allowed_for_dual_stack`: Whether to allow outbound IPv6 traffic on VPC functions that are connected to dual-stack subnets.
- `lambda_dir`: Directory where the lambda binaries are located.
- `manifest_path`: Path to Cargo.toml.
- `binary_name`: Name of the binary to deploy if it doesn't match the name that you want to deploy it with.
- `binary_path`: Local path of the binary to deploy if it doesn't match the target path generated by cargo-lambda-build.
- `s3_bucket`: The S3 bucket to upload the code to.
- `s3_key`: The name with prefix where the code will be uploaded to in S3.
- `extension`: Whether the code that you're deploying is a Lambda Extension.
- `internal`: Whether an extension is internal or external. Only valid when `extension` is `true`.
- `compatible_runtimes`: Comma separated list with compatible runtimes for the Lambda Extension (--compatible_runtimes=provided.al2,nodejs16.x)
- `output_format`: The format to render the output (text, or json)
- `tag`: Comma separated list of tags to apply to the function or extension (--tag organization=aws,team=lambda).
- `include`: Option to add one or more files and directories to include in the zip file to upload.
- `dry`: Perform all the operations to locate and package the binary to deploy, but don't do the final deploy.
- `name`: Name of the function or extension to deploy.

Example configuration:

```toml
[deploy]
s3_bucket = "my-s3-bucket"
s3_key = "my-s3-key"

[deploy.remote_config]
profile = "my-aws-profile"
region = "us-east-1"
alias = "my-alias"
```

## Watch configuration

The watch configuration is used to configure the watch process for the Lambda function. This is the configuration that is used when you run the `cargo lambda watch` command.

The watch configuration supports the following options:

- `ignore_changes`: Whether to ignore any code changes, and don't reload the function automatically.
- `only_lambda_apis`: Start the Lambda runtime APIs without starting the function. This is useful if you start (and debug) your function in your IDE.
- `invoke_address`: Address where users send invoke requests.
- `invoke_port`: Port where users send invoke requests.
- `invoke_timeout`: Timeout for the invoke requests.
- `print_traces`: Print OpenTelemetry traces after each function invocation.
- `wait`: Wait for the first invocation to compile the function.
- `disable_cors`: Disable the default CORS configuration.
- `timeout`: Timeout for the invoke requests.
- `router`: The router to use for the function.
- `manifest_path`: Path to Cargo.toml.
- `release`: Build artifacts in release mode, with optimizations.
- `ignore_rust_version`: Ignore `rust-version` specification in packages.
- `unit_graph`: Output build graph in JSON (unstable).
- `packages`: Package to run (see `cargo help pkgid`).
- `bin`: Run the specified binary.
- `example`: Run the specified example.
- `args`: Arguments for the binary to run.
- `quiet`: Whether to disable all log messages.
- `jobs`: The number of parallel jobs to use when building the Lambda function.
- `keep_going`: Whether to continue building the Lambda function even if there are errors.
- `profile`: The profile to use when building the Lambda function.
- `features`: The features to enable when building the Lambda function.
- `all_features`: Whether to enable all features when building the Lambda function.
- `no_default_features`: Whether to disable the `default` feature when building the Lambda function.
- `target`: The target triple to build the Lambda function for.
- `target_dir`: The directory where the build artifacts will be located.
- `message_format`: The format to use for the build messages.
- `verbose`: Whether to enable verbose output.
- `color`: Whether to enable color output.
- `frozen`: Whether to require Cargo.lock and cache are up to date.
- `locked`: Whether to require Cargo.lock is up to date.
- `offline`: Whether to run without accessing the network.
- `env_var`: The environment variables to set for the function.
- `env_file`: The environment file to read the environment variables from.
- `tls_cert`: Path to a TLS certificate file.
- `tls_key`: Path to a TLS key file.
- `tls_ca`: Path to a TLS CA file.

Example configuration:

```toml
[watch]
invoke_address = "0.0.0.0"
invoke_port = 8080
features = ["feature1", "feature2"]
```
