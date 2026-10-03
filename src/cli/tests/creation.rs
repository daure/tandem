#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

use rusqlite::OptionalExtension;
use serde_json::json;

mod cleanup;
mod existing;
mod history;
mod inspection;
mod lifecycle;
mod opencode;
mod preparation;
mod providers;
mod rules;
mod startup;
mod workspaces;

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
        connection.execute(
            "INSERT INTO app_settings(key, value) VALUES ('opencode.clear_history_on_creation', 'false')",
            [],
        ).unwrap();
        fs::write(bin.join("docker"), DOCKER).unwrap();
        fs::set_permissions(bin.join("docker"), fs::Permissions::from_mode(0o755)).unwrap();
        opencode::install(&home, &bin);
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

    fn command(&self, arguments: &[&str]) -> Command {
        self.command_at(Path::new(env!("CARGO_BIN_EXE_tandem")), arguments)
    }

    fn command_at(&self, executable: &Path, arguments: &[&str]) -> Command {
        let mut command = Command::new(executable);
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
            .env("ZELLIJ_SESSION_NAME", "main")
            .env("XDG_STATE_HOME", self.home.join("state"))
            .env("OC_DAEMON_STATE", self.home.join("daemons"))
            .env_remove("TANDEM_INITIAL_PROMPT")
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

    fn runtime_record(&self, name: &str, kind: &str) -> Option<serde_json::Value> {
        let connection = rusqlite::Connection::open(self.home.join("settings.sqlite3")).unwrap();
        connection.busy_timeout(Duration::from_secs(5)).unwrap();
        let text: Option<String> = connection.query_row(
            "SELECT payload FROM runtime_records WHERE namespace = 'cli-test' AND name = ?1 AND kind = ?2",
            rusqlite::params![name.to_ascii_lowercase(), kind], |row|row.get(0)
        ).optional().unwrap();
        text.map(|text| serde_json::from_str(&text).unwrap())
    }

    fn set_runtime_record(&self, name: &str, kind: &str, value: &serde_json::Value) {
        let connection = rusqlite::Connection::open(self.home.join("settings.sqlite3")).unwrap();
        connection.busy_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(connection.execute(
            "UPDATE runtime_records SET payload = ?3 WHERE namespace = 'cli-test' AND name = ?1 AND kind = ?2",
            rusqlite::params![name.to_ascii_lowercase(),kind,value.to_string()]
        ).unwrap(),1);
    }

    fn remove_runtime_record(&self, name: &str, kind: &str) {
        rusqlite::Connection::open(self.home.join("settings.sqlite3")).unwrap().execute(
            "DELETE FROM runtime_records WHERE namespace = 'cli-test' AND name = ?1 AND kind = ?2",
            rusqlite::params![name.to_ascii_lowercase(),kind]
        ).unwrap();
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

#[test]
fn opencode_receives_seed_files_and_guidance_before_repository_and_service_startup() {
    for flag in ["--opencode", "-o"] {
        let fixture = Fixture::new();
        let files = fixture.home.join("templates/website/tandem-files");
        fs::create_dir(&files).unwrap();
        fs::write(files.join("seed.txt"), "seed content").unwrap();
        fs::write(
            fixture.home.join("templates/website/tandem-agents.md"),
            "## Template workflow\n\nRead seed.txt before working.\n",
        )
        .unwrap();
        let git_path = Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .unwrap();
        assert!(git_path.status.success());
        let git_path = String::from_utf8(git_path.stdout).unwrap();
        fs::write(
            fixture.bin.join("git"),
            format!(
                "#!/bin/sh\ncase \" $* \" in *' clone '*) test -f \"$TANDEM_HOME/opened\" || exit 29; test -f \"$TANDEM_HOME/workspaces/review/seed.txt\" || exit 30;; esac\nexec '{}' \"$@\"\n",
                git_path.trim().replace('\'', "'\\''"),
            ),
        )
        .unwrap();
        fs::set_permissions(fixture.bin.join("git"), fs::Permissions::from_mode(0o755)).unwrap();
        let output = wait_output(
            fixture
                .command(&["new-instance", "review", "-t", "website", flag])
                .env("EXPECT_OPEN", "1")
                .env("EXPECT_PREPARED_GUIDANCE", "1")
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
            fs::read_to_string(workspace.join("seed.txt")).unwrap(),
            "seed content"
        );
        assert_eq!(
            fs::read_to_string(fixture.home.join("opened")).unwrap(),
            format!("{}\n", workspace.display())
        );
        assert_eq!(
            git(&workspace.join("app"), &["branch", "--show-current"]),
            "review"
        );
        let guidance = fs::read_to_string(workspace.join("AGENTS.md")).unwrap();
        assert!(guidance.contains("./app/AGENTS.md"));
        assert!(!guidance.contains("## Preparation"));
        assert!(String::from_utf8_lossy(&output.stdout).contains("Instance review ready"));
    }
}

#[test]
fn initial_prompts_reach_the_opencode_client_as_literal_text() {
    let fixture = Fixture::new();
    let prompt = "Explain 'this' \"project\"; $(touch injected)\nsecond line";

    let output = fixture.run(&[
        "new-instance",
        "review",
        "-t",
        "website",
        "-o",
        prompt,
        "-d",
        "Review environment",
    ]);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(fixture.home.join("opened-parameters")).unwrap(),
        prompt
    );
    assert!(!fixture.home.join("workspaces/review/injected").exists());
}

#[test]
fn creation_without_opencode_prepares_the_workspace_without_opening_a_client() {
    let fixture = Fixture::new();
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
fn repository_failures_preserve_the_early_client_and_block_container_startup() {
    for missing in ["template", "repository", "second-repository"] {
        let fixture = Fixture::new();
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
        let output = fixture.run(&["new-instance", "review", "-t", template, "-o"]);
        assert!(!output.status.success());
        assert_eq!(fixture.home.join("opened").exists(), missing != "template");
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
fn opencode_opens_a_prepared_workspace_without_repositories() {
    let fixture = Fixture::new();
    fs::write(fixture.home.join("templates/website/tandem.json"), "{}").unwrap();
    let output = wait_output(
        fixture
            .command(&["new-instance", "review", "-t", "website", "-o"])
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
        format!("{}\n", fixture.home.join("workspaces/review").display())
    );
}

#[test]
fn workspace_guidance_failure_blocks_client_repository_and_container_startup() {
    let fixture = Fixture::new();
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
    let output = fixture.run(&["new-instance", "review", "-t", "website", "-o"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("unknown workspace AGENTS.md template placeholder")
    );
    assert!(!fixture.home.join("opened").exists());
    assert!(!fixture.home.join("started").exists());
    assert!(!fixture.home.join("workspaces/review/app").exists());
    assert!(!fixture.home.join("workspaces/review/AGENTS.md").exists());
}

#[test]
fn client_guidance_edits_are_preserved_during_repository_and_service_preparation() {
    let fixture = Fixture::new();
    let output = wait_output(
        fixture
            .command(&["new-instance", "review", "-t", "website", "-o"])
            .env("EDIT_GUIDANCE", "1")
            .spawn()
            .unwrap(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(fixture.home.join("workspaces/review/AGENTS.md")).unwrap(),
        "Client guidance\n"
    );
    assert!(
        fixture
            .home
            .join("workspaces/review/app/file.txt")
            .is_file()
    );
    assert!(fixture.home.join("started").is_file());
}

#[test]
fn an_updated_binary_uses_its_bundled_template_for_new_instances_and_preserves_existing_guidance() {
    let fixture = Fixture::new();
    let guidance = "## Template guidance\n\nUse {{literal}} fixtures.\n";
    fs::write(
        fixture.home.join("templates/website/tandem-agents.md"),
        guidance,
    )
    .unwrap();
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
    assert!(text.starts_with("# review\n"));
    assert!(
        text.contains("docker compose -p 'cli-test-review' exec -T -w CODE_PATH SERVICE COMMAND")
    );
    assert!(text.contains("Read the agents.md files listed above before starting any work."));
    assert!(text.contains("./app/AGENTS.md"));
    assert!(text.ends_with(guidance));
    assert_eq!(
        fs::read_to_string(existing).unwrap(),
        "Existing instance guidance\n"
    );
}

#[test]
fn startup_failure_is_reported_after_early_open_and_preserves_checkout() {
    let fixture = Fixture::new();
    let output = wait_output(
        fixture
            .command(&["new-instance", "review", "-t", "website", "-o"])
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
fn opencode_launch_failure_reports_a_ready_instance_and_preserves_its_workspace() {
    let fixture = Fixture::new();
    let output = wait_output(
        fixture
            .command(&["new-instance", "review", "-t", "website", "-o"])
            .env("FAIL_OPENCODE", "1")
            .spawn()
            .unwrap(),
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("instance is ready, but OpenCode launch failed")
    );
    assert!(
        fixture
            .home
            .join("workspaces/review/app/file.txt")
            .is_file()
    );
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
    let arguments = vec!["delete-instance", "review", "--headless"];
    let output = wait_output(
        fixture
            .command(&arguments)
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
