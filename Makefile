.PHONY: build build-release build-release-tar build-release-zip check fmt install install-release publish-all run-integration

build:
	cargo build --features=skip-build-banner

build-release:
	cargo build --release --features=skip-build-banner

build-release-tar:
	cd $(target)-$(tag)-bin && \
		chmod +x cargo-lambda && \
		tar czvf cargo-lambda-$(tag).$(target).tar.gz cargo-lambda && \
		shasum -a 256 cargo-lambda-$(tag).$(target).tar.gz > cargo-lambda-$(tag).$(target).tar.gz.sha256 && \
		mv *.tar.gz* .. && cd ..

build-release-zip:
	cd $(target)-$(tag)-bin && \
		zip cargo-lambda-$(tag).$(target).zip cargo-lambda.exe && \
		shasum -a 256 cargo-lambda-$(tag).$(target).zip > cargo-lambda-$(tag).$(target).zip.sha256 && \
		mv *.zip* .. && cd ..

check:
	cargo check
	cargo +nightly udeps

fmt:
	cargo +nightly fmt --all

install:
	cargo install --path crates/cargo-lambda-cli --features=skip-build-banner

install-release:
	cargo install --release --path crates/cargo-lambda-cli --features=skip-build-banner

publish-all:
	cargo publish --package cargo-lambda-interactive
	sleep 10
	cargo publish --package cargo-lambda-remote
	sleep 10
	cargo publish --package cargo-lambda-metadata
	sleep 10
	cargo publish --package cargo-lambda-build
	sleep 10
	cargo publish --package cargo-lambda-deploy
	sleep 10
	cargo publish --package cargo-lambda-invoke
	sleep 10
	cargo publish --package cargo-lambda-new
	sleep 10
	cargo publish --package cargo-lambda-system
	sleep 10
	cargo publish --package cargo-lambda-watch
	sleep 10
	cd crates/cargo-lambda-cli && cargo publish

make docs-dev:
	cd docs && pnpm install && pnpm docs:dev

clippy:
	cargo clippy --workspace --all-features -- -D warnings

clippy-fix:
	cargo clippy --workspace --all-features --allow-dirty --allow-staged --fix
