//! Seam 1: the command layer, for `bli init`: a scripted prompter answering the
//! interview, and a real git checkout to write `bli.toml` into.

mod common;

use std::path::{Path, PathBuf};

use bli::config::Config;
use common::Answer::*;
use common::*;
use tempfile::TempDir;

/// A fresh git checkout with no `bli.toml`.
struct Checkout {
    _dir: TempDir,
    root: PathBuf,
}

impl Checkout {
    fn new() -> Checkout {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join("repo");
        std::fs::create_dir(&root).unwrap();
        git(&root, &["init", "-q"]);
        Checkout { _dir: dir, root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn config_text(&self) -> String {
        std::fs::read_to_string(self.root.join("bli.toml")).unwrap()
    }
}

fn answering(answers: &[Answer]) -> Fakes {
    Fakes {
        prompter: Some(ScriptedPrompter::new(answers)),
        ..Default::default()
    }
}

fn prompter(fakes: &Fakes) -> &ScriptedPrompter {
    fakes.prompter.as_ref().unwrap()
}

#[test]
fn accepting_every_default_writes_a_github_herdr_config_at_the_repo_root() {
    let repo = Checkout::new();
    // host, tracker, parent, on_merge, base, branch, prompt, then confirm the write.
    let fakes = answering(&[Enter, Enter, Enter, Enter, Enter, Enter, Enter, Enter]);

    let out = bli(&["init"], repo.path(), &fakes);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(prompter(&fakes).unused(), vec![]);
    assert_eq!(
        repo.config_text(),
        "\
[lane]
host = \"herdr\"
base = \"main\"
branch = \"ticket/{id}-{slug}\"
prompt = \"/implement {ticket_url}\"

[tracker]
kind = \"github\"
on_merge = \"close\"
"
    );
    let config = Config::load(repo.path()).unwrap();
    assert_eq!(config.tracker.parent, None);
}

#[test]
fn github_answers_are_written_with_the_parent_as_an_issue_number() {
    let repo = Checkout::new();
    let fakes = answering(&[
        Pick("superset"),
        Pick("github"),
        Type("#12"),
        Pick("none"),
        Type("develop"),
        Type("feat/{id}"),
        Type("/build {ticket_url}"),
        Yes,
    ]);

    let out = bli(&["init"], repo.path(), &fakes);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        repo.config_text(),
        "\
[lane]
host = \"superset\"
base = \"develop\"
branch = \"feat/{id}\"
prompt = \"/build {ticket_url}\"

[tracker]
kind = \"github\"
parent = 12
on_merge = \"none\"
"
    );
    let config = Config::load(repo.path()).unwrap();
    assert_eq!(config.tracker.parent.as_deref(), Some("12"));
}

#[test]
fn a_github_parent_that_is_not_an_issue_number_is_refused_and_asked_again() {
    let repo = Checkout::new();
    let fakes = answering(&[
        Enter,
        Pick("github"),
        Type("epic-1"),
        Type("7"),
        Enter,
        Enter,
        Enter,
        Enter,
        Yes,
    ]);

    let out = bli(&["init"], repo.path(), &fakes);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        prompter(&fakes).rejections(),
        vec!["a GitHub parent is an issue number, like 12 or #12"]
    );
    assert!(repo.config_text().contains("parent = 7\n"));
}

#[test]
fn a_branch_pattern_the_config_would_refuse_is_refused_with_its_reason_and_asked_again() {
    let repo = Checkout::new();
    let fakes = answering(&[
        Enter,
        Enter,
        Enter,
        Enter,
        Enter,
        Type("ticket/{slug}"),
        Type("t/{id}-{slug}"),
        Enter,
        Yes,
    ]);

    let out = bli(&["init"], repo.path(), &fakes);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        prompter(&fakes).rejections(),
        vec!["branch pattern `ticket/{slug}` is invalid: it must contain `{id}` exactly once"]
    );
    assert!(repo.config_text().contains("branch = \"t/{id}-{slug}\"\n"));
}

