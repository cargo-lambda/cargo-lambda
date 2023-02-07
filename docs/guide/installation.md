# Installation

Cargo Lambda uses [Zig](https://ziglang.org) to link your functions for Linux systems. The installers below also install Zig for you if it's not in your system.

## With Homebrew (MacOS and Linux)

You can use [Homebrew](https://brew.sh/) to install Cargo Lambda on MacOS and Linux. Run the following commands on your terminal to add our tap, and install it:

```sh
brew tap cargo-lambda/cargo-lambda
brew install cargo-lambda
```

## With Scoop (Windows)

You can use [Scoop](https://scoop.sh/) to install Cargo Lambda on Windows. Run the following commands to add our bucket, and install it:

```sh
scoop bucket add cargo-lambda https://github.com/cargo-lambda/scoop-cargo-lambda
scoop install cargo-lambda/cargo-lambda
```

## With PyPI

You can also use [PyPI](https://pypi.org/) to install Cargo Lambda on any system that has Python 3 installed:

```sh
pip3 install cargo-lambda
```

## With Docker

You can run Cargo Lambda directly from our official Docker image:

```sh
docker pull calavera/cargo-lambda
```

## With Nix

You can also use [Nix](https://nixos.org/manual/nix/stable/introduction.html) to install Cargo Lambda on any system that supports it:

```sh
nix-env -iA nixpkgs.cargo-lambda
```

## Binary releases

You can also download any Cargo Lambda binary from the [Release page](https://github.com/cargo-lambda/cargo-lambda/releases).

::: warning
When you download a binary directly, [Zig](https://ziglang.org) won't be installed for you.
:::

## Building from source

You can install cargo-lambda on your host machine with Cargo itself. This method is not recommended because the binary will be compiled in your system, which we cannot always guarantee. Using a package manager, or pre-built binaries is always more encouraged to have a functional service and avoid installation issues.

```sh
cargo install --locked cargo-lambda
```

::: warning
cargo-install compiles the binary in your system, which usually takes more than 10 minutes. This method doesn't install [Zig](https://ziglang.org) either, which is a requirement if you want to cross compile packages from MacOS or Windows to Lambda Linux environments.
:::
