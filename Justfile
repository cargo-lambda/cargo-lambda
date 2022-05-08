publish-all:
    cargo publish --package cargo-lambda-interactive
    sleep 5
    cargo publish --package cargo-lambda-metadata
    sleep 5
    cargo publish --package cargo-lambda-remote
    sleep 5
    cargo publish --package cargo-lambda-build
    sleep 5
    cargo publish --package cargo-lambda-deploy
    sleep 5
    cargo publish --package cargo-lambda-invoke
    sleep 5
    cargo publish --package cargo-lambda-new
    sleep 5
    cargo publish --package cargo-lambda-watch
    sleep 5
    cd crates/cargo-lambda-cli && cargo publish
