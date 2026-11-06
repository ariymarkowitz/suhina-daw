---
name: bug-fix-pr
description: Fix a set of bugs from GitHub issues in one pull request - claim them, fix each at its root cause in its own commits, ask the maintainer what needs a decision, review against the specs. Usage `/bug-fix-pr #123 #130`.
disable-model-invocation: true
---

Issues: $ARGUMENTS (none given: ask which).

[`AGENTS.md`](../../../AGENTS.md) and the `working-on-suhina` skill hold the rules for claims,
branches, code and checks; this skill adds how to work through a set of bugs. **The user is the
maintainer**: ask them with `AskUserQuestion`, your recommendation first. Never guess an answer
that is theirs to give.

## 0. Branch

Check whether the current checkout is under development: uncommitted changes, or a branch other
than `main` with commits not on `origin/main`. If it is, leave it untouched and work in a separate
worktree:

```sh
git fetch origin main
git worktree add ../suhina-<short-description> -b fix/<short-description> origin/main
```

Otherwise create `fix/<short-description>` from `origin/main` in place.

## 1. Claim

Read each issue, its comments and linked pull requests, then `/claim` it as
[AGENTS.md › Tasks](../../../AGENTS.md#3-tasks) says. If the bot refuses, says taken or blocked,
or names an open pull request, leave that issue out and tell the user.

## 2. Triage all issues before writing code

- **Bug, or a feature request in disguise?** A bug is Suhina not doing what its spec, its docs
  or the evident intent of the code says. A fix that adds behaviour nothing promises, changes
  specified behaviour or chooses between designs is a feature: ask, do not build it.
- **Root cause, not symptom.** Reproduce with a failing test, then follow the symptom back to the
  code that is wrong. A UI patch over an engine bug, a clamp over a bad calculation or a special
  case for the reported input is a symptom fix. If the issue asks for X but the problem is Y,
  fix Y and say so; issues with one cause get one fix.
- **Read the spec and ADRs** covering the code (`docs/INDEX.md` › *Code and its documents*).
  Note where they are silent, ambiguous or wrong, or disagree with the code.

Then ask, in one batch: features in disguise, spec gaps and conflicts, fixes that change a spec,
ADR, file format or command, and fixes much bigger than their issue.

## 3. Fix one bug at a time

Per bug: failing test, fix, at least one commit of its own naming the issue (`Fix ... (#123)`);
never two bugs in one commit, and the tree builds and passes after each.

The fix should leave the structure better, not just the behaviour: put it in the layer that owns
the logic, reuse what exists, remove the cause of a special case rather than add one, and be
concise where that helps reading, never where it hides intent. Unrelated cleanups stay out;
offer them as follow-ups.

## 4. Review

Read the whole diff (`git diff origin/main...`) as a reviewer, per fix: root cause? would its test
fail without it? matches the spec and ADRs? more readable and modular than before? no shortcut,
feature or unasked change slipped in? Fix findings in new commits, then run the checks.

## 5. One pull request

One pull request for the set from the step 0 branch, a draft from the first fix on. In the
description: `Closes #N` on its own line per issue; per bug, the cause, the fix and its test;
*How to check it* steps for each bug (`how-to-check` skill); and a **For the maintainer** section
with the spec problems or improvements found and what is still open.
