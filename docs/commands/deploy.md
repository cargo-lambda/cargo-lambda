# cargo lambda deploy

This subcommand uploads functions to AWS Lambda. You can use the same command to create new functions as well as update existent functions code. This command assumes that your AWS account has permissions to call several lambda operations, like `lambda:getFunction`, `lambda:createFunction`, and `lambda:updateFunctionCode`. This subcommand also requires an IAM role with privileges in AWS Lambda.

When you call this subcommand, the function binary must have been created with the [Build](/commands/build) subcommand ahead of time. The command will fail if it cannot find the binary file.

This command automatically detects the architecture that the binary was compiled for, so you don't have to specify it.

The example below deploys a function that has already been compiled with the default flags:

```
cargo lambda deploy
```

## IAM Roles

If you run this command without any flags, Cargo Lambda will try to create an execution role with Lambda's default service role policy `AWSLambdaBasicExecutionRole`.

Use the flag `--iam-role` to provide a specific execution role for your function:

```
cargo lambda deploy --iam-role FULL_ROLE_ARN http-lambda
```

If you're updating the code in a function, you don't need to pass this flag again, unless you want to update the execution role for the function.

## Function URLs

This subcommand can enable Lambda function URLs for your lambda. Use the flag `--enable-function-url` when you deploy your function, and when the operation completes, the command will print the function URL in the terminal.

::: warning
This flag always configures the function URL without any kind of authorization. Don't use it if you'd like to keep the URL secure.
:::

The example below shows how to enable the function URL for a function during deployment:

```
cargo lambda deploy --iam-role FULL_ROLE_ARN --enable-function-url http-lambda
```

You can use the flag `--disable-function-url` if you want to disable the function URL.

## Environment variables

You can add environment variables to a function during deployment with the flags `--env-var` and `--env-file`.

The flag `--env-var` allows you to pass several variables in the command like with the format `KEY=VALUE`:

```
cargo lambda deploy \
  --env-var FOO=BAR --env-var BAZ=QUX \
  http-lambda
```

The flag `--env-file` will read the variables from a file and add them to the function during the deploy. Each variable in the file must be in a new line with the same `KEY=VALUE` format:

```
cargo lambda deploy --env-file .env http-lambda
```

## Extensions

cargo-lambda can deploy Lambda Extensions built in Rust by adding the `--extension` flag to the `deploy` command. This command requires you to build the extension first with the same `--extension` flag in the `build` command:

```
cargo lambda build --release --extension
cargo lambda deploy --extension
```

## Other options

Use the `--help` flag to see other options to configure the function's deployment.

## State management

The deploy command doesn't use any kind of state management. If you require state management, you should use tools like [SAM Cli](https://github.com/aws/aws-sam-cli) or the [AWS CDK](https://github.com/aws/aws-cdk).

If you modify a flag and run the deploy command twice for the same function, the change will be updated in the function's configuration in AWS Lambda.
