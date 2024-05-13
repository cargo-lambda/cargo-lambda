Cargo Lambda automatically applies some optimizations to your binaries when you build your projects with the flag `--release`. This page describes these optimizations, how to change them, and how to disable them.

## Compile time Optimizations

### Strip symbols

When Rust compiles your code, it stores a table with symbolic references to instructions in your code. These references are usually called "Symbols". They are used mostly for debugging purposes. When you attach a debugger to your program, the debugger uses the symbols to translate the instructions that the program is executing to code that you can understand. These symbols are the biggest contributor to increase the size of binary programs. They can also slow startup time.

Since you cannot attach a tradditional debugger to AWS Lambda, these symbols are completely unnecesary. Cargo Lambda removes them by default in release mode.

### Link Time Optimization (LTO)

The last step when Rust compiles your code is known as "linking". In this step, LLVM can analyze your whole program to produce better binary code.

Cargo Lambda uses `lto="thin"` to compile your functions. This optimization achieve mostly optimal code without sacrificing speed at link time.

### Parallel Code Generation Units

By default, Rust compiles your code in parallel. The level of parallelism is indicated by a flag called `codegen-units`. The higher the value in this flag, the higher the level of parallelism that Rust uses to compile your code. The drawback of a high number of code generation units, is that Rust cannot optimize your code as much because each compilation unit acts independently.

Cargo Lambda uses `codegen-units=1` to compile your functions. This means that Rust won't parallelize your compilation. This can increase compilation times, but the result is a much more optimized binary.

### Panic behavior

When a program panics, Rust tries to read all the information in the memory stack to present as much information as possible about how the panic ocurred. This behavior is known as unwinding. This makes the compiled binary bigger because Rust needs to add this behavior to the instructions. When you run in AWS Lambda, unwinding is not a useful mechanism to collect error information.

Cargo Lambda uses `panic=abort` to compile your functions. This option removes the unwinding behavior from your binary, making it smaller.

## Changing compile time optimizations

If you want to change any of the options that Cargo Lambda sets by default, you can set them in your `Cargo.toml` under the `[profile.release]` section. This is an example of profile with all the options modified:

```toml
[profile.release]
strip = false
lto = "fat"
codegen-units = 16
panic = "unwind"
```

Setting `debug = true` in the release profile will also preserve all the debugging symbols in the release binary:

```toml
[profile.release]
debug = true
```

If you want to learn more about the possible values for these options, check out Cargo's reference about [Profile settings](https://doc.rust-lang.org/cargo/reference/profiles.html#profile-settings).

## Rutime CPU optimizations

Cargo Lambda also optimizes the resulting binaries for specific CPU instruction sets.

AWS Lambda uses the Neoverse N1 core for ARM architectures, and the Haswell code for X86-64 architectures. When you compile your code with Cargo Lambda, the right core is added to the `target-cpu` flag using Cargo's `build.rustflags` configuration option.

If you want to provide other rustflags options in the `.cargo/config.toml` file in your project you need to ensure the value is an array of options. That will ensure that both options, your flags and the `target-cpu` flag, are merged correctly.

This is an example of a valid `rustflags` option in the `.cargo/config.toml` file:

```toml
[build]
rustflags = ["--cfg", "tracing_unstable"]
```

This is an example of an _invalid_ `rustflags` option in the `.cargo/config.toml` file:

```toml
[build]
rustflags = "--cfg tracing_unstable"
```

## Disable all release optimizations

If you want to disable all of these optimizations and provide your own, you can pass the flag `--disable-optimizations` to the `cargo lambda build` command:

```shell
cargo lambda build --release --disable-optimizations
```
