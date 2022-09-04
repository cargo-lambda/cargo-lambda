.PHONY: build build-release-tar build-release-zip check fmt install-local publish-all run-integration

build:
	cargo build

build-release-tar:
	cd $(target)-$(tag)-bin && \
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

install-local:
	cargo install --path crates/cargo-lambda-cli

publish-all:
	cargo publish --package cargo-lambda-interactive
	sleep 10
	cargo publish --package cargo-lambda-metadata
	sleep 10
	cargo publish --package cargo-lambda-remote
	sleep 10
	cargo publish --package cargo-lambda-build
	sleep 10
	cargo publish --package cargo-lambda-deploy
	sleep 10
	cargo publish --package cargo-lambda-invoke
	sleep 10
	cargo publish --package cargo-lambda-new
	sleep 10
	cargo publish --package cargo-lambda-watch
	sleep 10
	cd crates/cargo-lambda-cli && cargo publish

run-integration: build
	ruby test/integration.rb