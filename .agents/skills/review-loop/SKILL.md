---
name: review-loop
description: Use when the user asks for a review loop, review pass with fixes, or to keep fixing until review is clean. The review loop may be requested after implementation work or for an existing diff.
---

# Review loop

Use this workflow when the user asks for repeated review/fix passes until reviewer feedback is clean or clarification is needed.

## Workflow

1. Ensure the requested implementation or existing diff is ready for review.
2. Run the `review` subagent against the current diff only.
3. Read and synthesize the reviewer findings.
4. Validate each finding against the local evidence, repo instructions, and user-approved scope.
   - Fix only findings that are real, applicable, and within scope.
   - Do not blindly implement speculative, incorrect, duplicate, or out-of-scope findings.
   - If a finding is invalid, note why and continue.
5. Fix all valid applicable findings yourself, preserving the user-approved scope.
6. Run focused validation.
7. Repeat the `review` subagent pass.
8. Stop when the reviewer reports no findings, or when a finding requires user/product/architecture clarification.
