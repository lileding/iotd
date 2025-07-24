use std::process::Command;

fn main() {
    // Get git revision
    let output = Command::new("git")
        .args(&["rev-parse", "--short", "HEAD"])
        .output()
        .expect("Failed to execute git command");
    
    let git_hash = String::from_utf8(output.stdout)
        .expect("Invalid UTF-8")
        .trim()
        .to_string();
    
    // Check if working directory is clean
    let status_output = Command::new("git")
        .args(&["status", "--porcelain"])
        .output()
        .expect("Failed to execute git status");
    
    let is_dirty = !status_output.stdout.is_empty();
    let git_revision = if is_dirty {
        format!("{}-dirty", git_hash)
    } else {
        git_hash
    };
    
    // Set the GIT_REVISION environment variable for use in the code
    println!("cargo:rustc-env=GIT_REVISION={}", git_revision);
    
    // Rerun if .git directory changes
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
}