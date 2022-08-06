# Introduction

Cargo Lambda provides several subcommands for different tasks:

The [new](/commands/new) subcommand creates a basic Rust package from a well defined template to help you start writing AWS Lambda functions in Rust.

The [build](/commands/build) subcommand compiles AWS Lambda functions natively and produces artifacts which you can then [upload to AWS Lambda](/commands/deploy) or use with other echosystem tools, like [SAM Cli](https://github.com/aws/aws-sam-cli) or the [AWS CDK](https://github.com/aws/aws-cdk).

The [watch](/commands/watch) subcommand boots a development server that emulates interations with the AWS Lambda control plane. This subcommand also reloads your Rust code as you work on it.

The [invoke](/commands/invoke) subcommand sends requests to the control plane emulator to test and debug interactions with your Lambda functions. This command can also be used to send requests to remote functions once deployed on AWS Lambda.

The [deploy](/commands/deploy) subcommand uploads functions to AWS Lambda. You can use the same command to create new functions as well as update existent functions code.
