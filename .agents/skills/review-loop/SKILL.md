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
4. Fix all valid findings yourself, preserving the user-approved scope.
5. Run focused validation.
6. Repeat the `review` subagent pass.
7. Stop when the reviewer reports no findings, or when a finding requires user/product/architecture clarification.
