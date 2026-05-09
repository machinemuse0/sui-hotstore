use std::process::Command;

fn main() {
    emit_command_env("HOTSTORE_GIT_SHA", "git", &["rev-parse", "HEAD"]);
    emit_command_env("HOTSTORE_GIT_DIRTY", "git", &["status", "--porcelain"]);
    emit_command_env("HOTSTORE_RUSTC_VERSION", "rustc", &["--version"]);
}

fn emit_command_env(key: &str, program: &str, args: &[&str]) {
    let Ok(output) = Command::new(program).args(args).output() else {
        return;
    };
    if !output.status.success() {
        return;
    }

    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if key == "HOTSTORE_GIT_DIRTY" {
        let dirty = if value.is_empty() { "false" } else { "true" };
        println!("cargo:rustc-env={key}={dirty}");
    } else if !value.is_empty() {
        println!("cargo:rustc-env={key}={value}");
    }
}
