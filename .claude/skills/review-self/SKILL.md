# PR Self-Review Validator

Validate your pull request against Anvil quality standards before requesting review. This skill runs automated checks, validates branch state, and ensures your PR meets all requirements before human review.

## When to use

Trigger this skill when:
- You're about to request review on a PR
- You want to validate your changes against Anvil standards
- You need to ensure all quality gates are met before submission

## What this skill does

### Pre-review validation (automated)
- Checks branch is up-to-date with target branch
- Validates no merge conflicts exist
- Verifies CI status (if available)
- Ensures all commits reference a Paperclip issue

### Post-validation fixes (automated)
- Runs `cargo fmt --all` to format code
- Runs `cargo clippy --workspace` to catch common issues
- Checks for TODO comments that should be addressed
- Validates commit message format

### Final checklist generation
- Generates PR checklist based on Anvil standards
- Suggests appropriate PR labels
- Validates PR description links to Paperclip issue
- Confirms all automated review comments have been addressed

## Usage

```bash
/review-self
```

The skill will:
1. Run pre-validation checks
2. Apply automated fixes if needed
3. Generate a completion checklist
4. Report any blocking issues that need manual resolution

## Quality Standards

This skill enforces the Anvil PR workflow protocol:
- Branch created from latest main
- All tests pass locally
- PR description links to Paperclip issue
- All automated review comments addressed
- All CI checks green
- Code formatted with `cargo fmt`
- No clippy warnings
- No temporary debugging code

## Blocking Issues

The skill will block and report if it finds:
- Merge conflicts with target branch
- Failing tests
- Clippy errors
- Missing Paperclip issue reference
- Uncommitted changes that would be lost

## Output

The skill generates a markdown checklist you can paste into your PR description to track completion of all quality gates.
