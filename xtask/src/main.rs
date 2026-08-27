use std::{
    collections::BTreeSet,
    env,
    error::Error,
    ffi::OsStr,
    fmt::{self, Write as _},
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
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
        [command, options @ ..] if command == "preflight" => preflight(&root, options),
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
Piqae contributor tasks

Usage:
  cargo xtask doctor
  cargo xtask dev [web|agent]
  cargo xtask test changed
  cargo xtask test all
  cargo xtask preflight [--all] [--list]
  cargo xtask fixture reset
  cargo xtask release check

`preflight` reproduces the CI jobs this change selects, names any missing
prerequisite before it spends time, and never claims a pass for a job it
could not run.

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
            "run xtask from inside the Piqae repository".into(),
        ));
    }
    let root = String::from_utf8(output.stdout)
        .map_err(|error| TaskError(format!("git returned a non-UTF-8 path: {error}")))?;
    Ok(PathBuf::from(root.trim()))
}

fn doctor(root: &Path) -> TaskResult {
    println!("Piqae development environment");
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
    if env::var("PIQAE_ALLOW_PHYSICAL_TESTS").as_deref() == Ok("1") {
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
            let mut process = command(root, "pnpm", ["--filter", "@piqae/web", "dev"]);
            process
                .env("PIQAE_AUTH_MODE", "demo")
                .env("PUBLIC_PIQAE_DASHBOARD_MODE", "demo");
            run(process)
        }
        "agent" => {
            let data_directory = env::var_os("PIQAE_STATE_DIR")
                .map_or_else(|| root.join(".piqae-dev"), PathBuf::from);
            fs::create_dir_all(&data_directory).map_err(|error| {
                TaskError(format!(
                    "cannot create {}: {error}",
                    data_directory.display()
                ))
            })?;
            run(command(
                root,
                "cargo",
                ["build", "-p", "piqae-fake-executor"],
            ))?;
            let executor = root
                .join("target")
                .join("debug")
                .join(format!("piqae-fake-executor{}", env::consts::EXE_SUFFIX));
            let mut process = command(root, "cargo", ["run", "-p", "piqae-agent", "--"]);
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
        if file.starts_with("shells/macos") || file.starts_with("sdk/apple") {
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
    ))?;
    run(command(
        root,
        "release/tools/test_apple_node_sdk.sh",
        std::iter::empty::<&str>(),
    ))?;
    run(command(
        root,
        "release/tools/test_apple_node_sdk_linked.sh",
        std::iter::empty::<&str>(),
    ))
}

