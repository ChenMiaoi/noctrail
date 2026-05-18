use std::path::Path;

pub(super) fn review_output_command(marker: &str) -> String {
    format!("printf '{marker}\\n'")
}

pub(super) fn review_file_command(path: &Path) -> String {
    format!("sh -lc 'printf review-high > \"{}\"'", path.display())
}

pub(super) fn review_patch_cli_command(path: &Path) -> Vec<String> {
    vec![
        "sh".to_string(),
        "-lc".to_string(),
        format!("cat \"{}\"", path.display()),
    ]
}