#[test]
fn clickup_answers_are_written_with_a_clickup_table_and_no_token() {
    let repo = Checkout::new();
    let fakes = answering(&[
        Enter,
        Pick("clickup"),
        Type("9012345"),
        Type("901234567"),
        Enter,
        Type("86c1x2y3z"),
        Pick("status"),
        Type("Shipped"),
        Enter,
        Enter,
        Enter,
        Yes,
    ]);

    let out = bli(&["init"], repo.path(), &fakes);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(prompter(&fakes).unused(), vec![]);
    assert_eq!(
        repo.config_text(),
        "\
[lane]
host = \"herdr\"
base = \"main\"
branch = \"ticket/{id}-{slug}\"
prompt = \"/implement {ticket_url}\"

[tracker]
kind = \"clickup\"
parent = \"86c1x2y3z\"
on_merge = { status = \"Shipped\" }

[tracker.clickup]
workspace_id = \"9012345\"
list_id = \"901234567\"
"
    );
    let config = Config::load(repo.path()).unwrap();
    let clickup = config.tracker.clickup.unwrap();
    assert_eq!(clickup.workspace_id, "9012345");
    assert_eq!(clickup.space_id, None);
    assert!(!repo.config_text().contains("token"));
}

const ON_MERGE: &str = "What happens to a ticket when its pull request merges?";

fn pairs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

#[test]
fn the_on_merge_choices_are_the_ones_the_chosen_tracker_supports() {
    let github = answering(&[Enter, Pick("github"), Enter, Enter, Enter, Enter, Enter, No]);
    bli(&["init"], Checkout::new().path(), &github);
    let clickup = answering(&[
        Enter,
        Pick("clickup"),
        Type("1"),
        Enter,
        Enter,
        Enter,
        Enter,
        Enter,
        Enter,
        Enter,
        No,
    ]);
    bli(&["init"], Checkout::new().path(), &clickup);

    assert_eq!(
        prompter(&github).offered(ON_MERGE),
        pairs(&[
            (
                "close",
                "close the issue, if the PR's `Closes #<id>` didn't"
            ),
            ("none", "leave the ticket as it is"),
        ])
    );
    assert_eq!(
        prompter(&clickup).offered(ON_MERGE),
        pairs(&[
            ("close", "move the task to its list's closed status"),
            ("none", "leave the ticket as it is"),
            ("status", "move the task to a status you name"),
        ])
    );
}

#[test]
fn cancelling_at_any_question_or_declining_the_write_leaves_no_config() {
    let scripts: [&[Answer]; 4] = [
        &[Cancel],
        &[Enter, Pick("clickup"), Cancel],
        &[Enter, Enter, Enter, Enter, Enter, Enter, Cancel],
        &[Enter, Enter, Enter, Enter, Enter, Enter, Enter, No],
    ];
    for script in scripts {
        let repo = Checkout::new();
        let fakes = answering(script);

        let out = bli(&["init"], repo.path(), &fakes);

        assert_eq!(out.code, 1, "{script:?}");
        assert_eq!(out.stderr, "error: cancelled; no bli.toml was written\n");
        assert!(!repo.path().join("bli.toml").exists(), "{script:?}");
        assert_eq!(prompter(&fakes).unused(), vec![], "{script:?}");
    }
}

#[test]
fn an_existing_config_is_refused_before_any_question_and_left_as_it_was() {
    let repo = Checkout::new();
    std::fs::write(repo.path().join("bli.toml"), GITHUB).unwrap();
    let fakes = answering(&[]);

    let out = bli(&["init"], repo.path(), &fakes);

    assert_eq!(out.code, 1);
    assert_eq!(
        out.stderr,
        format!(
            "error: {} already exists\nhint: edit it by hand, or run `bli init --force` to replace it\n",
            repo.path().join("bli.toml").display()
        )
    );
    assert_eq!(repo.config_text(), GITHUB);
    assert_eq!(prompter(&fakes).asked(), Vec::<String>::new());
}

#[test]
fn force_replaces_an_existing_config_only_once_the_interview_is_finished() {
    let repo = Checkout::new();
    std::fs::write(repo.path().join("bli.toml"), GITHUB).unwrap();

    let cancelled = answering(&[Enter, Cancel]);
    let out = bli(&["init", "--force"], repo.path(), &cancelled);
    assert_eq!(out.code, 1);
    assert_eq!(repo.config_text(), GITHUB);

    let finished = answering(&[
        Pick("superset"),
        Enter,
        Enter,
        Enter,
        Enter,
        Enter,
        Enter,
        Yes,
    ]);
    let out = bli(&["init", "--force"], repo.path(), &finished);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert!(repo.config_text().contains("host = \"superset\"\n"));
}

#[test]
fn outside_a_git_checkout_init_is_refused_before_any_question() {
    let dir = tempfile::tempdir().unwrap();
    let fakes = answering(&[]);

    let out = bli(&["init"], dir.path(), &fakes);

    assert_eq!(out.code, 1);
    assert_eq!(
        out.stderr,
        "error: not in a git checkout: bli.toml belongs at the root of one\n\
         hint: run `bli init` inside the repo, or `git init` first\n"
    );
    assert!(!dir.path().join("bli.toml").exists());
    assert_eq!(prompter(&fakes).asked(), Vec::<String>::new());
}

#[test]
fn from_a_subdirectory_the_config_is_written_at_the_checkout_root_where_load_finds_it() {
    let repo = Checkout::new();
    let sub = repo.path().join("src/deep");
    std::fs::create_dir_all(&sub).unwrap();
    let fakes = answering(&[Enter, Enter, Enter, Enter, Enter, Enter, Enter, Yes]);

    let out = bli(&["init"], &sub, &fakes);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert!(repo.path().join("bli.toml").is_file());
    assert!(!sub.join("bli.toml").exists());
    Config::load(&sub).unwrap();
}

#[test]
fn without_a_terminal_init_fails_at_once_pointing_at_yes_and_writes_nothing() {
    let repo = Checkout::new();

    let out = bli(&["init"], repo.path(), &Fakes::default());

    assert_eq!(out.code, 1);
    assert_eq!(
        out.stderr,
        "error: bli init asks its questions in a terminal, and this is not one\n\
         hint: run it in a terminal, or run `bli init --yes` to write the defaults for GitHub Issues and herdr\n"
    );
    assert!(!repo.path().join("bli.toml").exists());
}

#[test]
fn yes_writes_the_github_and_herdr_defaults_without_asking() {
    let repo = Checkout::new();

    let out = bli(&["init", "--yes"], repo.path(), &Fakes::default());

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let path = repo.path().join("bli.toml");
    assert_eq!(out.stdout, format!("wrote {}\n", path.display()));
    let written = repo.config_text();
    let interviewed = Checkout::new();
    let defaults = answering(&[Enter, Enter, Enter, Enter, Enter, Enter, Enter, Yes]);
    bli(&["init"], interviewed.path(), &defaults);
    assert_eq!(written, interviewed.config_text());
}

#[test]
fn an_existing_config_is_refused_ahead_of_a_missing_terminal_and_by_yes_without_force() {
    let repo = Checkout::new();
    std::fs::write(repo.path().join("bli.toml"), GITHUB).unwrap();

    for args in [&["init"][..], &["init", "--yes"]] {
        let out = bli(args, repo.path(), &Fakes::default());
        assert_eq!(out.code, 1, "{args:?}");
        assert!(
            out.stderr.contains("already exists"),
            "{args:?}: {}",
            out.stderr
        );
    }
    assert_eq!(repo.config_text(), GITHUB);

    let out = bli(
        &["init", "--yes", "--force"],
        repo.path(),
        &Fakes::default(),
    );
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert!(repo.config_text().contains("[lane]\n"));
}

#[test]
fn a_base_that_is_not_a_branch_name_is_refused_and_asked_again() {
    let repo = Checkout::new();
    let fakes = answering(&[
        Enter,
        Enter,
        Enter,
        Enter,
        Type("release 2"),
        Type("release/2"),
        Enter,
        Enter,
        Yes,
    ]);

    let out = bli(&["init"], repo.path(), &fakes);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        prompter(&fakes).rejections(),
        vec!["base `release 2` is not a branch name git accepts"]
    );
    assert!(repo.config_text().contains("base = \"release/2\"\n"));
}

