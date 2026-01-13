use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{LazyLock, Mutex, MutexGuard};
use tempfile::TempDir;

static CWD_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

pub(crate) struct DirGuard {
    original: PathBuf,
    _lock: MutexGuard<'static, ()>,
}

impl DirGuard {
    pub(crate) fn new(new_dir: &Path) -> Self {
        // Changing the process current working directory is global and not thread-safe.
        // Lock it so tests don't race even if a #[serial] annotation is missed.
        let lock = CWD_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(new_dir).unwrap();
        Self {
            original,
            _lock: lock,
        }
    }
}

impl Drop for DirGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original);
    }
}

pub(crate) fn create_test_repo() -> TempDir {
    create_repo(CreateRepoOptions {
        commits: 1,
        add_origin_remote: false,
    })
}

pub(crate) fn create_test_repo_with_remote() -> TempDir {
    create_repo(CreateRepoOptions {
        commits: 2,
        add_origin_remote: true,
    })
}

struct CreateRepoOptions {
    commits: usize,
    add_origin_remote: bool,
}

fn create_repo(opts: CreateRepoOptions) -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    let path = temp_dir.path();

    git(path, &["init"]);
    // Ensure the repo uses a deterministic default branch name across environments.
    // This sets HEAD to an unborn `main` branch before the first commit.
    git(path, &["symbolic-ref", "HEAD", "refs/heads/main"]);

    // Configure git user for commits
    git(path, &["config", "user.email", "test@example.com"]);
    git(path, &["config", "user.name", "Test User"]);

    // Create initial commit (required for worktree creation)
    std::fs::write(path.join("README.md"), "# Test\n").unwrap();
    git(path, &["add", "."]);
    git(path, &["commit", "-m", "Initial commit"]);

    // Optional extra commits for tests that need a non-trivial history.
    for i in 2..=opts.commits {
        std::fs::write(path.join(format!("file{}.txt", i)), format!("File {}\n", i)).unwrap();
        git(path, &["add", "."]);
        git(path, &["commit", "-m", &format!("Commit {}", i)]);
    }

    if opts.add_origin_remote {
        // Add remote pointing to itself (simulates a remote for fetch in tests).
        let path_str = path.to_string_lossy().to_string();
        git(path, &["remote", "add", "origin", &path_str]);
    }

    temp_dir
}

fn git(repo_dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(repo_dir)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to execute git {}: {}", args.join(" "), e));

    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!(
            "git {} failed (exit code {:?})\nstdout:\n{}\nstderr:\n{}",
            args.join(" "),
            output.status.code(),
            stdout,
            stderr
        );
    }
}
