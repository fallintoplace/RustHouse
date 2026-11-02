# Required integration gate

The `Rust CI / Integration gate` check combines the required checks. It passes
only when formatting, Clippy, rustdoc, the Linux and macOS Rust jobs, the
Windows smoke test, the Python harness jobs, and the minimum Rust version tests
all pass.

**Repository administrator setup (not handled by the workflow):**

1. Configure a branch ruleset for `main` that requires pull requests.
2. Require the `Rust CI / Integration gate` status check. Select the GitHub
   Actions source, not an untrusted external status reporter.
3. Require a current branch before merging (or use GitHub's merge queue).
4. Prevent bypasses for ordinary collaborators; restrict direct pushes.

The integration gate fails if a required check fails, is skipped, or is
cancelled. Without a branch ruleset, the gate is **advisory** and does not
prevent a direct merge.

Keep the individual jobs and their logs. They explain any failures. A cancelled
run does not prove that a commit passed the checks.

## Latest-main fast verification

Every push to `main` starts a fast Linux check. It builds all workspace
libraries and binaries, then runs the query and storage library unit tests.
A newer push cancels an older run.

This is **not** the full integration gate. Run the `Rust CI` workflow manually,
or use its weekly schedule, for cross-platform tests, Clippy, rustdoc, SQL
smoke checks, analytics harness checks, and minimum Rust version checks.
For a release, the final candidate commit must pass the required checks.
