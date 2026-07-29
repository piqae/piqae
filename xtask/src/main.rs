use std::{
    collections::BTreeSet,
    env,
    error::Error,
    ffi::OsStr,
    fmt, fs,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

type TaskResult<T = ()> = Result<T, TaskError>;

#[derive(Debug)]
struct TaskError(String);

impl fmt::Display for TaskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for TaskError {}

fn main() {
    let arguments: Vec<_> = env::args().skip(1).collect();
    if let Err(error) = execute(&arguments) {
        eprintln!("xtask: {error}");
        std::process::exit(1);
    }
}

fn execute(arguments: &[String]) -> TaskResult {
    let root = repository_root()?;
    match arguments {
        [command] if command == "doctor" => doctor(&root),
        [command] if command == "dev" => dev(&root, "web"),
        [command, target] if command == "dev" => dev(&root, target),
        [command, scope] if command == "test" && scope == "changed" => test_changed(&root),
        [command, scope] if command == "test" && scope == "all" => test_all(&root),
        [command, action] if command == "fixture" && action == "reset" => fixture_reset(&root),
        [command, action] if command == "release" && action == "check" => release_check(&root),
        [] | [..] if arguments.iter().any(|argument| argument == "--help") => {
            print_help();
            Ok(())
        }
        _ => {
            print_help();
            Err(TaskError("unknown xtask command".into()))
        }
    }
}

fn print_help() {
    println!(
        "\
Spool contributor tasks

Usage:
  cargo xtask doctor
  cargo xtask dev [web|agent]
  cargo xtask test changed
  cargo xtask test all
  cargo xtask fixture reset
  cargo xtask release check

No command submits a physical print job."
    );
}

fn repository_root() -> TaskResult<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|error| TaskError(format!("cannot run git: {error}")))?;
    if !output.status.success() {
        return Err(TaskError(
            "run xtask from inside the Spool repository".into(),
        ));
    }
    let root = String::from_utf8(output.stdout)
        .map_err(|error| TaskError(format!("git returned a non-UTF-8 path: {error}")))?;
    Ok(PathBuf::from(root.trim()))
}

fn doctor(root: &Path) -> TaskResult {
    println!("Spool development environment");
    let mut missing = Vec::new();
    for (tool, arguments) in [
        ("git", &["--version"][..]),
        ("rustc", &["--version"][..]),
        ("cargo", &["--version"][..]),
        ("node", &["--version"][..]),
        ("pnpm", &["--version"][..]),
    ] {
        if let Ok(version) = tool_output(root, tool, arguments) {
            println!("  ok      {tool:<8} {version}");
        } else {
            println!("  missing {tool}");
            missing.push(tool);
        }
    }
    match tool_output(root, "docker", &["--version"]) {
        Ok(version) => println!("  ok      docker   {version}"),
        Err(_) => println!("  optional docker   not installed; required only for self-hosting"),
    }
    if env::var("SPOOL_ALLOW_PHYSICAL_TESTS").as_deref() == Ok("1") {
        println!("  warning physical-printer opt-in is enabled in this shell");
    } else {
        println!("  safe    physical-printer tests are disabled");
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(TaskError(format!(
            "install required tools: {} (see mise.toml)",
            missing.join(", ")
        )))
    }
}

