# Installation

Cargo Lambda uses [Zig](https://ziglang.org) to link your functions for Linux systems. The installers below also install Zig for you if it's not in your system.

## On Linux and MacOS

With [Homebrew](https://brew.sh/):

```sh
brew install cargo-lambda/tap/cargo-lambda
```

With [Curl](https://curl.se/):

```sh
curl -fsSL https://cargo-lambda.info/install.sh | sh
```

With [PyPI](https://pypi.org/):

```sh
pip3 install cargo-lambda
```

With [Nix](https://nixos.org/manual/nix/stable/introduction.html):

```sh
nix-env -iA nixpkgs.cargo-lambda
```

## On Windows

With [WinGet](https://learn.microsoft.com/en-us/windows/package-manager/):

```sh
winget install CargoLambda.CargoLambda
```

With [Scoop](https://scoop.sh/):

```sh
scoop bucket add cargo-lambda https://github.com/cargo-lambda/scoop-cargo-lambda
scoop install cargo-lambda/cargo-lambda
```

With [PowerShell](https://learn.microsoft.com/en-us/powershell/):

```powershell
irm https://cargo-lambda.info/install.ps1 | iex
```

## With Docker

You can run Cargo Lambda directly from our official Docker image:

```sh
docker pull ghcr.io/cargo-lambda/cargo-lambda
```

## Binary releases

You can also download any Cargo Lambda binary from the [Release page](https://github.com/cargo-lambda/cargo-lambda/releases).

::: warning
When you download a binary directly, [Zig](https://ziglang.org) won't be installed for you. You can run `cargo lambda system --install-zig` to get a list of possible installers for your system.
:::

You can use a tool like [Cargo Binstall](https://github.com/cargo-bins/cargo-binstall) to automatically download a binary package from GitHub:

```sh
cargo binstall cargo-lambda
```

## Building from source

You can install Cargo Lambda on your host machine with from its source code repository. This method is not recommended because the binary will be compiled in your system, which we cannot always guarantee. Using a package manager, or pre-built binaries is always more encouraged to have a functional service and avoid installation issues. Cargo Lambda does not publish its source in crates.io anymore because we cannot guarantee the reproducibility of the build when using `cargo install`.

```sh
git clone https://github.com/cargo-lambda/cargo-lambda && \
  cd cargo-lambda && \
  make install-release
```

::: warning
Cargo Install compiles the binary in your system, which usually takes more than 10 minutes. This method doesn't install [Zig](https://ziglang.org) either, which is a requirement if you want to cross compile packages from macOS or Windows to Lambda Linux environments.
:::
