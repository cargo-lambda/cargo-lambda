# Cargo Lambda System

The `system` command checks the current installation of Zig and shows its location. Normally, if Zig is not installed 
and cross-compilation is used, Cargo Lambda will prompt to install Zig. The `system` command can be used to
separately install Zig.

To show the current installation of Zig, run the `system` command:

```sh
cargo lambda system
```

## Setup

To install Zig, run the `system` command with the `--setup` flag:

```sh
cargo lambda system --setup
```