#[test]
fn the_config_is_previewed_before_asking_to_write_it() {
    let repo = Checkout::new();
    let fakes = answering(&[Enter, Enter, Enter, Enter, Enter, Enter, Enter, No]);

    bli(&["init"], repo.path(), &fakes);

    let asked = prompter(&fakes).asked();
    assert_eq!(asked.last().unwrap(), "Write bli.toml?");
    let preview = format!(
        "{}\n{}",
        repo.path().join("bli.toml").display(),
        "\
[lane]
host = \"herdr\"
base = \"main\"
branch = \"ticket/{id}-{slug}\"
prompt = \"/implement {ticket_url}\"

[tracker]
kind = \"github\"
on_merge = \"close\""
    );
    assert!(
        prompter(&fakes).printed().contains(&preview),
        "{:#?}",
        prompter(&fakes).printed()
    );
}

#[test]
fn the_last_word_is_what_to_do_next_and_clickup_is_told_where_its_token_goes_without_asking() {
    let github_repo = Checkout::new();
    let github = answering(&[Enter, Enter, Enter, Enter, Enter, Enter, Enter, Yes]);
    bli(&["init"], github_repo.path(), &github);
    let clickup_repo = Checkout::new();
    let clickup = answering(&[
        Enter,
        Pick("clickup"),
        Type("9012345"),
        Enter,
        Enter,
        Enter,
        Enter,
        Enter,
        Enter,
        Enter,
        Yes,
    ]);
    bli(&["init"], clickup_repo.path(), &clickup);

    assert_eq!(
        prompter(&github).printed().last().unwrap(),
        &format!(
            "Wrote {}. Next, run `bli doctor` to check the setup",
            github_repo.path().join("bli.toml").display()
        )
    );
    assert_eq!(
        prompter(&clickup).printed().last().unwrap(),
        &format!(
            "Wrote {}. Next, set CLICKUP_API_TOKEN in the environment or in .env (never in bli.toml), then run `bli doctor` to check the setup",
            clickup_repo.path().join("bli.toml").display()
        )
    );
    for question in prompter(&clickup).asked() {
        assert!(!question.to_lowercase().contains("token"), "{question}");
    }
}

