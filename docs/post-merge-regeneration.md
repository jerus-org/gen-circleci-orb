# Post-merge regeneration (`[post_merge_regen]`)

`[post_merge_regen]` relocates the regen+record job chain — and, when `[ci].test_generation`
is on, the pack/review self-test — off a *qualifying* PR's own branch and into a CI-managed
workflow that runs after merge instead. It exists to fix one specific failure mode: auto-record
freezing a bot-authored PR (Renovate, Dependabot, ...) that it was never meant to touch.

This is an advanced, opt-in feature. Most consumers of `[record]` never need it — read the
[Problem](#the-problem-it-solves) section below to check whether it applies to you before adding
it.

## The problem it solves

With `[record].enabled = true`, the `build-binary` → `regenerate-orb` chain runs on every
non-`main` branch, including a Renovate branch. When regeneration produces a diff — for
example, a Renovate-driven bump to `[orb].circleci_cli_version` or a pinned image digest changes
what the generated orb looks like — `regenerate-orb` GPG-signs and pushes a
`chore: regenerate orb` commit straight onto that branch.

Renovate treats any commit it did not author on its own branch as manual intervention, and
**permanently stops rebasing or re-resolving that PR**. The fix (bumping the offending pin
again) triggers another auto-record push, so the PR stays stuck. This was reproduced on
gen-circleci-orb#326 with a stale `circleci-cli` pin.

The regen itself is not optional — see [`[record]`](configuration-guide.md#record--auto-record-the-regenerated-orb)
and [gen-circleci-orb#382](https://github.com/jerus-org/gen-circleci-orb/issues/382): it is the
only mechanism keeping `orb/src` in sync with the CLI and reviewable pre-merge. Simply skipping
auto-record on bot branches would silently reintroduce that bug, with no replacement path for
the change to land. `[post_merge_regen]` instead moves the same chain to a place where it can
never touch a live bot branch: a workflow that runs on `main`, after the PR has already merged.

## The mechanism

1. **Qualifying-branch guard.** Every relocated job's first step is a `pre-steps` guard that
   inspects `CIRCLE_BRANCH` against `branch_patterns` (a bash `case` statement) and halts the
   job (`circleci-agent step halt`) — a fast no-op — when it does not match. Every relocated job
   carries its own copy of this guard, not just the first: a halted job is still reported
   successful, so a job `requires:`-ing it would otherwise run anyway.

   This guard is necessary because CircleCI blocks ordinary `filters: branches:` entirely on a
   "PR merged"-triggered pipeline — see [Prerequisite](#prerequisite-you-must-configure-the-circleci-trigger-yourself)
   below. `CIRCLE_BRANCH` is still reliably set to the merged PR's original branch name at job
   start, before anything overrides it, which is what makes the pattern match possible.

2. **Switching onto the real target branch.** On a "PR merged" pipeline, `checkout` lands on the
   now-deleted PR branch, and `CIRCLE_BRANCH` stays stale at that name even after `checkout`
   runs. The relocated `generate` job's `target_branch` parameter (set to `main`) adds a step
   right after `checkout` that does the plain-git equivalent of switching branches — fetch,
   checkout, and an explicit `CIRCLE_BRANCH` override in `$BASH_ENV` — so the generate invocation,
   and any commit it makes, targets the right branch.

3. **Recording on `main`, deliberately and narrowly.** `generate`'s auto-record logic normally
   refuses to push to `main` at all — a push there would otherwise need a branch-protection
   bypass this tool deliberately does not use. The relocated chain passes `--allow-main-record`
   (only after step 2 has switched onto `main` itself), a narrow, explicit opt-in that skips
   *only* the `main` exclusion. The forked-PR and empty-branch exclusions stay unconditional, and
   an ordinary `generate --record` run (in `validation`, or a developer's local run) never sees
   this flag.

4. **A fresh binary build, every time.** The relocated chain always compiles a fresh binary
   (`build-binary`) rather than reusing the container's own published CLI. A Renovate bump to a
   dependency shaping the binary's `--help` output — `clap` itself, or anything else — can change
   the interface `generate` introspects, even though the repository's own source is untouched by
   a dependency-only PR. Only a fresh build reliably captures that.

## Prerequisite: you must configure the CircleCI trigger yourself

`[post_merge_regen]` only controls **what jobs run and where** — it does not, and cannot, make
CircleCI actually invoke your chosen workflow/file after a PR merges. That trigger is a CircleCI
**project setting**, not repo-committable YAML:

- [GitHub trigger event options](https://circleci.com/docs/guides/orchestrate/github-trigger-event-options/)
  documents the "PR merged" trigger itself: Project Settings → "GitHub trigger +" → choose the
  "PR merged" event.
- [Pipelines and triggers overview](https://circleci.com/docs/guides/orchestrate/pipelines/)
  documents the accompanying **Config File Path** field — it defaults to `.circleci/config.yml`,
  but you can point it at a different file (matching `[post_merge_regen].file` below).

If you have never set this up before, do it **before** adding `[post_merge_regen]` — otherwise
the generated jobs are correct but nothing ever triggers them. `init` and `generate` print a
reminder with both links above the first time they see `[post_merge_regen]` configured.

Some organizations already run a post-merge workflow for other reasons (this org's own
`update_prlog.yml`, for instance, updates `PRLOG.md`) and can simply add the relocated chain to
that existing workflow/file. If you don't already have one, you will need to create it and wire
up the trigger yourself first.

## Configuration

```toml
[post_merge_regen]
branch_patterns = ["renovate/*"]     # bash-glob branch name pattern(s) that qualify
workflow = "update_prlog"            # workflow (within `file`) to add the relocated jobs to
file = "update_prlog.yml"            # CI file containing that workflow; defaults to config.yml
```

- `branch_patterns` — one or more bash-glob patterns (e.g. `["renovate/*", "dependabot/*"]`).
  Any PR branch matching any pattern qualifies; every other merge is a fast no-op.
- `workflow` — the name of the workflow the relocated jobs are inserted into.
- `file` — the CI file (relative to your CI directory) containing that workflow. Optional;
  defaults to `config.yml`. Only set this when the target workflow lives in a dedicated file, as
  it typically will for a "PR merged" trigger (see the prerequisite above) — you generally do not
  want an ordinary push-triggered pipeline evaluating the same workflow.

`[post_merge_regen]` requires `[record].enabled = true` — `update`/`init` refuse to proceed
otherwise, since there is nothing to relocate without auto-record enabled in the first place.

## Job ordering within the target workflow

A dedicated post-merge workflow commonly exists specifically to push administrative changes to
`main` — this org's own `update_prlog.yml`, for example, also runs `toolkit/update_prlog`, which
pushes a `PRLOG.md` update. To avoid racing that against `post-merge-regenerate-orb` (the only
job in the relocated chain that itself pushes), the generator automatically adds
`requires: [post-merge-regenerate-orb]` to every job **already** in your named workflow, the
first time the relocated chain is inserted — appended to an existing `requires:` list rather than
replacing it, or added fresh if the job has none. This wiring is one-time: if you remove it by
hand afterward, a later `update` will not re-add it.

## Verifying it live

Because this changes where a real GPG-signed push lands, verify it against a real (or
deliberately staged) qualifying merge before trusting it:

1. Confirm a merge on a **non-qualifying** branch leaves the relocated jobs halted (check the
   job logs for "Not a qualifying branch").
2. Confirm a merge on a **qualifying** branch (e.g. a real Renovate PR) runs the relocated chain
   to completion and pushes the regenerated orb to `main`.
3. Confirm a Renovate PR that previously froze under the old (non-relocated) behavior is no
   longer frozen after adopting `[post_merge_regen]`.

## See also

- [`[record]` — auto-record the regenerated orb](configuration-guide.md#record--auto-record-the-regenerated-orb)
- [Configuration Guide](configuration-guide.md)
- [gen-circleci-orb#328](https://github.com/jerus-org/gen-circleci-orb/issues/328) — the issue
  this feature addresses
