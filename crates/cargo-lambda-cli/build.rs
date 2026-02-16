fn main() {
    print_banner();

    let git_commit = build_data::get_git_commit_short().unwrap_or_else(|_| git_sha_from_env());
    let git_dirty = build_data::get_git_dirty().unwrap_or_default();
    let build_date =
        build_data::format_date(build_data::now()).unwrap_or_else(|_| String::from("unknown date"));

    let build_info = if !git_commit.is_empty() {
        let git_info = if git_dirty {
            format!("{git_commit}-dirty")
        } else {
            git_commit
        };

        format!("({git_info} {build_date})")
    } else {
        format!("({build_date})")
    };

    println!("cargo:rustc-env=CARGO_LAMBDA_BUILD_INFO={build_info}");
    let _ = build_data::no_debug_rebuilds();
}

fn git_sha_from_env() -> String {
    let mut s = std::env::var("CARGO_LAMBDA_RELEASE_GIT_SHA").unwrap_or_default();
    s.truncate(7);
    s
}

#[cfg(feature = "skip-build-banner")]
fn print_banner() {}

#[cfg(not(feature = "skip-build-banner"))]
fn print_banner() {
    print_warning("");
    print_warning(
        "You're compiling a version of Cargo Lambda from a source that doesn't guarantee reproducibility.",
    );
    print_warning(
        "Please consider using one of the officially supported installation methods instead:",
    );
    print_warning("");
    print_warning("https://www.cargo-lambda.info/guide/installation.html");
    print_warning("");
    print_warning(
        "If you're interested in contributing to Cargo Lambda, and would like to hide this banner,",
    );
    print_warning(
        "you can build Cargo Lambda from source with `make build` or `make build-release`.",
    );
    print_warning("Read the contributing guide for more information:");
    print_warning("");
    print_warning("https://github.com/cargo-lambda/cargo-lambda/blob/main/CONTRIBUTING.md");
    print_warning("");
}

#[cfg(not(feature = "skip-build-banner"))]
fn print_warning(s: &str) {
    println!("cargo:warning=\x1b[2K\r\x1b[1m\x1b[33mwarning\x1b[0m {}", s);
}
