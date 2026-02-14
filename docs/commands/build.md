# Cargo Lambda Build

Within a Rust project that includes a `Cargo.toml` file, run the `cargo lambda build` command to natively cross-compile your Lambda functions in the project to Linux. The resulting artifacts such as binaries or zips, will be placed in the `target/lambda` directory. This is an example of the output produced by this command:

```
❯ tree target/lambda
target/lambda
├── delete-product
│   └── bootstrap
├── dynamodb-streams
│   └── bootstrap
├── get-product
│   └── bootstrap
├── get-products
│   └── bootstrap
└── put-product
    └── bootstrap

5 directories, 5 files
```

The following video shows you how to use this subcommand:

<iframe width="560" height="315" src="https://www.youtube.com/embed/ICUSfTorBnI" title="YouTube video player" frameborder="0" allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture; web-share" allowfullscreen></iframe>

If you want to learn more abour cross-compiling Rust Lambda functions, checkout the [Cross Compiling Guide](/guide/cross-compiling).

## Output Format

By default, cargo-lambda produces a binary artifact for each Lambda functions in the project.
However, you can configure cargo-lambda to produce a ready to upload zip artifact.

The `--output-format` parameter controls the output format, the two current options are `zip` and `binary` with `binary` being the default.

Example usage to create a zip.

```
cargo lambda build --output-format zip
```

## Architectures

By default, cargo-lambda compiles the code for Linux X86-64 architectures, you can compile for Linux ARM architectures by providing the right target:

```
cargo lambda build --target aarch64-unknown-linux-gnu
```

ℹ️ Starting in version 0.6.2, you can use the shortcut `--arm64` to compile your functions for Linux ARM architectures:

```
cargo lambda build --arm64
```

## Compilation Profiles

By default, cargo-lambda compiles the code in `debug` mode. If you want to change the profile to compile in `release` mode, you can provide the right flag.

```
cargo lambda build --release
```

When you compile your code in release mode, cargo-lambda will apply some optimizations to make the binary size smaller. Check out the [Release Optimizations](/guide/release-optimizations) guide for more details.

## Extensions

cargo-lambda can also build Lambda Extensions written in Rust. If you want to build a extension, use the flag `--extension` to put the output under `target/lambda/extensions`, so you don't mix extensions and functions.

```
cargo lambda build --release --extension
```

If you want to create a zip file with the structure that AWS Lambda expects to find extensions in, add the `--output-format` flag to the previous command, and cargo-lambda will zip the extensions directory with your extension inside.

```
cargo lambda build --release --extension --output-format zip
```

If you're building an internal extension, add the `--internal` flag to the build command. You can skip this flag if you use `cargo lambda deploy` to deploy the extension later.

```
cargo lambda build --release --extension --internal --output-format zip
```

## Compiler backends

Cargo Lambda has an internal abstraction to work with different ways to compile functions.

