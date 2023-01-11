# cargo lambda init

The `init` command initializes new Rust packages with a basic skeleton to help you start writing AWS Lambda functions with Rust. This command will create this package inside the directory where it's invoked. Run `cargo lambda init` to generate your new package. This command will preserve any files already present in the current directory, it will only add new files.

The difference between this command and `new` is where the package is created.

By default, the `init` command uses the name of the directory as the Rust package's name. Use the flag `--name` to specify the name of the package to create:

```sh
cargo lambda init --name init-project
```

This command supports all options described for the [`new`](/commands/new) subcommand. Read the documentation for that command if you want to learn how to create extension packages, or extend templates.