#[test]
fn clickup_ids_are_checked_and_a_pasted_task_url_gives_the_parent_id() {
    let repo = Checkout::new();
    let fakes = answering(&[
        Enter,
        Pick("clickup"),
        Enter,
        Type("acme"),
        Type("9012345"),
        Type("list 1"),
        Enter,
        Enter,
        Type("https://app.clickup.com/t/86c1x2y3z"),
        Enter,
        Enter,
        Enter,
        Enter,
        Yes,
    ]);

    let out = bli(&["init"], repo.path(), &fakes);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        prompter(&fakes).rejections(),
        vec![
            "ClickUp needs a workspace ID: the number after app.clickup.com/ in its URLs",
            "`acme` is not a ClickUp ID: those are numbers, like 9012345",
            "`list 1` is not a ClickUp ID: those are numbers, like 9012345",
        ]
    );
    assert!(repo.config_text().contains("parent = \"86c1x2y3z\"\n"));
}

#[test]
fn quotes_backslashes_and_comment_marks_in_answers_survive_the_round_trip() {
    let repo = Checkout::new();
    let prompt = r#"/implement "{ticket_url}" \ # not a comment — ✓"#;
    let status = r#"Done "really" \o/"#;
    let fakes = answering(&[
        Enter,
        Pick("clickup"),
        Type("1"),
        Enter,
        Enter,
        Enter,
        Pick("status"),
        Type(status),
        Enter,
        Enter,
        Type(prompt),
        Yes,
    ]);

    let out = bli(&["init"], repo.path(), &fakes);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let config = Config::load(repo.path()).unwrap();
    assert_eq!(config.lane.prompt, prompt);
    assert_eq!(
        config.tracker.on_merge,
        bli::tracker::OnMerge::Status(status.into())
    );
}

#[test]
fn a_status_name_is_written_without_surrounding_spaces() {
    let repo = Checkout::new();
    let fakes = answering(&[
        Enter,
        Pick("clickup"),
        Type("1"),
        Enter,
        Enter,
        Enter,
        Pick("status"),
        Type("  In review "),
        Enter,
        Enter,
        Enter,
        Yes,
    ]);

    bli(&["init"], repo.path(), &fakes);

    assert!(
        repo.config_text()
            .contains("on_merge = { status = \"In review\" }\n"),
        "{}",
        repo.config_text()
    );
}

#[test]
fn a_cancelled_question_closes_the_interview_itself_and_a_declined_write_says_nothing_was_written()
{
    let cancelled = answering(&[Enter, Cancel]);
    bli(&["init"], Checkout::new().path(), &cancelled);
    let declined = answering(&[Enter, Enter, Enter, Enter, Enter, Enter, Enter, No]);
    bli(&["init"], Checkout::new().path(), &declined);

    assert_eq!(prompter(&cancelled).printed(), vec!["bli init"]);
    assert_eq!(
        prompter(&declined).printed().last().unwrap(),
        "Nothing written"
    );
}