The default compiler is `cargo-zigbuild`. This compiler uses [Zig](https://ziglang.org) to cross compile any Rust project to a Linux target on your own OS, without the need to a virtual machine or a Linux container. If Zig is not installed in your host machine, the first time that your run Cargo Lambda, it will guide you through some installation options. If you run Cargo Lambda in a non-interactive shell, the build process will fail until you install that dependency.

Cargo Lambda also supports building Rust projects without Zig as the target linker. This compiler is identifed as just `cargo`. A disadvantage of this is that it's up to you to guarantee that the binary works on Linux. An advantage is that if you always build functions on Linux, you don't need to install Zig to use Cargo Lambda.

Cargo Lambda supports building Rust projects with [cross](https://crates.io/crates/cross) as well. Read the [Cross Compiling reference](/guide/cross-compiling.html#cross-compiling-with-cross) to learn more abour using cross as the Lambda compiler.

### Adding Zig to PATH on Windows/WSL

If you installed Zig using Pip3 and still encounter issues with Cargo Lambda not finding Zig, it might be because the Zig binary is not in your system’s `$PATH`.

To resolve this, you need to manually add the Zig installation path to your environment variables.

#### Steps:
1. First, locate where Zig is installed. If you installed it via Pip3, the directory might look like this:
   ```
   /c/Users/your-username/appdata/local/continuum/anaconda3/lib/site-packages/ziglang/
   ```
   or
   ```
   /home/your-username/.local/lib/python3.9/site-packages/ziglang/
   ```

2. To ensure the path persists across terminal sessions, append the following command to your `~/.bashrc` (or `~/.zshrc` for Zsh users):

   ```bash
   echo 'export PATH="/path/to/zig:$PATH"' >> ~/.bashrc
   ```

3. Apply the changes by running:
   ```bash
   source ~/.bashrc
   ```

4. Finally, verify that Zig is in the PATH by running:
   ```bash
   which zig
   ```

After doing this, the `cargo lambda build` command should now be able to find Zig and compile your project correctly.

### Switching compilers

To switch compilers, you can use the flag `--compiler` with the name of the compiler to use when you run `cargo lambda build`. For example:

```
cargo lambda build --compiler cargo
```

You can also use an environment variable to select the compiler:

```
export CARGO_LAMBDA_COMPILER=cargo
cargo lambda build
```

Additionally, you can also add this option in your project's `Cargo.toml` metadata. Add the snippet below if you want to use Cargo without Zig as linker in your project:

```
[package.metadata.lambda.build.compiler]
type = "cargo"
```

### Additional compilers

The concept of compilers on Cargo Lambda is an abstraction on top of different shell commands. If you want to add an additional compiler, you need to implement [Compiler](https://github.com/cargo-lambda/cargo-lambda/blob/main/crates/cargo-lambda-build/src/compiler/mod.rs#L14) trait. The command to execute needs to follow Rust compilations' convenctions, for example, if the user wants to build an Arm64 binary with the `release` profile, Cargo Lambda will expect that the resulting binary is in `target/aarch64-unknown-linux-gnu/release/`.

## Environment Variables

You can pass explicit environment variables to the cargo build process if you want to keep them all together under the same configuration.

### Using command line flags

Use `--env-var` to pass environment variables directly:

```bash
cargo lambda build --env-var LEPTOS_OUTPUT_NAME=myapp
```

You can pass multiple environment variables by repeating the flag or using comma separation:

```bash
cargo lambda build --env-var KEY1=VALUE1 --env-var KEY2=VALUE2
# or
cargo lambda build --env-var KEY1=VALUE1,KEY2=VALUE2
```

### Using environment files

Use `--env-file` to load environment variables from a file:

```bash
cargo lambda build --env-file .env.build
```

The file should contain environment variables in `KEY=VALUE` format, one per line:

```
LEPTOS_OUTPUT_NAME=myapp
RUST_LOG=debug
```

### Environment variable precedence

When environment variables are specified in multiple ways, they are merged in the following order (later sources override earlier ones):

1. Variables from `--env-file`
2. Variables from `--env-var`

This allows you to set base configuration in a file and override specific values via command line flags.

## Build configuration in Cargo's Metadata

You can keep some build configuration options in your project's `Cargo.toml` file. This give you a more "configuration as code" approach since you can store that configuration alongside your project. The following example shows the options that you can specify in the metadata, all of them are optional:

```toml
[package.metadata.lambda.build]
include = [ "README.md" ]      # Extra list of files to add to the zip bundle
env_var = [                    # Environment variables to set during build
    "LEPTOS_OUTPUT_NAME=myapp",
    "RUST_LOG=debug"
]
env_file = ".env.build"        # Path to environment file
```

Environment variables specified via command line flags (`--env-var` and `--env-file`) will override those specified in the metadata.

## Output Directory Structure

### Lambda Directory (`--lambda-dir`)

By default, cargo-lambda places compiled binaries in `target/lambda`. You can customize this location using the `--lambda-dir` flag.

**Important:** The `--lambda-dir` flag specifies a *base directory*. Each binary will be placed in a subdirectory named after the binary within this base directory.

**Example:**
```bash
cargo lambda build --bin foo --lambda-dir a/b/c
```

**Result:**
```
a/b/c/foo/bootstrap
```

### Flattening Directory Structure (`--flatten`)

If you want the binary to be placed directly in the specified directory without the additional subdirectory, use the `--flatten` flag:

```bash
cargo lambda build --bin foo --lambda-dir a/b/c --flatten foo
```

**Result:**
```
a/b/c/bootstrap
```

The `--flatten` flag requires you to specify which binary to flatten by providing the binary name.

## Adding extra files to the zip file

In some situations, you might want to add extra files inside the zip file built. You can use the option `--include` to add extra files or directories to the zip file. For example, if you have a directory with configuration files, you can add it to the zip file using the command below:

```
cargo lambda build --output-format zip --include config
```
