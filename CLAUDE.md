<!--
SPDX-FileCopyrightText: 2026 jerusdp

SPDX-License-Identifier: MIT OR Apache-2.0
-->

# CLAUDE.md

Rules for Claude Code working in this repository.

## Git and pull requests

- Never commit to `main`. Branch from a freshly fetched `origin/main`, named `<type>/<description>` (`feat`, `fix`, `docs`, `chore`, `refactor`). A cloud session works on the branch it was given.
- Never merge, approve or close a pull request, including your own superseded ones: open it, return the URL, comment, and recommend closure.
- Stop and report before any commit, amend or push, and wait for "go", "commit and push" or "commit and pr". After a review or fixes, apply them locally, then stop.
- Stage explicit paths; never `git add -A`.
- Before pushing to a branch that has a PR, check the PR's state: it may have been merged. A `[new branch]` line on push means a merged branch was re-created.
- One PR at a time per workpackage: wait for the merge before starting the next unless told otherwise.
- Never force-push a Renovate branch. A fix needed only because of a Renovate update goes on that PR's branch as a new commit.
- Prefer merge commits to squash, to keep attribution.

## Commits

- Conventional Commits, subject under 50 characters, `git commit -s` for the sign-off.
- Commits are also GPG-signed as the agent's GitHub account, never as the generic `noreply@anthropic.com` identity: pcu's verifier cannot tie that address to a known operator and rejects it. Run `.claude/hooks/agent-signing.sh` at the start of a session and check `git var GIT_AUTHOR_IDENT` before the first commit; see `.claude/hooks/README.md` for the environment variables and the security rules for the dedicated environment.
- A commit made with Claude Code carries these trailers before the sign-off:

  ```
  Generated-by: claude-code/<version> (model: <model-id>; operator: <operator-email>)
  Claude-Session: <session-url>
  ```

  `<version>` is `claude --version`, `<model-id>` the model serving the session, `<operator-email>` the person directing it.
- Never write the literal CI-skip token in a commit or PR message.
