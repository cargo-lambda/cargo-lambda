# Cargo Lambda Deploy

This subcommand uploads functions to AWS Lambda. You can use the same command to create new functions as well as update existent functions code. This command assumes that your AWS account has permissions to call several lambda operations, like `lambda:getFunction`, `lambda:createFunction`, and `lambda:updateFunctionCode`. If you are using layers, you must also add `lambda:GetLayerVersion`. This subcommand also requires an IAM role with privileges in AWS Lambda.

When you call this subcommand, the function binary must have been created with the [Build](/commands/build) subcommand ahead of time. The command will fail if it cannot find the binary file.

This command automatically detects the architecture that the binary was compiled for, so you don't have to specify it.

The example below deploys a function that has already been compiled with the default flags:

```
cargo lambda deploy
```

The following video shows you how to use this subcommand:

<iframe width="560" height="315" src="https://www.youtube.com/embed/ICUSfTorBnI" title="YouTube video player" frameborder="0" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen></iframe>

## Versioning

By default, Cargo Lambda will create a new version of the function when you deploy it. This allows you to keep old versions of the function alive, and you can roll back to a previous version if you need to. Versions are named with a version number by AWS Lambda, this version number cannot be changed. However, you can provide a description for the new version of the function using the `--description` flag. This description will be used as the description for the new version of the function. It's a good idea to provide a description for the new version of the function, so you can easily identify it when you have multiple versions of the function.

For example, you can use the description to associate the deployment with a specific git commit hash:

```
cargo lambda deploy --description $(git rev-parse HEAD)
```

## Working with multiple packages

By default, Cargo Lambda tries to detect the binary that you built before deploying it. This can be challenging if you're working in a workspace with multiple Rust packages. There are multiple ways to provide the information about the package you want to deploy more explicitly in this subcommand.

### Deploying a function with its same package name

If you have multiple packages in a workspace, and you want to deploy one of them with the same name as it's function name, you can provide the name of the package as an argument to the deploy subcommand:

```sh
cargo lambda deploy PACKAGE_NAME
```

### Deploying a function with a different name than it's package name

If you want to deploy a function with a different name than it's package name, you use the first argument to the subcommand as the function name, while using the flag `--binary-name` as the name of the package or binary to deploy:

```sh
cargo lambda deploy --binary-name PACKAGE_NAME FUNCTION_NAME
```

### Deploying a specific binary with a different function name

You can also deploy a specific binary outside your target and assign a function name to it. You set the function name in the first argument to the subcommand, while using the flag `--binary-path` to provide the path to the binary. Keep in mind that the binary's name MUST be `bootstrap`:

```sh
cargo lambda deploy --binary-path PATH_TO_BOOTSTRAP_FILE FUNCTION_NAME
```

## IAM Roles

If you run this command without any flags, Cargo Lambda will try to create an execution role with Lambda's default service role policy `AWSLambdaBasicExecutionRole`.

Use the flag `--iam-role` to provide a specific execution role for your function:

```
cargo lambda deploy --iam-role FULL_ROLE_ARN http-lambda
```

If you're updating the code in a function, you don't need to pass this flag again, unless you want to update the execution role for the function.

## User Profile

You can run this command with a different user profile using the `-p` or `--profile` flags.

## IAM policy configuration

The minimum policy document to deploy functions is described below.

