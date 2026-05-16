# AGENTS.md

This file is the root operating contract for agents in this repository. Keep root instructions here for repo-wide constraints, and put specialized workflows in `.agents/skills/*/SKILL.md`.

## Research first

- Check whether the request conflicts with repo architecture or standards.
- Ask before broad product, architecture, naming, workflow, scope, security, or policy decisions.
- Prefer cleaner architecture when justified. Do not preserve messy code only to avoid churn.

## Skill routing

Before generating code, inspect the `description` frontmatter for every local skill at `.agents/skills/<name>/SKILL.md`.

## Rust module size and focus

- Prefer focused Rust modules with one clear responsibility.
- Treat Rust files over roughly 200–400 lines as a prompt to consider splitting, especially when they contain multiple responsibilities.

## Testing

- Add or update tests for behavior you changed.
- Prefer tests that protect behavior, state transitions, data contracts, parsing, error handling, and regression-prone cases.
- Add focused success and failure coverage for new parsing, validation, policy, persistence, concurrency, or process-control logic.
- Do not add tests that only pin trivial literals or states the type system already makes unrepresentable.
- Keep tests deterministic. Avoid real network calls, wall-clock sleeps, host-specific paths, and order dependence unless the test is explicitly integration-level.

## Documentation

- Update README, examples, and configuration docs when public behavior changes.

## Reader-facing text

- Use sentence case where sensible for reader-facing text, including UI text, button labels, command-style actions, and Markdown headings, while preserving proper nouns, acronyms, filenames, package names, and domain terms.

## Definition of done

Use this as a feedback loop, not a ritual.

1. Add or update tests for behavior you changed.
2. Search for stale references to changed concepts, names, paths, commands, options, policies, or public APIs.
3. Run `just check` before marking work done. If it fails, read the full error, fix the root cause, and run it again.

## Comments

- Prefer self-documenting code and clear type names.
- Add comments only for non-obvious intent.
- Do not leave commented-out code or redundant narration.