#[test]
fn force_never_writes_through_a_symlink_planted_at_a_temporary_name() {
    let repo = Checkout::new();
    std::fs::write(repo.path().join("bli.toml"), GITHUB).unwrap();
    let outside = repo.path().parent().unwrap().join("victim");
    std::fs::write(&outside, "precious\n").unwrap();
    let planted = repo
        .path()
        .join(format!(".bli.toml.{}.tmp", std::process::id()));
    std::os::unix::fs::symlink(&outside, &planted).unwrap();

    let out = bli(
        &["init", "--yes", "--force"],
        repo.path(),
        &Fakes::default(),
    );

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "precious\n");
    assert!(planted.symlink_metadata().unwrap().is_symlink());
    assert!(repo.config_text().contains("[lane]\n"));
}

#[test]
fn a_failed_force_replace_keeps_the_original_and_leaves_no_temporary_file() {
    let repo = Checkout::new();
    // A directory where bli.toml goes: the rename onto it fails.
    std::fs::create_dir(repo.path().join("bli.toml")).unwrap();
    std::fs::write(repo.path().join("bli.toml/keep"), GITHUB).unwrap();

    let out = bli(
        &["init", "--yes", "--force"],
        repo.path(),
        &Fakes::default(),
    );

    assert_eq!(out.code, 1);
    assert!(
        out.stderr.starts_with("error: could not write"),
        "{}",
        out.stderr
    );
    assert_eq!(
        std::fs::read_to_string(repo.path().join("bli.toml/keep")).unwrap(),
        GITHUB
    );
    let mut left: Vec<String> = std::fs::read_dir(repo.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, vec![".git", "bli.toml"]);
}

#[test]
fn a_workspace_id_is_written_without_surrounding_spaces() {
    let repo = Checkout::new();
    let fakes = answering(&[
        Enter,
        Pick("clickup"),
        Type("  9012345 "),
        Enter,
        Enter,
        Enter,
        Enter,
        Enter,
        Enter,
        Enter,
        Yes,
    ]);

    let out = bli(&["init"], repo.path(), &fakes);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert!(
        repo.config_text().contains("workspace_id = \"9012345\"\n"),
        "{}",
        repo.config_text()
    );
}

#[test]
fn a_malformed_clickup_task_url_is_refused_rather_than_guessed_at() {
    let malformed = [
        "https://app.clickup.com/t/",
        "https://evil.example/app.clickup.com/t/86c1x2y3z",
        "http://app.clickup.com/t/86c1x2y3z",
        "https://app.clickup.com/t/9012345/86c1x2y3z",
    ];
    let mut script = vec![Enter, Pick("clickup"), Type("1"), Enter, Enter];
    script.extend(malformed.iter().map(|url| Type(url)));
    script.extend([
        Type("https://app.clickup.com/t/86c1x2y3z/?comment=1#top"),
        Enter,
        Enter,
        Enter,
        Enter,
        Yes,
    ]);
    let repo = Checkout::new();
    let fakes = answering(&script);

    let out = bli(&["init"], repo.path(), &fakes);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let expected: Vec<String> = malformed
        .iter()
        .map(|url| {
            format!(
                "`{url}` is not a ClickUp task ID: use the <id> in https://app.clickup.com/t/<id>"
            )
        })
        .collect();
    assert_eq!(prompter(&fakes).rejections(), expected);
    assert!(repo.config_text().contains("parent = \"86c1x2y3z\"\n"));
}

#[test]
fn a_blank_agent_prompt_is_refused_and_asked_again() {
    let repo = Checkout::new();
    let fakes = answering(&[
        Enter,
        Enter,
        Enter,
        Enter,
        Enter,
        Enter,
        Type("   "),
        Type("/go {ticket_url}"),
        Yes,
    ]);

    let out = bli(&["init"], repo.path(), &fakes);

    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        prompter(&fakes).rejections(),
        vec!["the agent needs a first prompt, e.g. /implement {ticket_url}"]
    );
    assert!(repo
        .config_text()
        .contains("prompt = \"/go {ticket_url}\"\n"));
}
