fn main() {
    let git_commit = match build_data::get_git_commit_short() {
        Ok(commit) => commit,
        Err(err) => {
            println!("failed to get git commit information: {err}");
            String::new()
        }
    };

    let git_dirty = match build_data::get_git_dirty() {
        Ok(dirty) => dirty,
        Err(err) => {
            println!("failed to get git status information: {err}");
            false
        }
    };

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
