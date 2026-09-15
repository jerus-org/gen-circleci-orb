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

5. **The original chain steps aside on a qualifying branch.** `[post_merge_regen]` relocates the
   chain — it does not merely duplicate it. `build-binary`/`regenerate-orb` (and, per
   `[ci].test_generation`, `pack-orb`/`review-orb`) in the ordinary `[ci].build_workflow` (the
   workflow that runs on every push, including a Renovate branch) each carry their own
   `pre-steps` guard: the inverse of the relocated chain's own guard, it halts when
   `CIRCLE_BRANCH` *does* match `branch_patterns`, since that branch is now handled entirely by
   the relocated chain instead. Without this, the original chain would still run — and still push
   a regen commit — on the very branches `[post_merge_regen]` exists to protect, silently
   defeating the whole feature by running both copies. A downstream job (`pack-orb`/`review-orb`)
   needs its own copy of the guard too: a halted job still reports success, so a job that
   `requires:` it would otherwise run anyway — with no workspace ever persisted upstream.

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

Some organizations already run a post-merge workflow for other administrative purposes and can
simply add the relocated chain to that existing workflow/file. If you don't already have one, you
will need to create it and wire up the trigger yourself first.

## Configuration

```toml
[post_merge_regen]
branch_patterns = ["renovate/*"]     # bash-glob branch name pattern(s) that qualify
workflow = "update_prlog"            # workflow (within `file`) to add the relocated jobs to
file = "update_prlog.yml"            # CI file containing that workflow; defaults to config.yml
requires = ["update-prlog-on-main"]  # optional; see "Job ordering" below
```

- `branch_patterns` — one or more bash-glob patterns (e.g. `["renovate/*", "dependabot/*"]`).
  Any PR branch matching any pattern qualifies; every other merge is a fast no-op.
- `workflow` — the name of the workflow the relocated jobs are inserted into.
- `file` — the CI file (relative to your CI directory) containing that workflow. Optional;
  defaults to `config.yml`. Only set this when the target workflow lives in a dedicated file, as
  it typically will for a "PR merged" trigger (see the prerequisite above) — you generally do not
  want an ordinary push-triggered pipeline evaluating the same workflow.
- `requires` — job name(s) already in `workflow` for the relocated chain's first job to wait on.
  Optional; see "Job ordering" below.

`[post_merge_regen]` requires `[record].enabled = true` — `update`/`init` refuse to proceed
otherwise, since there is nothing to relocate without auto-record enabled in the first place.

## Job ordering within the target workflow

A dedicated post-merge workflow commonly exists specifically to push administrative changes to
`main`. To avoid racing one of those pushes against the relocated chain's own push, the generator
by default makes the relocated chain run **last**: its first job automatically requires every job
already in the workflow, using each job's effective name (an explicit `name:` override when it has
one, else the job reference itself).

This edit is made only on the relocated chain's own job — never by rewriting a pre-existing,
customer-owned job block. Even inside a file this feature otherwise manages, editing someone
else's job is out of scope for a generator.

An override embedded in an inline flow-mapping entry (`- job: {name: x, ...}`) is not parsed; that
job is required by its bare reference instead, which fails loudly (a CircleCI "job not found"
error) rather than silently, in the rare case that matters.

Requiring **every** pre-existing job also means one that is itself excluded by its own `filters:`
on a given trigger silently keeps the relocated chain from running on that trigger too — and it
cannot distinguish a job meant to run *after* the relocated chain from one meant to run before it.
Set `requires` explicitly to name only the job(s) that should precede the relocated chain when
either of these applies, especially when a job in the workflow itself depends on the relocated
chain having already run. A concrete example: a workflow whose first job both updates PRLOG.md
*and* labels the oldest open Renovate PR for rebase (`toolkit/update_prlog`'s `run_label`) should
not let that label fire before the relocated chain's own regen commit lands — the labeled PR would
be rebased against a `main` that's about to change again, one commit stale. The fix is to split
the two concerns and order them correctly:

```yaml
workflows:
  update_prlog:
    jobs:
      - toolkit/update_prlog:
          name: update-prlog-on-main
          run_label: false   # disable the built-in (premature) label step
          # ... other params unchanged
      # >>> gen-circleci-orb (managed — edits overwritten by 'gen-circleci-orb update')
      # ... the relocated chain, requires: [update-prlog-on-main] via [post_merge_regen].requires
      # <<< gen-circleci-orb
      - toolkit/label:
          name: label-oldest-renovate-pr
          requires: [post-merge-regenerate-orb]   # the chain's own last job
```

`update` inserts the managed block immediately after the last job named in `[post_merge_regen].requires`
— not necessarily the absolute end of the workflow's `jobs:` list. With `requires` set as above, the
block lands right after `update-prlog-on-main`, so a hand-added trailing job like `toolkit/label` stays
positioned after the block, exactly as written. `update --check` enforces this exact position — an
unrelated job placed even later in the file (not named in `requires`) is left untouched wherever it is.

With `[post_merge_regen].requires = ["update-prlog-on-main"]` set, the relocated chain's first job
waits only on `update-prlog-on-main` — never on `label-oldest-renovate-pr`, even though it's also
"already in the workflow." Without `requires` set, the auto-detect default would instead require
*both* jobs, including the trailing one — and since `label-oldest-renovate-pr` itself requires the
relocated chain's last job, that produces a circular `requires:` CircleCI rejects outright.

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