fn fixture_reset(root: &Path) -> TaskResult {
    for name in [".piqae-dev", ".piqae-test-fixtures"] {
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
    run(command(
        root,
        "python3",
        ["release/tools/check_printpacket_source_policy.py"],
    ))?;
    run(command(
        root,
        "python3",
        ["release/tools/check_postgres_release_tests.py"],
    ))?;
    test_all(root)?;
    let mut build = command(root, "pnpm", ["build"]);
    if env::var_os("PAYLOAD_SECRET").is_none() {
        build.env(
            "PAYLOAD_SECRET",
            "piqae-release-build-only-secret-not-for-runtime",
        );
    }
    if env::var_os("DATABASE_URL").is_none() {
        build.env(
            "DATABASE_URL",
            "postgresql://piqae_cms_build@127.0.0.1:1/piqae_cms_build",
        );
    }
    run(build)?;
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
                        | ".next"
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

/// Probes for a command without letting its output leak into task output.
fn command_success(root: &Path, program: &str, arguments: &[&str]) -> bool {
    Command::new(program)
        .args(arguments)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
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

/// A prerequisite a preflight check needs before it can honestly run.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Need {
    /// A program that must be on `PATH`.
    Tool(&'static str),
    /// A disposable `PostgreSQL` database, named by `PIQAE_TEST_DATABASE_URL`.
    Postgres,
    /// An operating system the check can only run on.
    Os(&'static str),
}

impl Need {
    fn describe(self) -> String {
        match self {
            Self::Tool(tool) => format!("{tool} on PATH"),
            Self::Postgres => "PIQAE_TEST_DATABASE_URL".into(),
            Self::Os(os) => format!("{os} host"),
        }
    }

    /// Remediation a contributor can act on without reading CI YAML.
    fn remedy(self) -> String {
        match self {
            Self::Tool(tool @ ("cargo" | "rustc")) => {
                format!("install the pinned toolchain from rust-toolchain.toml ({tool})")
            }
            Self::Tool(tool @ ("node" | "pnpm")) => {
                format!("`mise install` provides {tool} at the version CI pins")
            }
            Self::Tool("cargo-deny") => {
                "install the CI-pinned tool with `cargo install cargo-deny --version 0.20.2 --locked`"
                    .into()
            }
            Self::Tool("cargo-audit") => {
                "install the CI-pinned tool with `cargo install cargo-audit --version 0.22.2 --locked`"
                    .into()
            }
            Self::Tool("swift") => "install Xcode command line tools".into(),
            Self::Tool("terraform") => "install Terraform 1.9.8, the version CI pins".into(),
            Self::Tool(tool) => format!("install {tool}"),
            Self::Postgres => concat!(
                "start a disposable database and export its URL, for example:\n",
                "      docker run --rm -d -p 5432:5432 -e POSTGRES_PASSWORD=postgres \\\n",
                "        -e POSTGRES_DB=piqae_test --name piqae-preflight-db postgres:16\n",
                "      export PIQAE_TEST_DATABASE_URL=",
                "postgres://postgres:postgres@127.0.0.1:5432/piqae_test\n",
                "      Point it only at a database you can afford to lose."
            )
            .into(),
            Self::Os(os) => format!("run this check on {os}; CI covers it on every push"),
        }
    }
}

/// One CI job reproduced locally.
struct Check {
    /// The `Select CI scope` outputs that select this job. An empty list means
    /// the job has no `if:` guard and always runs.
    scopes: &'static [&'static str],
    /// The job name as it appears in the GitHub Actions UI.
    job: &'static str,
    needs: &'static [Need],
    steps: &'static [&'static [&'static str]],
}

/// Every CI job that a contributor can reproduce, in the order CI would.
///
/// Jobs that only exist to produce release artifacts are listed in
/// [`CI_ONLY_JOBS`] instead of being silently dropped.
const CHECKS: &[Check] = &[
    Check {
        scopes: &[],
        job: "Supply-chain policy / Release policy and tooling",
        needs: &[Need::Tool("python3"), Need::Tool("ruby")],
        steps: &[
            &[
                "python3",
                "-m",
                "unittest",
                "discover",
                "-s",
                "release/tools",
                "-p",
                "test_*.py",
            ],
            &["ruby", "release/tools/check_release_policy.rb"],
            &["ruby", "release/tools/test_release_policy.rb"],
            &["python3", "release/tools/check_security_exceptions.py"],
            &["python3", "release/tools/check_competitor_mentions.py"],
            &[
                "python3",
                "release/tools/check_printpacket_source_policy.py",
            ],
        ],
    },
    Check {
        scopes: &[],
        job: "Supply-chain policy / workflow policy",
        needs: &[Need::Tool("python3")],
        steps: &[
            &[
                "python3",
                "release/tools/check_workflow_pins.py",
                "@workflows",
            ],
            &[
                "python3",
                "release/tools/check_workflow_runners.py",
                "@workflows",
            ],
        ],
    },
    Check {
        scopes: &["dependency_policy"],
        job: "Supply-chain policy / Rust dependency policy",
        needs: &[
            Need::Tool("cargo"),
            Need::Tool("cargo-deny"),
            Need::Tool("cargo-audit"),
        ],
        steps: &[
            &["cargo", "deny", "check", "--hide-inclusion-graph"],
            &["cargo", "audit"],
        ],
    },
    Check {
        scopes: &["rust_server", "rust_shared"],
        job: "CI / Rust (ubuntu-latest)",
        needs: &[Need::Tool("cargo")],
        steps: &[
            &["cargo", "fmt", "--all", "--", "--check"],
            &[
                "cargo",
                "clippy",
                "--workspace",
                "--all-targets",
                "--locked",
                "--",
                "-D",
                "warnings",
            ],
            &["cargo", "test", "--workspace", "--locked"],
        ],
    },
    Check {
        scopes: &["rust_server", "rust_shared"],
        job: "CI / Rust (ubuntu-latest, otlp)",
        needs: &[Need::Tool("cargo")],
        steps: &[
            &[
                "cargo",
                "clippy",
                "-p",
                "piqae-control-plane",
                "--all-targets",
                "--features",
                "otlp",
                "--locked",
                "--",
                "-D",
                "warnings",
            ],
            &[
                "cargo",
                "test",
                "-p",
                "piqae-control-plane",
                "--features",
                "otlp",
                "--locked",
            ],
            &[
                "cargo",
                "clippy",
                "-p",
                "piqae-control-plane",
                "--all-targets",
                "--features",
                "sentry",
                "--locked",
                "--",
                "-D",
                "warnings",
            ],
            &[
                "cargo",
                "test",
                "-p",
                "piqae-control-plane",
                "--features",
                "sentry",
                "--locked",
            ],
            &[
                "cargo",
                "clippy",
                "-p",
                "piqae-control-plane",
                "--all-targets",
                "--features",
                "otlp,sentry",
                "--locked",
                "--",
                "-D",
                "warnings",
            ],
        ],
    },
    Check {
        scopes: &["rust_server", "rust_shared"],
        job: "CI / Rust (PostgreSQL evidence)",
        needs: &[Need::Tool("cargo"), Need::Tool("python3"), Need::Postgres],
        steps: &[&["python3", "release/tools/check_postgres_release_tests.py"]],
    },
    Check {
        scopes: &["macos_rust"],
        job: "CI / Rust (macos-latest)",
        needs: &[Need::Os("macos"), Need::Tool("cargo")],
        steps: &[
            &[
                "cargo",
                "clippy",
                "--locked",
                "--all-targets",
                "-p",
                "piqae-agent",
                "-p",
                "piqae-executor-cups",
                "--",
                "-D",
                "warnings",
            ],
            &[
                "cargo",
                "test",
                "--locked",
                "-p",
                "piqae-agent",
                "-p",
                "piqae-executor-cups",
            ],
        ],
    },
    Check {
        scopes: &["macos_shell"],
        job: "CI / macOS menu shell",
        needs: &[Need::Os("macos"), Need::Tool("swift")],
        steps: &[
            &["swift", "test", "--package-path", "shells/macos"],
            &["release/tools/test_apple_node_sdk.sh"],
            &["release/tools/test_apple_node_sdk_linked.sh"],
        ],
    },
    Check {
        scopes: &["windows_rust"],
        job: "CI / Rust (windows-latest)",
        needs: &[
            Need::Os("windows"),
            Need::Tool("cargo"),
            Need::Tool("pwsh"),
            Need::Tool("dotnet"),
        ],
        steps: &[
            &[
                "cargo",
                "clippy",
                "--locked",
                "--all-targets",
                "--target",
                "x86_64-pc-windows-msvc",
                "-p",
                "piqae-agent",
                "-p",
                "piqaectl",
                "-p",
                "piqae-executor-windows",
                "-p",
                "piqae-shell-windows",
                "-p",
                "piqae-local-ipc",
                "-p",
                "piqae-node-host-api",
                "-p",
                "piqae-node-runtime",
                "-p",
                "piqae-node-client",
                "-p",
                "piqae-node-ffi",
                "--",
                "-D",
                "warnings",
            ],
            &[
                "cargo",
                "test",
                "--locked",
                "--target",
                "x86_64-pc-windows-msvc",
                "-p",
                "piqae-agent",
                "-p",
                "piqaectl",
                "-p",
                "piqae-executor-windows",
                "-p",
                "piqae-shell-windows",
                "-p",
                "piqae-local-ipc",
                "-p",
                "piqae-node-host-api",
                "-p",
                "piqae-node-runtime",
                "-p",
                "piqae-node-client",
                "-p",
                "piqae-node-ffi",
            ],
            &[
                "pwsh",
                "-NoProfile",
                "-File",
                "release/tools/test_windows_node_sdk.ps1",
            ],
        ],
    },
    Check {
        scopes: &["web"],
        job: "CI / Web",
        needs: &[Need::Tool("pnpm"), Need::Tool("node")],
        steps: &[
            &["pnpm", "install", "--frozen-lockfile", "--prefer-offline"],
            &["pnpm", "--filter", "@piqae/web", "check"],
            &["pnpm", "--filter", "@piqae/web", "test"],
            &["pnpm", "--filter", "@piqae/web", "build"],
            &[
                "node",
                "--test",
                "deploy/cloudflare/domain-router/router.test.mjs",
            ],
            &[
                "pnpm",
                "dlx",
                "wrangler@4.115.0",
                "deploy",
                "--dry-run",
                "--config",
                "deploy/cloudflare/domain-router/wrangler.jsonc",
            ],
        ],
    },
    Check {
        scopes: &["sdk"],
        job: "CI / SDK",
        needs: &[Need::Tool("pnpm")],
        steps: &[
            &["pnpm", "install", "--frozen-lockfile", "--prefer-offline"],
            &["pnpm", "--filter", "@piqae/sdk", "generate:check"],
            &["pnpm", "--filter", "@piqae/sdk", "check"],
            &["pnpm", "--filter", "@piqae/sdk", "test"],
            &["pnpm", "--filter", "@printpacket/core", "generate:check"],
            &["pnpm", "--filter", "@printpacket/core", "check"],
            &["pnpm", "--filter", "@printpacket/core", "test"],
            &["pnpm", "--filter", "@printpacket/core", "build"],
            &["pnpm", "--filter", "@printpacket/core", "lint"],
            &["pnpm", "--filter", "@printpacket/core", "smoke:package"],
            &["pnpm", "--filter", "@piqae/sdk", "build"],
            &["pnpm", "--filter", "@piqae/sdk", "lint"],
            &["pnpm", "--filter", "@piqae/sdk", "smoke:package"],
        ],
    },
    Check {
        scopes: &["mcp"],
        job: "CI / MCP",
        needs: &[Need::Tool("pnpm")],
        steps: &[
            &["pnpm", "install", "--frozen-lockfile", "--prefer-offline"],
            &["pnpm", "--filter", "@piqae/sdk", "build"],
            &["pnpm", "--filter", "@piqae/mcp-server", "check"],
            &["pnpm", "--filter", "@piqae/mcp-server", "test"],
            &["pnpm", "--filter", "@piqae/mcp-server", "format:check"],
            &["pnpm", "--filter", "@piqae/mcp-server", "smoke:package"],
        ],
    },
    Check {
        scopes: &["shopify"],
        job: "CI / Shopify",
        needs: &[
            Need::Tool("pnpm"),
            Need::Tool("bash"),
            Need::Tool("grep"),
            Need::Postgres,
        ],
        steps: &[
            &["pnpm", "install", "--frozen-lockfile", "--prefer-offline"],
            &["pnpm", "--filter", "@piqae/sdk", "build"],
            &["pnpm", "--filter", "@piqae/shopify-app", "check"],
            &["pnpm", "--filter", "@piqae/shopify-app", "test"],
            &[
                "bash",
                "-c",
                "PIQAE_REQUIRE_POSTGRES_TESTS=1 pnpm --filter @piqae/shopify-app test:postgres",
            ],
            &["pnpm", "--filter", "@piqae/shopify-app", "format:check"],
            &[
                "bash",
                "-c",
                "set -euo pipefail; cd apps/shopify; SHOPIFY_CLIENT_ID=ci-client-id SHOPIFY_APP_URL=https://shopify-ci.example.com pnpm release:config; ! grep -Eq 'example\\.invalid|development-client-id' shopify.app.release.toml; grep -F 'https://shopify-ci.example.com/auth/callback' shopify.app.release.toml",
            ],
            &["pnpm", "--filter", "@piqae/shopify-app", "build"],
        ],
    },
    Check {
        scopes: &["openapi"],
        job: "CI / API contract",
        needs: &[Need::Tool("pnpm")],
        steps: &[&[
            "pnpm",
            "--package=@redocly/cli@1.34.3",
            "dlx",
            "redocly",
            "lint",
            "contracts/openapi/piqae-v1.yaml",
        ]],
    },
    Check {
        scopes: &["terraform"],
        job: "CI / Terraform",
        needs: &[Need::Tool("terraform")],
        steps: &[
            &[
                "terraform",
                "-chdir=deploy/terraform",
                "fmt",
                "-check",
                "-recursive",
            ],
            &[
                "terraform",
                "-chdir=deploy/terraform",
                "init",
                "-backend=false",
                "-input=false",
            ],
            &[
                "terraform",
                "-chdir=deploy/terraform",
                "validate",
                "-no-color",
            ],
        ],
    },
];

/// Selected CI work that cannot be reproduced from a contributor checkout.
/// Naming it is the point: preflight must not imply coverage it did not give.
const CI_ONLY_JOBS: &[(&str, &str)] = &[
    (
        "windows_installer",
        "CI / Rust (windows-latest) compiles the Inno Setup installer",
    ),
    (
        "macos_packaging",
        "the macOS release workflow signs and notarizes the bundle",
    ),
    (
        "release_tooling",
        "the release workflows are exercised on tags, not on a checkout",
    ),
];

/// Always-on CI work whose checked-history context is owned by GitHub Actions.
const ALWAYS_CI_ONLY_JOBS: &[&str] = &[
    "Supply-chain policy / Repository secret history scans the changed Git history with Gitleaks",
];

/// Reproduces the CI jobs that the current change selects.
fn preflight(root: &Path, arguments: &[String]) -> TaskResult {
    let mut everything = false;
    let mut list_only = false;
    for argument in arguments {
        match argument.as_str() {
            "--all" => everything = true,
            "--list" => list_only = true,
            other => {
                return Err(TaskError(format!(
                    "unknown preflight option '{other}'; expected --all or --list"
                )));
            }
        }
    }

    let scopes = selected_scopes(root, everything)?;
    let selected: Vec<&Check> = CHECKS
        .iter()
        .filter(|check| {
            check.scopes.is_empty() || check.scopes.iter().any(|scope| scopes.contains(*scope))
        })
        .collect();

    println!("Piqae preflight: the CI jobs this change selects");
    println!(
        "Scope came from release/tools/ci_changed_paths.py, the same classifier\n\
         the 'Select CI scope' job uses, so this list is what CI will run.\n"
    );

    let mut runnable = Vec::new();
    let mut deferred = Vec::new();
    let mut blocked = Vec::new();
    for check in selected {
        match unmet_need(root, check) {
            None => {
                println!("  run      {}", check.job);
                runnable.push(check);
            }
            Some(need @ Need::Os(_)) => {
                println!("  skip     {} ({})", check.job, need.remedy());
                deferred.push((check, need));
            }
            Some(need) => {
                println!("  blocked  {} (needs {})", check.job, need.describe());
                blocked.push((check, need));
            }
        }
    }
    for (scope, reason) in CI_ONLY_JOBS {
        if scopes.contains(*scope) {
            println!("  ci-only  {reason}");
        }
    }
    for reason in ALWAYS_CI_ONLY_JOBS {
        println!("  ci-only  {reason}");
    }
    if runnable.is_empty() && blocked.is_empty() {
        println!("\nNothing selected; CI would run no gated job for this change.");
    }

    if !blocked.is_empty() {
        println!("\nSome selected jobs cannot run on this machine:");
        for (check, need) in &blocked {
            println!("  {} needs {}", check.job, need.describe());
            println!("      {}", need.remedy());
        }
        println!(
            "\nEverything else still runs. Preflight will not report a pass while\n\
             a selected job is unverified."
        );
    }

    if list_only {
        return Ok(());
    }

    println!();
    for check in &runnable {
        for step in check.steps {
            run(check_command(root, step)?)?;
        }
    }

    println!("\nPreflight reproduced {} CI job(s).", runnable.len());
    for (check, need) in &deferred {
        println!("  not run: {} ({})", check.job, need.describe());
    }
    if blocked.is_empty() {
        return Ok(());
    }
    let mut message =
        String::from("every job that could run passed, but these were not verified here");
    for (check, need) in &blocked {
        let _ = write!(message, "\n  {} (needs {})", check.job, need.describe());
    }
    message.push_str(
        "\n\nInstall the prerequisite to verify them locally. CI runs every\n\
         selected job on the pull request either way.",
    );
    Err(TaskError(message))
}

/// Expands the `@workflows` placeholder into the checked-in workflow files, so
/// preflight passes the same argument list the shell glob gives CI.
fn check_command(root: &Path, step: &[&str]) -> TaskResult<Command> {
    let (program, rest) = step
        .split_first()
        .ok_or_else(|| TaskError("preflight step is empty".into()))?;
    let mut command = Command::new(program);
    command.current_dir(root);
    for argument in rest {
        if *argument == "@workflows" {
            for workflow in workflow_files(root)? {
                command.arg(workflow.strip_prefix(root).unwrap_or(&workflow));
            }
        } else {
            command.arg(argument);
        }
    }
    Ok(command)
}

fn workflow_files(root: &Path) -> TaskResult<Vec<PathBuf>> {
    let directory = root.join(".github/workflows");
    let mut files: Vec<_> = fs::read_dir(&directory)
        .map_err(|error| TaskError(format!("cannot inspect {}: {error}", directory.display())))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension() == Some(OsStr::new("yml")))
        .collect();
    files.sort();
    Ok(files)
}

fn unmet_need(root: &Path, check: &Check) -> Option<Need> {
    check.needs.iter().copied().find(|need| match need {
        Need::Tool(tool) => tool_output(root, tool, &["--version"]).is_err(),
        Need::Postgres => !env::var("PIQAE_TEST_DATABASE_URL")
            .is_ok_and(|value| value.trim_start().starts_with("postgres")),
        Need::Os(os) => env::consts::OS != *os,
    })
}

/// Classifies the change with the script CI uses, so local and CI scope cannot
/// drift apart.
fn selected_scopes(root: &Path, everything: bool) -> TaskResult<BTreeSet<String>> {
    let mut command = Command::new("python3");
    command
        .arg("release/tools/ci_changed_paths.py")
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let output = if everything {
        command.arg("--all").stdin(Stdio::null()).output()
    } else {
        command.stdin(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| TaskError(format!("cannot classify changed paths: {error}")))?;
        {
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| TaskError("cannot write the changed-path list".into()))?;
            for file in changed_files(root)? {
                writeln!(stdin, "{}", file.display())
                    .map_err(|error| TaskError(format!("cannot write a changed path: {error}")))?;
            }
        }
        child.wait_with_output()
    }
    .map_err(|error| TaskError(format!("cannot classify changed paths: {error}")))?;
    if !output.status.success() {
        return Err(TaskError(format!(
            "release/tools/ci_changed_paths.py failed with {}",
            output.status
        )));
    }
    let mut scopes = BTreeSet::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some((group, "true")) = line.trim().split_once('=') {
            scopes.insert(group.to_owned());
        }
    }
    Ok(scopes)
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
        for name in [".piqae-dev", ".piqae-test-fixtures"] {
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

    #[test]
    fn preflight_models_supply_chain_and_observability_commands() {
        assert!(
            CHECKS.iter().any(|check| {
                check.job == "Supply-chain policy / Rust dependency policy"
                    && check.scopes == ["dependency_policy"]
                    && check.steps
                        == [
                            &["cargo", "deny", "check", "--hide-inclusion-graph"][..],
                            &["cargo", "audit"][..],
                        ]
            }),
            "dependency policy check must reproduce CI"
        );
        assert!(
            CHECKS.iter().any(|check| {
                check.job == "Supply-chain policy / Release policy and tooling"
                    && check
                        .steps
                        .iter()
                        .any(|step| *step == ["ruby", "release/tools/check_release_policy.rb"])
                    && check
                        .steps
                        .iter()
                        .any(|step| *step == ["ruby", "release/tools/test_release_policy.rb"])
                    && check.steps.iter().any(|step| {
                        *step
                            == [
                                "python3",
                                "release/tools/check_printpacket_source_policy.py",
                            ]
                    })
            }),
            "release policy check must reproduce CI"
        );
        assert!(
            CHECKS.iter().any(|check| {
                check.job == "CI / Rust (ubuntu-latest, otlp)"
                    && check.steps.len() == 5
                    && check.steps.iter().any(|step| {
                        step.windows(2)
                            .any(|arguments| arguments == ["--features", "otlp,sentry"])
                    })
            }),
            "observability feature matrix must reproduce CI"
        );
    }

    #[test]
    fn preflight_exposes_github_owned_history_scan() {
        assert!(
            ALWAYS_CI_ONLY_JOBS
                .iter()
                .any(|job| job.contains("Gitleaks"))
        );
    }
}
