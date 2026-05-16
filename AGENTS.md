# AGENTS.md

This file is the root operating contract for agents in this repository. Keep root instructions here for repo-wide constraints, and put specialized workflows in `.agents/skills/*/SKILL.md`.

## Research first

- Check whether the request conflicts with repo architecture or standards.
- Ask before broad product, architecture, naming, workflow, scope, security, or policy decisions.
- Prefer cleaner architecture when justified. Do not preserve messy code only to avoid churn.

## Skill routing

Before generating code, inspect the `description` frontmatter for every local skill at `.agents/skills/<name>/SKILL.md`.

## Testing

- Add or update tests for meaningful behavior changes.
- Prefer tests that protect behavior, state transitions, data contracts, parsing, error handling, and regression-prone cases.
- Add focused success and failure coverage for new parsing, validation, policy, persistence, concurrency, or process-control logic.
- Do not add tests that only pin trivial literals or states the type system already makes unrepresentable.
- Keep tests deterministic. Avoid real network calls, wall-clock sleeps, host-specific paths, and order dependence unless the test is explicitly integration-level.

## Documentation

- Update README, examples, and configuration docs when public behavior changes.

## Testing and verification

For code changes, run the narrowest meaningful checks first, then the broader repo checks when available.

Expected checks for strict Rust projects:

1. `cargo fmt --check`
2. `cargo clippy --all-targets --all-features -- -D warnings`
3. `cargo test --all-features`
4. `cargo doc --no-deps`

For documentation-only changes, run a narrower verification when the full check would not add useful signal. State what was run and why.

## Definition of done

Use this as a feedback loop, not a ritual.

1. Verify tests are proportional to the risk: New or changed logic has meaningful success and failure coverage.
2. Search for stale references to changed concepts, names, paths, commands, options, policies, or public APIs.
3. Run proportional verification for the files changed.
4. If a check fails, read the full error, identify the root cause, fix it, and repeat the loop.

## Comments

- Prefer self-documenting code and clear type names.
- Add comments only for non-obvious intent.
- Do not leave commented-out code or redundant narration.