::: warning
This policy is not exhaustive, and it might not be enough for your use case. For example, if you're deploying your function on a VPC, you'll need additional permissions described in the [VPC documentation](https://docs.aws.amazon.com/lambda/latest/dg/vpc.html). If you're deploying an extension, you'll need additional permissions to upload the extension to an S3 bucket.
:::

```json
{
    "Version": "2012-10-17",
    "Statement": [
        {
            "Effect": "Allow",
            "Action": [
                "iam:CreateRole",
                "iam:AttachRolePolicy",
                "iam:UpdateAssumeRolePolicy",
                "iam:PassRole"
            ],
            "Resource": [
                "arn:aws:iam::{AWS:Account}:role/AWSLambdaBasicExecutionRole",
                "arn:aws:iam::{AWS:Account}:role/cargo-lambda-role*"
            ]
        },
        {
            "Effect": "Allow",
            "Action": [
                "lambda:CreateFunction",
                "lambda:UpdateFunctionCode",
                "lambda:GetFunction"
            ],
            "Resource": "arn:aws:lambda::{AWS:Account}:function:{function-name}"
        }
    ]
}
```

Make sure to replace the items in curly braces (`{}`) with the appropriate values.

## Function URLs

This subcommand can enable Lambda function URLs for your lambda. Use the flag `--enable-function-url` when you deploy your function, and when the operation completes, the command will print the function URL in the terminal.

Note that you would need to add the following IAM Actions to use this flag:
- `lambda:GetFunctionUrlConfig`
- `lambda:CreateFunctionUrlConfig`
- `lambda:AddPermission`

::: warning
This flag always configures the function URL without any kind of authorization. Don't use it if you'd like to keep the URL secure.
:::

The example below shows how to enable the function URL for a function during deployment:

```
cargo lambda deploy --iam-role FULL_ROLE_ARN --enable-function-url http-lambda
```

You can use the flag `--disable-function-url` if you want to disable the function URL.

## Environment variables

You can use the flag `--env-vars` to add environment variables to a function. This flag supports a comma separated list of values:

```
cargo lambda deploy \
  --env-vars FOO=BAR,BAZ=QUX \
  http-lambda
```

The flag `--env-var` allows you to pass several variables in the command line with the format `KEY=VALUE`. This flag overrides the previous one, and cannot be combined.

```
cargo lambda deploy \
  --env-var FOO=BAR --env-var BAZ=QUX \
  http-lambda
```

The flag `--env-file` will read the variables from a file and add them to the function during the deploy. Each variable in the file must be in a new line with the same `KEY=VALUE` format:

```
cargo lambda deploy --env-file .env http-lambda
```

### Merging environment variables

By default, when you deploy a function with environment variables, Cargo Lambda will **overwrite** all existing environment variables on AWS Lambda with the ones specified in your configuration. This ensures that your configuration is the single source of truth.

However, in some scenarios (such as hybrid workflows where infrastructure tools like AWS CDK manage some variables and Cargo Lambda manages others), you may want to preserve existing environment variables and only update or add new ones. You can use the `--merge-env` flag to enable this behavior:

```
cargo lambda deploy --merge-env --env-var NEW_VAR=VALUE http-lambda
```

When `--merge-env` is enabled:
- Existing environment variables on the Lambda function are preserved
- Variables specified in your configuration (via `--env-var`, `--env-vars`, or `--env-file`) are added or updated
- If a variable exists both remotely and in your configuration, the local value takes precedence

**Example:**

If your Lambda function currently has these environment variables:
```
CDK_MANAGED_VAR=value1
INFRA_VAR=value2
```

And you deploy with:
```
cargo lambda deploy --merge-env --env-var APP_VAR=value3 http-lambda
```

The final environment variables will be:
```
CDK_MANAGED_VAR=value1  (preserved)
INFRA_VAR=value2         (preserved)
APP_VAR=value3           (added)
```

::: warning
Without the `--merge-env` flag, the default behavior would replace all environment variables, leaving only `APP_VAR=value3`.
:::

## Resource tagging

You can use the flag `--tags` to add resource tags to a function or layer. This flag supports a comma separated list of values. If the function is deployed via S3, the tags are also applied to the S3 object:

```
cargo lambda deploy \
  --tags organization=aws,team=lambda \
  http-lambda
```

You can also use the flag `--tag` to add one or multiple tags separated by flags. This flag overrides the previous one, and cannot be combined.

```
cargo lambda deploy \
  --tag organization=aws \
  --tag team=lambda \
  http-lambda
```

## Extensions

cargo-lambda can deploy Lambda Extensions built in Rust by adding the `--extension` flag to the `deploy` command. This command requires you to build the extension first with the same `--extension` flag in the `build` command:

```
cargo lambda build --release --extension
cargo lambda deploy --extension
```

### Internal extensions

To deploy an [internal extension](https://docs.aws.amazon.com/lambda/latest/dg/lambda-extensions.html), add the `--internal` flag to the deploy command:

```
cargo lambda build --release --extension
cargo lambda deploy --extension --internal
```

## Deploy configuration in Cargo's Metadata

You can keep some deploy configuration options in your project's `Cargo.toml` file. This give you a more "configuration as code" approach since you can store that configuration alongside your project. The following example shows the options that you can specify in the metadata, all of them are optional:

```toml
[package.metadata.lambda.deploy]
memory = 512                    # Function's memory
timeout = 60                    # Function's execution timeout
tracing = "active"              # Tracing mode
role = "role-full-arn"          # Function's execution role
env_file = ".env.production"    # File to load environment variables from
env = { "VAR1" = "VAL1" }       # Additional environment variables
merge_env = true                # Merge environment variables with existing ones (default: false)
layers = [ "layer-full-arn" ]   # List of layers to deploy with your function
tags = { "team" = "lambda" }    # List of AWS resource tags for this function
s3_bucket = "deploy-bucket"     # S3 bucket to upload the Lambda function to
s3_key = "prefix/bootstrap.zip" # S3 object key to upload the Lambda function to
include = [ "README.md" ]       # Extra list of files to add to the zip bundle
```

## Deploying to S3

AWS Lambda has a 50MB limit for Zip file direct uploads. If the Zip file that you're trying to deploy is larger than 50MB, you can upload it to S3 using the `--s3-bucket` option. This option takes the name of a bucket in your account where the Zip file will be stored. To use this option, you need `Post` or `Put` access to S3 in your deployment credentials:

```
cargo lambda deploy --s3-bucket bucket-name
```

If you want to change the location where the S3 object is created, you can use the `--s3-key` flag to set the S3 object key for the zip file that's uploaded to S3:

```
cargo lambda deploy --s3-bucket bucket-name --s3-key prefix/deploy/my-bootstrap-file.zip
```

## Adding extra files to the zip file

In some situations, you might want to add extra files inside the zip file uploaded to AWS. You can use the option `--include` to add extra files or directories to the zip file. For example, if you have a directory with configuration files, you can add it to the zip file using the command below:

```
cargo lambda deploy --include config
```

### Modifying the included paths in the zip file

If you want to include files from a parent directory, not inside your project structure, you'll find the challenge that `--include` will use the same path inside the zip file. For example, `../config/data.json` will be named exactly like that inside the zip file, and it will be extracted in a relative location. If you want to change the name of the file or directory when it's included in the zip file, you can use the special syntax `FINAL_NAME:PATH`.

For example, the following command will include all files inside the relative path `../../config` in the zip file, but the base directory will be called `config`:

```
cargo lambda deploy --include config:../../config
```

If we were to inspect the zip file to deploy with `unzip -l`, we'd see a structure like this one:

```
Archive:  bootstrap.zip
  Length      Date    Time    Name
---------  ---------- -----   ----
        0  2024-09-14 18:25   config/
        8  2024-09-14 18:22   config/production.json
        8  2024-09-14 18:22   config/database.json
  3037216  2024-09-14 18:24   bootstrap
---------                     -------
  3037232                     4 files
```

## CloudWatch Log Retention

You can set the CloudWatch Log Retention for your function using the `--log-retention` flag. This flag takes an integer value that represents the number of days to keep the logs. If the log group does not exist, it will be created. If the log group already exists, the retention policy will be updated.

```
cargo lambda deploy --log-retention 30 http-lambda
```

## Other options

Use the `--help` flag to see other options to configure the function's deployment.

## State management

The deploy command doesn't use any kind of state management. If you require state management, you should use tools like [SAM Cli](https://github.com/aws/aws-sam-cli) or the [AWS CDK](https://github.com/aws/aws-cdk).

If you modify a flag and run the deploy command twice for the same function, the change will be updated in the function's configuration in AWS Lambda.
