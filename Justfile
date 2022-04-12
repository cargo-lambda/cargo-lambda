publish-all:
    cargo publish --package crates/cargo-lambda-interactive
    cargo publish --package crates/cargo-lambda-metadata
    cargo publish --package crates/cargo-lambda-build
    cargo publish --package crates/cargo-lambda-invoke
    cargo publish --package crates/cargo-lambda-new
    cargo publish --package crates/cargo-lambda-watch
    cargo publish --package crates/cargo-lambda-cli