fn tool_output(root: &Path, program: &str, arguments: &[&str]) -> TaskResult<String> {
    let output = Command::new(program)
        .args(arguments)
        .current_dir(root)
        .output()
        .map_err(|error| TaskError(format!("{program} is unavailable: {error}")))?;
    if !output.status.success() {
        return Err(TaskError(format!("{program} returned {}", output.status)));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let value = if stdout.trim().is_empty() {
        stderr.trim()
    } else {
        stdout.trim()
    };
    Ok(value.lines().next().unwrap_or_default().to_owned())
}

fn dev(root: &Path, target: &str) -> TaskResult {
    match target {
        "web" => {
            let mut process = command(root, "pnpm", ["--filter", "@spool/web", "dev"]);
            process
                .env("SPOOL_AUTH_MODE", "demo")
                .env("PUBLIC_SPOOL_DASHBOARD_MODE", "demo");
            run(process)
        }
        "agent" => {
            let data_directory = env::var_os("SPOOL_STATE_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| root.join(".spool-dev"));
            fs::create_dir_all(&data_directory).map_err(|error| {
                TaskError(format!(
                    "cannot create {}: {error}",
                    data_directory.display()
                ))
            })?;
            run(command(
                root,
                "cargo",
                ["build", "-p", "spool-fake-executor"],
            ))?;
            let executor = root
                .join("target")
                .join("debug")
                .join(format!("spool-fake-executor{}", env::consts::EXE_SUFFIX));
            let mut process = command(root, "cargo", ["run", "-p", "spool-agent", "--"]);
            process.args([
                "--mode",
                "local",
                "--data-dir",
                path_text(&data_directory)?,
                "--executor",
                "process",
                "--executor-path",
                path_text(&executor)?,
            ]);
            run(process)
        }
        _ => Err(TaskError("dev target must be web or agent".into())),
    }
}

fn test_changed(root: &Path) -> TaskResult {
    let files = changed_files(root)?;
    if files.is_empty() {
        println!("No changed files detected.");
        return Ok(());
    }
    run(command(root, "git", ["diff", "--check", "HEAD"]))?;

    let mut packages = BTreeSet::new();
    let mut all_rust = false;
    let mut javascript = false;
    let mut macos = false;
    for file in &files {
        if file == Path::new("Cargo.toml") || file == Path::new("Cargo.lock") {
            all_rust = true;
        } else if (file.starts_with("crates")
            || file.starts_with("bins")
            || file.starts_with("xtask"))
            && (file.extension() == Some(OsStr::new("rs"))
                || file.file_name() == Some(OsStr::new("Cargo.toml")))
            && let Some(package) = package_for_path(root, file)?
        {
            packages.insert(package);
        }
        if file.starts_with("apps")
            || file.starts_with("sdk")
            || matches!(
                file.to_str(),
                Some("package.json" | "pnpm-lock.yaml" | "pnpm-workspace.yaml")
            )
        {
            javascript = true;
        }
        if file.starts_with("shells/macos") {
            macos = true;
        }
    }

    if all_rust {
        test_rust_workspace(root)?;
    } else if !packages.is_empty() {
        run(command(root, "cargo", ["fmt", "--all", "--", "--check"]))?;
        for package in packages {
            run(command(
                root,
                "cargo",
                [
                    "clippy",
                    "-p",
                    &package,
                    "--all-targets",
                    "--",
                    "-D",
                    "warnings",
                ],
            ))?;
            run(command(root, "cargo", ["test", "-p", &package]))?;
        }
    }
    if javascript {
        test_javascript(root)?;
    }
    if macos && cfg!(target_os = "macos") {
        test_macos(root)?;
    }
    Ok(())
}

fn changed_files(root: &Path) -> TaskResult<BTreeSet<PathBuf>> {
    let mut files = BTreeSet::new();
    collect_git_paths(root, &["diff", "--name-only", "HEAD"], &mut files)?;
    collect_git_paths(
        root,
        &["ls-files", "--others", "--exclude-standard"],
        &mut files,
    )?;
    if command_success(root, "git", &["rev-parse", "--verify", "origin/main"]) {
        let merge_base = tool_output(root, "git", &["merge-base", "HEAD", "origin/main"])?;
        collect_git_paths(
            root,
            &["diff", "--name-only", &format!("{merge_base}..HEAD")],
            &mut files,
        )?;
    }
    Ok(files)
}

fn collect_git_paths(
    root: &Path,
    arguments: &[&str],
    destination: &mut BTreeSet<PathBuf>,
) -> TaskResult {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .map_err(|error| TaskError(format!("cannot inspect changed files: {error}")))?;
    if !output.status.success() {
        return Err(TaskError(format!(
            "git {} failed with {}",
            arguments.join(" "),
            output.status
        )));
    }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if !line.is_empty() {
            destination.insert(PathBuf::from(line));
        }
    }
    Ok(())
}

fn package_for_path(root: &Path, file: &Path) -> TaskResult<Option<String>> {
    let components: Vec<_> = file.components().collect();
    let manifest = if file.starts_with("xtask") {
        root.join("xtask/Cargo.toml")
    } else if components.len() >= 2 {
        root.join(components[0].as_os_str())
            .join(components[1].as_os_str())
            .join("Cargo.toml")
    } else {
        return Ok(None);
    };
    if !manifest.is_file() {
        return Ok(None);
    }
    let content = fs::read_to_string(&manifest)
        .map_err(|error| TaskError(format!("cannot read {}: {error}", manifest.display())))?;
    let name = content
        .lines()
        .find_map(|line| line.trim().strip_prefix("name = "))
        .and_then(|value| value.trim_matches('"').split('"').next())
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    Ok(name)
}

fn test_all(root: &Path) -> TaskResult {
    test_rust_workspace(root)?;
    test_javascript(root)?;
    if cfg!(target_os = "macos") {
        test_macos(root)?;
    }
    Ok(())
}

fn test_rust_workspace(root: &Path) -> TaskResult {
    run(command(root, "cargo", ["fmt", "--all", "--", "--check"]))?;
    run(command(
        root,
        "cargo",
        [
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    ))?;
    run(command(root, "cargo", ["test", "--workspace", "--locked"]))
}

fn test_javascript(root: &Path) -> TaskResult {
    run(command(root, "pnpm", ["check"]))?;
    run(command(root, "pnpm", ["test"]))
}

fn test_macos(root: &Path) -> TaskResult {
    run(command(
        root,
        "swift",
        ["test", "--package-path", "shells/macos"],
    ))
}

fn fixture_reset(root: &Path) -> TaskResult {
    for name in [".spool-dev", ".spool-test-fixtures"] {
        let target = root.join(name);
        if target.exists() {
            fs::remove_dir_all(&target).map_err(|error| {
                TaskError(format!("cannot remove {}: {error}", target.display()))
            })?;
            println!("Removed {}", target.display());
        }
    }
    println!("Installed printers and operating-system queues were not touched.");
    Ok(())
}

fn release_check(root: &Path) -> TaskResult {
    test_all(root)?;
    run(command(root, "pnpm", ["build"]))?;
    if command_success(root, "cargo", &["deny", "--version"]) {
        run(command(
            root,
            "cargo",
            [
                "deny",
                "check",
                "--allow",
                "warnings",
                "--hide-inclusion-graph",
            ],
        ))?;
    } else {
        println!("warning: cargo-deny is unavailable; CI must run the dependency policy gate");
    }
    check_licenses(root)?;
    run(command(root, "git", ["diff", "--check", "HEAD"]))?;
    let status = tool_output(root, "git", &["status", "--porcelain"])?;
    if !status.is_empty() {
        return Err(TaskError(
            "release checks require a clean working tree".into(),
        ));
    }
    Ok(())
}

fn check_licenses(root: &Path) -> TaskResult {
    let license = fs::read_to_string(root.join("LICENSE"))
        .map_err(|error| TaskError(format!("cannot read LICENSE: {error}")))?;
    if !license.contains("Apache License") {
        return Err(TaskError("root LICENSE is not Apache-2.0".into()));
    }
    let mut manifests = Vec::new();
    collect_manifests(root, &mut manifests)?;
    for manifest in manifests {
        let content = fs::read_to_string(&manifest)
            .map_err(|error| TaskError(format!("cannot read {}: {error}", manifest.display())))?;
        if content.contains("AGPL-") || content.contains("CC-BY-") {
            return Err(TaskError(format!(
                "{} declares a non-Apache project license",
                manifest.display()
            )));
        }
        let is_cargo = manifest.file_name() == Some(OsStr::new("Cargo.toml"));
        let is_private_package = content.contains("\"private\": true");
        let declares_apache = if is_cargo {
            content.contains("license = \"Apache-2.0\"")
                || content.contains("license.workspace = true")
        } else {
            content.contains("\"license\": \"Apache-2.0\"")
        };
        if !declares_apache && !is_private_package {
            return Err(TaskError(format!(
                "{} does not declare Apache-2.0",
                manifest.display()
            )));
        }
    }
    Ok(())
}

fn collect_manifests(directory: &Path, destination: &mut Vec<PathBuf>) -> TaskResult {
    for entry in fs::read_dir(directory)
        .map_err(|error| TaskError(format!("cannot inspect {}: {error}", directory.display())))?
    {
        let entry = entry.map_err(|error| TaskError(format!("cannot inspect entry: {error}")))?;
        let path = entry.path();
        if path.is_dir() {
            if matches!(
                path.file_name().and_then(OsStr::to_str),
                Some(
                    ".git"
                        | "node_modules"
                        | "target"
                        | ".build"
                        | ".artifacts"
                        | ".vercel"
                        | ".svelte-kit"
                        | "build"
                        | "dist"
                )
            ) {
                continue;
            }
            collect_manifests(&path, destination)?;
        } else if matches!(
            path.file_name().and_then(OsStr::to_str),
            Some("Cargo.toml" | "package.json")
        ) {
            destination.push(path);
        }
    }
    Ok(())
}

fn command<I, S>(root: &Path, program: &str, arguments: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    command.args(arguments).current_dir(root);
    command
}

fn run(mut command: Command) -> TaskResult {
    println!("+ {}", display_command(&command));
    let status = command
        .status()
        .map_err(|error| TaskError(format!("cannot run command: {error}")))?;
    require_success(status, &display_command(&command))
}

fn command_success(root: &Path, program: &str, arguments: &[&str]) -> bool {
    Command::new(program)
        .args(arguments)
        .current_dir(root)
        .status()
        .is_ok_and(|status| status.success())
}

fn require_success(status: ExitStatus, label: &str) -> TaskResult {
    if status.success() {
        Ok(())
    } else {
        Err(TaskError(format!("{label} failed with {status}")))
    }
}

fn display_command(command: &Command) -> String {
    let mut value = command.get_program().to_string_lossy().into_owned();
    for argument in command.get_args() {
        value.push(' ');
        value.push_str(&argument.to_string_lossy());
    }
    value
}

fn path_text(path: &Path) -> TaskResult<&str> {
    path.to_str()
        .ok_or_else(|| TaskError(format!("path is not valid UTF-8: {}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_surface_has_no_physical_print_operation() {
        let source = include_str!("main.rs");
        assert!(source.contains("No command submits a physical print job."));
        assert!(!source.contains("\"print\" =>"));
    }

    #[test]
    fn disposable_fixture_names_are_repository_local() {
        for name in [".spool-dev", ".spool-test-fixtures"] {
            let path = Path::new(name);
            assert!(path.is_relative());
            assert_eq!(path.components().count(), 1);
        }
    }

    #[test]
    fn project_manifests_are_apache_licensed() {
        let result = repository_root().and_then(|root| check_licenses(&root));
        assert!(result.is_ok(), "{result:?}");
    }
}
