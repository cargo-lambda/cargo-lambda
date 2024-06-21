# Cross Compiling with Cargo Lambda

AWS Lambda functions run on Linux sandboxes. These sandboxes only include the bare minimum functionality for Rust binaries to work. It's important to understand that if your function depends on native libraries, like `libpq` for example, it's unlikely that it'll work on AWS Lambda out of the box.

Cargo Lambda compiles your code for Linux targets using several techniques, regardless of whether you work on a Linux, Windows, or macOS machine. Cargo Lambda also compiles for ARM64 and X86-64 architectures, regardless of your host's architecture.

## Cross Compiling with the Zig toolchain

By default, Cargo Lambda uses the [Zig toolchain](https://crates.io/crates/cargo-zigbuild) to cross compile your code. This is the most convenient cross compilation mechanism because it comes built in, and it works for the majority of use cases. Any pure Rust Lambda function should compile correctly with this toolchain.

## Cross Compiling with Cross

Cargo Lambda also supports [cross](https://crates.io/crates/cross) as the compiler. Cross compiles your code inside Linux containers. If you want to use it with Cargo Lambda, you'll have to install it manually in your system, as well as installing [Docker](https://www.docker.com/). All builds with Cross happen inside Docker, so they are slower, but it preserves Cargo Lambda's optimizations and conventions.

Once you've installed the dependencies, you can use cross by setting the `--compiler` option when you build your function:

```
cargo lambda build --compiler cross --release
```

## Cargo Lambda without cross compilation

If you work on Linux, you might not need any cross compiling toolchain. You can still take advantage of Cargo Lambda's optimizations and conventions, and build directly with Cargo. You can tell Cargo Lambda to not cross compile your code by settings the `--compiler` option to `cargo` when you build your function:

```
cargo lambda build --compiler cargo --release
```

## Known cross compilation issues

As mentioned earlier, AWS Lambda uses Linux sandboxes to run your functions. Those sandboxes use Amazon Linux 2 as the operating system. By default, sandboxes only include the necessary libraries for the OS to work. `*-sys` libraries are not guaranteed to work unless they are completely linked to your binary, or you provide the native dependencies in some other way.

This is a list of non-exhaustive problems that you might bump into if you try to build your Rust application to work on AWS Lambda:

- `reqwest` uses OpenSSL as TLS backend by default. If you want to use `reqwest` in your application, you can enable the `native-tls-vendored` or the `rustls` features to include the TLS backend in your application.

- `ring` and any other crates that depend on `cc-rs` have compile-time requirements. Look at [their documentation](https://docs.rs/cc/latest/cc/#compile-time-requirements) to see those requirements depending on your platform.

- `diesel` uses native dependencies to connect to Postgres and MySQL. Use [diesel-async](https://crates.io/crates/diesel-async) instead to have a better integration with the Rust Runtime for Lambda. See the [example in the runtime's repository](https://github.com/awslabs/aws-lambda-rust-runtime/commit/cd0a19cbceb0d340299b25b7957be0e7be85bf73).
