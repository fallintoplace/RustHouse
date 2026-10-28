# Required integration gate

RustHouse has one aggregate `Rust CI / Integration gate` check. It only succeeds
when formatting, Clippy, rustdoc, all Linux/macOS Rust matrix jobs, Windows smoke,
all Python harness matrix jobs, and minimum-Rust-version tests succeed.

**Repository administrator setup (not handled by the workflow):**

1. Configure a branch ruleset for `main` that requires pull requests.
2. Require the `Rust CI / Integration gate` status check. Select the GitHub
   Actions source, not an untrusted external status reporter.
3. Require a current branch before merging (or use GitHub's merge queue).
4. Prevent bypasses for ordinary collaborators; restrict direct pushes.

The workflow reports failed, skipped, or cancelled prerequisites as a failed
aggregate check. Without the branch ruleset, this workflow is **advisory**
and does not prevent a direct merge.

Keep individual jobs and their logs: the aggregate check is a decision signal,
not a replacement for useful failure detail. Do not rely on a cancelled run as
proof that the corresponding commit was verified.
