# Lambda Extensions

AWS Lambda Extensions are sidecar services that can provide extra capabilities to your functions.

Cargo Lambda can also help you build and deploy Lambda Extensions.

## Step 1: Create a new project

To start a new extension project, use with `new` subcommand with the `--extension` flag:

```sh
cargo lambda new --extension extension-project
```

::: tip
Add the `--logs` flag if your extension in a Logs extension.
:::

## Step 2: Build your extension

Once you're ready to compile your project, use the `build` subcommand with the `--extension` flag:

```sh
cargo lambda build --extension
```

## Step 3: Deploy your extension

Lambda extensions are deployed as layers for functions to consume. Use the `deploy` subcommand with the `--extension` flag to upload the extension layer to AWS. When this command completes, it will print the full ARN to use to attach this extension to a function:

```sh
cargo lambda deploy --extension
```

## Step 4: Associate your extension to a function

If you already have a function project  that you want to attach your extension to, you can use the same `deploy` subcommand to do so. Go to the root directory for your function's project, and add the `--layer-arn` flag with the extension ARN that the previous command printed in the terminal:

```sh
cargo lambda deploy --layer-arn EXTENSION_ARN
```
