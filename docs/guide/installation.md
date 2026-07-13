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

With [Nix Flakes](https://nixos.wiki/wiki/flakes):
```nix
{
  description = "Rust development environment";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay.url = "github:oxalica/rust-overlay";

  };

   outputs = { self, nixpkgs, flake-utils, rust-overlay }:
    flake-utils.lib.eachDefaultSystem (system:
    let
      overlays = [ (import rust-overlay) ];
      pkgs = import nixpkgs {
        inherit system overlays;
        config = {
          packageOverrides = pkgs: {
          rust-bin.stable.latest.default = pkgs.rust-bin.stable.latest.default.override {
              targets = [
                "aarch64-unknown-linux-gnu"
                "aarch64-apple-darwin"
                "x86_64-unknown-linux-gnu"
              ];
          };
          };
        };
      };
      lib = import lib {
        inherit lib;
      };
      stdenv = import stdenv {
        inherit stdenv;
      };
    in
    {
      devShells.default = pkgs.mkShell {

          nativeBuildInputs = with pkgs; [
            pkg-config
            cargo
            cargo-lambda
            glibc
          ];

        buildInputs = with pkgs;[
            #qemu uncomment if crossplatform builds
            openssl
            rustfmt
            clippy
            rust-analyzer
        ];
        extraPackages = with pkgs;[
          rust-bin.stable.latest.default
        ];
        shellHook = ''
          export LD_LIBRARY_PATH=$NIX_LD_LIBRARY_PATH
        '';
        env = {
          # uncomment if doing crossplatform builds CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER = "${pkgs.stdenv.cc.targetPrefix}cc";
          # uncomment if doing crossplatform builds  CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER = "qemu-aarch64";
          # for more information on crossplatform builds  https://github.com/oxalica/rust-overlay/blob/master/docs/cross_compilation.md
          # example flake for crossplatform build https://github.com/oxalica/rust-overlay/blob/master/examples/cross-aarch64/flake.nix
          AWS_ACCESS_KEY_ID = "XXXXXXXXXXXXXXXXX"; # this is for aws-cdk and cli
          AWS_SECRET_ACCESS_KEY = "XXXXXXXXXXXXXXXX"; # this is for aws-cdk and cli
        };
      };
    }
  );
}

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

You can install Cargo Lambda on your host machine from its source code repository. This method is not recommended because the binary will be compiled in your system, which we cannot always guarantee. Using a package manager, or pre-built binaries is always more encouraged to have a functional service and avoid installation issues. Cargo Lambda does not publish its source in crates.io anymore because we cannot guarantee the reproducibility of the build when using `cargo install`.

```sh
git clone https://github.com/cargo-lambda/cargo-lambda && \
  cd cargo-lambda && \
  make install-release
```

::: warning
Cargo Install compiles the binary in your system, which usually takes more than 10 minutes. This method doesn't install [Zig](https://ziglang.org) either, which is a requirement if you want to cross compile packages from macOS or Windows to Lambda Linux environments.
:::
