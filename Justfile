publish-all:
    cargo publish --package cargo-lambda-interactive
    cargo publish --package cargo-lambda-metadata
    cargo publish --package cargo-lambda-build
    cargo publish --package cargo-lambda-invoke
    cargo publish --package cargo-lambda-new
    cargo publish --package cargo-lambda-watch
    cargo publish --package cargo-lambda-cli
