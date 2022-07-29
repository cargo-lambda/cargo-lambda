fn main() {
    let git_commit = build_data::get_git_commit_short();
    let git_dirty = build_data::get_git_dirty();
    let build_date = build_data::format_date(build_data::now());

    let build_info = if let (Ok(git_commit), Ok(git_dirty)) = (git_commit, git_dirty) {
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
