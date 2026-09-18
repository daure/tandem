#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use serde_json::json;

mod existing;

struct Fixture {
    _directory: tempfile::TempDir,
    home: PathBuf,
    bin: PathBuf,
    source: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("home with spaces");
        let bin = directory.path().join("bin");
        let source = directory.path().join("source");
        fs::create_dir_all(home.join("templates/website")).unwrap();
        fs::create_dir(&bin).unwrap();
        fs::create_dir(&source).unwrap();
        git(&source, &["init", "-b", "trunk"]);
        fs::write(source.join("file.txt"), "source content").unwrap();
        fs::write(source.join("AGENTS.md"), "Repository guidance\n").unwrap();
        git(&source, &["add", "."]);
        git(&source, &["commit", "-m", "Seed fixture"]);
        fs::write(
            home.join("templates/website/compose.yaml"),
            "services: {}\n",
        )
        .unwrap();
        fs::write(
            home.join("templates/website/tandem.json"),
            json!({"repositories": [{"source": source, "target": "app"}]}).to_string(),
        )
        .unwrap();
        let connection = rusqlite::Connection::open(home.join("settings.sqlite3")).unwrap();
        connection
            .execute_batch(include_str!("../../../migrations/0001_app_settings.sql"))
            .unwrap();
        fs::write(bin.join("docker"), DOCKER).unwrap();
        fs::set_permissions(bin.join("docker"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(
            home.join("inspect.json"),
            json!([{
                "Id": "fixture-container",
                "Config": {"Image": "fixture", "Labels": {
                    "io.tandem.namespace": "cli-test", "io.tandem.kind": "instance",
                    "io.tandem.instance": "review", "io.tandem.template": "website",
                    "io.tandem.template-directory": home.join("templates/website"),
                    "io.tandem.workspace": home.join("workspaces/review"),
                    "com.docker.compose.project": "cli-test-review",
                    "com.docker.compose.service": "app",
                    "com.docker.compose.container-number": "1"
                }},
                "State": {"Status": "running", "Running": true, "StartedAt": "2026-01-01T00:00:00Z"}
            }])
            .to_string(),
        )
        .unwrap();
        Self {
            _directory: directory,
            home,
            bin,
            source,
        }
    }

    fn save_command(&self, command: &str) {
        rusqlite::Connection::open(self.home.join("settings.sqlite3")).unwrap().execute(
            "INSERT OR REPLACE INTO app_settings(key, value) VALUES ('instances.open_command', ?1)",
            [command],
        ).unwrap();
    }

    fn command(&self, arguments: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_tandem"));
        let path = std::env::join_paths(
            std::iter::once(self.bin.clone())
                .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
        )
        .unwrap();
        command
            .args(arguments)
            .env("PATH", path)
            .env("TANDEM_HOME", &self.home)
            .env("TANDEM_NAMESPACE", "cli-test")
            .env("TANDEM_GATEWAY_PORT", "9876")
            .env("RUST_BACKTRACE", "0")
            .env_remove("TANDEM_INSTRUCTIONS_FILE")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }

    fn run(&self, arguments: &[&str]) -> Output {
        wait_output(self.command(arguments).spawn().unwrap())
    }
}

fn git(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(directory)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn wait_output(mut child: Child) -> Output {
    let deadline = Instant::now() + Duration::from_secs(20);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!("CLI timed out: {}", String::from_utf8_lossy(&output.stderr));
        }
        thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().unwrap()
}

const OPEN: &str = r#"test "$(cat app/file.txt)" = 'source content' || exit 21
test -s AGENTS.md || exit 27
grep -q './app/AGENTS.md' AGENTS.md || exit 28
git -C app branch --show-current > "$TANDEM_HOME/branch"
printf '%s\n%s\n%s\n' "$TANDEM_INSTANCE" "$TANDEM_WORKSPACE" "$PWD" >> "$TANDEM_HOME/opened""#;

#[test]
fn open_aliases_launch_once_after_checkout_before_compose_startup() {
    for flag in ["--open-command", "-oc"] {
        let fixture = Fixture::new();
        fixture.save_command(OPEN);
        let output = wait_output(
            fixture
                .command(&["new-instance", "review", "-t", "website", flag])
                .env("EXPECT_OPEN", "1")
                .spawn()
                .unwrap(),
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let workspace = fixture.home.join("workspaces/review");
        assert_eq!(
            fs::read_to_string(fixture.home.join("opened")).unwrap(),
            format!("review\n{}\n{}\n", workspace.display(), workspace.display())
        );
        assert_eq!(
            fs::read_to_string(fixture.home.join("branch")).unwrap(),
            "review\n"
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("Instance review ready"));
    }
}

#[test]
fn creation_without_open_flag_preserves_saved_command_without_executing_it() {
    let fixture = Fixture::new();
    fixture.save_command(OPEN);
    let output = fixture.run(&["new-instance", "review", "--template", "website"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fixture
            .home
            .join("workspaces/review/app/file.txt")
            .is_file()
    );
    assert!(!fixture.home.join("opened").exists());
    assert!(fixture.home.join("workspaces/review/AGENTS.md").is_file());
}

#[test]
fn preparation_failures_never_launch_the_saved_command() {
    for missing in ["template", "repository", "second-repository"] {
        let fixture = Fixture::new();
        fixture.save_command(OPEN);
        if missing == "repository" {
            fs::remove_dir_all(&fixture.source).unwrap();
        }
        if missing == "second-repository" {
            fs::write(
                fixture.home.join("templates/website/tandem.json"),
                json!({
                    "repositories": [
                        {"source": fixture.source, "target": "app"},
                        {"source": fixture.home.join("missing-source"), "target": "second"}
                    ]
                })
                .to_string(),
            )
            .unwrap();
        }
        let template = if missing == "template" {
            "missing"
        } else {
            "website"
        };
        let output = fixture.run(&["new-instance", "review", "-t", template, "-oc"]);
        assert!(!output.status.success());
        assert!(!fixture.home.join("opened").exists());
        assert!(!fixture.home.join("configured").exists());
        assert!(!fixture.home.join("started").exists());
        if missing == "second-repository" {
            assert!(
                fixture
                    .home
                    .join("workspaces/review/app/file.txt")
                    .is_file()
            );
        }
    }
}

#[test]
fn an_empty_command_opens_the_prepared_workspace_with_the_folder_opener() {
    let fixture = Fixture::new();
    fs::write(fixture.home.join("templates/website/tandem.json"), "{}").unwrap();
    fs::write(
        fixture.bin.join("xdg-open"),
        "#!/bin/sh\ntest -s \"$1/AGENTS.md\" || exit 21\nprintf '%s' \"$1\" > \"$TANDEM_HOME/opened\"\n",
    )
    .unwrap();
    fs::set_permissions(
        fixture.bin.join("xdg-open"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let output = wait_output(
        fixture
            .command(&["new-instance", "review", "-t", "website", "-oc"])
            .env("EXPECT_OPEN", "1")
            .spawn()
            .unwrap(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(fixture.home.join("opened")).unwrap(),
        fixture.home.join("workspaces/review").display().to_string()
    );
}

#[test]
fn workspace_guidance_failure_blocks_opening_and_container_startup() {
    let fixture = Fixture::new();
    fixture.save_command(OPEN);
    fs::write(
        fixture.home.join(".workspace-agents.bundled.md"),
        include_str!("../../../workspace-agents.template.md"),
    )
    .unwrap();
    fs::write(
        fixture.home.join("workspace-agents.template.md"),
        "{{unknown}}",
    )
    .unwrap();
    let output = fixture.run(&["new-instance", "review", "-t", "website", "-oc"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("unknown workspace AGENTS.md template placeholder")
    );
    assert!(!fixture.home.join("opened").exists());
    assert!(!fixture.home.join("started").exists());
    assert!(!fixture.home.join("workspaces/review/AGENTS.md").exists());
}

#[test]
fn an_updated_binary_uses_its_bundled_template_for_new_instances_and_preserves_existing_guidance() {
    let fixture = Fixture::new();
    fs::write(
        fixture.home.join("workspace-agents.template.md"),
        "Legacy template\n",
    )
    .unwrap();
    let existing = fixture.home.join("workspaces/other/AGENTS.md");
    fs::create_dir_all(existing.parent().unwrap()).unwrap();
    fs::write(&existing, "Existing instance guidance\n").unwrap();
    let output = fixture.run(&["new-instance", "review", "-t", "website"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = fs::read_to_string(fixture.home.join("workspaces/review/AGENTS.md")).unwrap();
    assert!(text.starts_with("# Workspace: review\n"));
    assert!(text.contains("If a repository has `AGENTS.md` or `agents.md`"));
    assert!(text.contains("./app/AGENTS.md"));
    assert_eq!(
        fs::read_to_string(existing).unwrap(),
        "Existing instance guidance\n"
    );
}

#[test]
fn startup_failure_is_reported_after_early_open_and_preserves_checkout() {
    let fixture = Fixture::new();
    fixture.save_command(OPEN);
    let output = wait_output(
        fixture
            .command(&["new-instance", "review", "-t", "website", "-oc"])
            .env("EXPECT_OPEN", "1")
            .env("FAIL_START", "1")
            .spawn()
            .unwrap(),
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("fixture startup failed"));
    assert!(fixture.home.join("opened").is_file());
    assert!(
        fixture
            .home
            .join("workspaces/review/app/file.txt")
            .is_file()
    );
}

#[test]
fn a_long_lived_editor_does_not_hold_the_cli_open() {
    let fixture = Fixture::new();
    fixture.save_command(&format!("{OPEN}\nwhile [ -d \"$TANDEM_HOME\" ] && [ ! -f \"$TANDEM_HOME/release-editor\" ]; do sleep 0.05; done\ntouch \"$TANDEM_HOME/editor-ended\""));
    let output = wait_output(
        fixture
            .command(&["new-instance", "review", "-t", "website", "-oc"])
            .env("EXPECT_OPEN", "1")
            .spawn()
            .unwrap(),
    );
    fs::write(fixture.home.join("release-editor"), "").unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fixture.home.join("editor-ended").exists() {
        assert!(Instant::now() < deadline, "editor did not survive CLI exit");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn delete_instance_removes_owned_resources_and_workspace() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.home.join("workspaces/review")).unwrap();
    fs::write(fixture.home.join("workspaces/review/keep"), "local work").unwrap();
    fs::write(fixture.home.join("started"), "").unwrap();

    let output = fixture.run(&["delete-instance", "review"]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Instance review deleted"));
    assert!(!fixture.home.join("workspaces/review").exists());
    assert!(
        fs::read_to_string(fixture.home.join("docker-calls"))
            .unwrap()
            .contains("rm --force --volumes fixture-container")
    );
}

#[test]
fn headless_delete_returns_before_deletion_finishes() {
    let fixture = Fixture::new();
    fs::create_dir_all(fixture.home.join("workspaces/review")).unwrap();
    fs::write(fixture.home.join("started"), "").unwrap();

    let output = wait_output(
        fixture
            .command(&["delete-instance", "review", "--headless"])
            .env("BLOCK_DELETE", "1")
            .spawn()
            .unwrap(),
    );

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("deletion started"));
    let deadline = Instant::now() + Duration::from_secs(5);
    while !fixture.home.join("deleting").exists() {
        assert!(Instant::now() < deadline, "headless deletion did not start");
        thread::sleep(Duration::from_millis(20));
    }
    assert!(fixture.home.join("workspaces/review").exists());
    fs::write(fixture.home.join("release-delete"), "").unwrap();
    while fixture.home.join("workspaces/review").exists() {
        assert!(
            Instant::now() < deadline,
            "headless deletion did not finish"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

const DOCKER: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$TANDEM_HOME/docker-calls"
case "$1" in
  ps)
    case "$*" in *cli-test-gateway*) exit 0;; esac
    if [ -f "$TANDEM_HOME/started" ]; then printf 'fixture-container\n'; fi
    ;;
  inspect) cat "$TANDEM_HOME/inspect.json";;
  network|volume) exit 0;;
  rm)
    if [ "$BLOCK_DELETE" = 1 ]; then
      touch "$TANDEM_HOME/deleting"
      while [ ! -f "$TANDEM_HOME/release-delete" ]; do sleep 0.05; done
    fi
    ;;
  compose)
    case "$*" in
      *'config --format json')
        touch "$TANDEM_HOME/configured"
        if [ "$BLOCK_CONFIG" = 1 ]; then
          count=0
          while [ ! -f "$TANDEM_HOME/release-config" ]; do
            count=$((count + 1))
            if [ "$count" -gt 200 ]; then exit 26; fi
            sleep 0.05
          done
        fi
        printf '{"services":{"app":{"image":"fixture"}}}\n'
        ;;
      *'up --detach'*)
        if [ "$TANDEM_INSTANCE" = gateway ]; then exit 0; fi
        if [ "$EXPECT_OPEN" = 1 ]; then
          count=0
          while [ ! -f "$TANDEM_HOME/opened" ]; do
            count=$((count + 1))
            if [ "$count" -gt 100 ]; then printf 'opener did not run before Compose startup\n' >&2; exit 22; fi
            sleep 0.05
          done
        fi
        if [ "$FAIL_START" = 1 ]; then printf 'fixture startup failed\n' >&2; exit 23; fi
        touch "$TANDEM_HOME/started"
        ;;
      *) printf 'unexpected Compose command\n' >&2; exit 24;;
    esac
    ;;
  *) printf 'unexpected Docker command\n' >&2; exit 25;;
esac
"#;
