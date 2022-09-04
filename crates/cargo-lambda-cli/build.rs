fn main() {
    let git_commit = build_data::get_git_commit_short().unwrap_or_else(|_| git_sha_from_env());
    let git_dirty = build_data::get_git_dirty().unwrap_or_default();
    let build_date = build_data::format_date(build_data::now());

    let build_info = if !git_commit.is_empty() {
        let git_info = if git_dirty {
            format!("{}-dirty", git_commit)
        } else {
            git_commit
        };

        format!("({git_info} {build_date})")
    } else {
        format!("({build_date})")
    };

    println!("cargo:rustc-env=CARGO_LAMBDA_BUILD_INFO={}", build_info);
    build_data::no_debug_rebuilds();
}

fn git_sha_from_env() -> String {
    let mut s = std::env::var("CARGO_LAMBDA_RELEASE_GIT_SHA").unwrap_or_default();
    s.truncate(7);
    s
}
