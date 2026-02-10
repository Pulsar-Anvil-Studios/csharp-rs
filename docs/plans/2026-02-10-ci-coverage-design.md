# CI Coverage Report Design

## Goal

Add code coverage reporting to CI on pull requests and main branch pushes.
Fail CI if coverage drops below 98% (global or diff).

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Coverage tool | `cargo-llvm-cov` | LLVM-native instrumentation, accurate for proc macros |
| Workflow structure | Single job (replace `cargo test` with coverage) | Avoids running tests twice |
| Reporting | Job Summary + PR comment | Maximum visibility |
| PR comment content | Global %, diff %, uncovered lines per changed file | Actionable feedback |
| Threshold scope | Global 98% + diff 98% | Forces coverage of both existing and new code |
| External services | None | Self-contained with `cargo-llvm-cov` + `gh` CLI |

## Architecture

One job in `rust-checks.yml`: fmt, clippy, then instrumented tests with coverage.

### Steps

1. Checkout (full history for `git diff`)
2. Install Rust + `llvm-tools-preview` + `cargo-llvm-cov`
3. Cache cargo
4. `cargo fmt --all -- --check`
5. `cargo clippy --all-targets --all-features -- -D warnings`
6. `cargo llvm-cov --workspace --lcov --output-path lcov.info`
7. `cargo llvm-cov report --fail-under 98` (global threshold, `continue-on-error`)
8. `.github/scripts/diff-coverage.sh` (PR only, `continue-on-error`)
9. Job Summary (always)
10. Final enforcement step (fails if either threshold step failed)

### Files

- `.github/workflows/rust-checks.yml` (modified)
- `.github/scripts/diff-coverage.sh` (new)

## PR Comment Format

```markdown
## Coverage Report

| Metric | Value | Threshold | Status |
|--------|-------|-----------|--------|
| Global | 98.35% | 98% | Pass |
| Diff   | 100%   | 98% | Pass |

### Changed Files

| File | Coverage | Uncovered Lines |
|------|----------|-----------------|
| `crates/.../foo.rs` | 95.0% (19/20) | L42, L67-69 |
```
