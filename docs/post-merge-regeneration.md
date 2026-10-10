# Post-merge check (`[post_merge_check]`) and release gate

Generated orb source reaches `main` only through a reviewed PR. CI never commits to `main`. Two
opt-in checks make sure what lands on `main`, and what is released from it, is what the pinned
generator produces and passes pack and review:

- **`[post_merge_check]`** runs once after a qualifying PR merges: build the binary,
  `generate --check` against `main`, then pack and review the committed orb. It records nothing
  and pushes nothing. It also lets PR branches that match its patterns (e.g. Renovate's) skip the
  validation workflow's *testing* jobs (`pack-orb`, `review-orb`), since the post-merge check covers
  them. Generation is never skipped.
- **The release gate** (`[ci].release_gate_before`) runs the same check, pack and review in the
  crate release workflow **before its approval**, so a release can't publish a crate whose orb
  would fail.

You manage the binary's source; CI does the regeneration. On every PR branch, including the ones
whose testing is skipped, `regenerate-orb` regenerates the orb and records it on the branch, so the
code owner reviews it with the rest of the PR. **Generation always happens on the PR**: the
post-merge check can't commit to `main`, so a change it would have to generate could only fail
there. Only the *testing* of the generated orb (pack and review) is deferred to the post-merge
chain. Drift reaches `main` only if the recorded commit is absent from the merged branch. When
either check then fails, the fix is a PR branch that CI regenerates and records onto; nobody runs
`generate` by hand. Having CI open that PR itself is tracked as future work (#462, D4).

The design is tracked in [gen-circleci-orb#462](https://github.com/jerus-org/gen-circleci-orb/issues/462).
`[post_merge_check]` replaces `[post_merge_regen]` (see [Migrating](#migrating-from-post_merge_regen)).

## Why check after merge

The validation workflow's `build-binary` → `regenerate-orb` chain regenerates the orb on each PR
branch and, with `[record]` enabled, commits the result back to that branch for review. Two kinds
of PR don't suit that:

- **Dependency bumps (Renovate, Dependabot).** Most don't change the CLI's `--help`, so packing
  and reviewing on every push is redundant work. Regeneration is not: when a bump *does* change
  the orb (the gen-circleci-orb pin, an image digest, a `cargo_tools` pin) the change must be
  generated and recorded on the PR. The regen commit makes Renovate treat the PR as manually
  edited and stop rebasing it (gen-circleci-orb#326), unless the record bot's commit email is
  listed in Renovate's `gitIgnoredAuthors` (see
  [Renovate and the recorded commit](#renovate-and-the-recorded-commit)).
- **Repeated pushes to one PR.** Packing and reviewing on each push repeats the same validation.
  Doing it once, on what actually merged, is enough (see `[ci].test_generation`).

`[post_merge_check]` skips the testing jobs on matching PR branches and tests the result once,
after merge. The residual risk — a dependency that changes the help output without anyone noticing — is
exactly what the fresh-build check catches.

## The post-merge chain

The chain is four managed jobs added to the workflow you name:

1. `post-merge-build-binary` builds the binary fresh. A dependency such as `clap` can change the
   interface `generate` introspects even when the repository's own source is unchanged; only a
   fresh build captures that.
2. `post-merge-check-orb` switches onto `main` (`target_branch: main`) and runs
   `generate --check`: it regenerates in memory, compares against the committed orb and fails on
   any difference. It writes nothing and never records. On a "PR merged" pipeline `checkout`
   lands on the merged PR's head, and this is the only job that switches onto `main`, so it
   persists `main`'s committed orb (unchanged) to the workspace: that is how the next two jobs,
   which can't switch branches, get the exact files the check verified.
3. `post-merge-pack-orb` and `post-merge-review-orb` pack and review that committed orb. They run
   whatever `[ci].test_generation` says: this is the one place each merge's orb is packed and
   reviewed.

Nothing in the chain loads a write key, attaches a signing context or pushes.

**Qualifying-branch guard.** Each job starts with a `pre-steps` guard that compares
`CIRCLE_BRANCH` (still the merged PR's branch at job start) with `branch_patterns` and halts the
job (`circleci-agent step halt`) when it doesn't match. Every job carries its own copy, because a
halted job still reports success and a job that `requires:` it would otherwise run. The guard is
needed because CircleCI doesn't allow `filters: branches:` on a "PR merged" pipeline. With
`branch_patterns = ["*"]` every merge qualifies, so the jobs carry no guard.

**The validation side.** For PR branches matching the skip patterns, the validation workflow's
`pack-orb` and `review-orb` get a branch `filters: ignore` entry (`build-binary` and
`regenerate-orb` never do: generation always runs)
(a bash glob such as `renovate/*` becomes the regex `/^renovate\/.*$/`). Filters stop the job
before a container starts, so a skipped job costs nothing — unlike the `pre-steps` halt, which
still pays for container spin-up.

## Prerequisite: you must configure the CircleCI trigger yourself

`[post_merge_check]` only controls **what jobs run and where**. It can't make CircleCI run your
workflow after a PR merges. That trigger is a CircleCI **project setting**, not repo-committable
YAML:

- [GitHub trigger event options](https://circleci.com/docs/guides/orchestrate/github-trigger-event-options/)
  documents the "PR merged" trigger: Project Settings → "GitHub trigger +" → the "PR merged"
  event.
- [Pipelines and triggers overview](https://circleci.com/docs/guides/orchestrate/pipelines/)
  documents the **Config File Path** field. It defaults to `.circleci/config.yml`; point it at the
  file named in `[post_merge_check].file`.

Set this up **before** adding `[post_merge_check]`, or the generated jobs are correct but nothing
runs them. `init` and `generate` print a reminder with both links while the section is configured.

If you already run a post-merge workflow (for example one that updates `PRLOG.md`), add the chain
to it.

## Configuration

```toml
[post_merge_check]
branch_patterns = ["renovate/*"]        # merged PR branches to check
# skip_branch_patterns = ["renovate/*"] # PR branches whose pack/review jobs are skipped
workflow = "update_prlog"               # workflow (within `file`) to add the chain to
file = "update_prlog.yml"               # CI file containing that workflow; defaults to config.yml
requires = ["update-prlog-on-main"]     # optional; see "Job ordering" below
```

- `branch_patterns`: bash-glob pattern(s) (`*` and `?`) selecting which merged PRs are checked.
  `["*"]` checks every merge.
- `skip_branch_patterns`: bash-glob pattern(s) for PR branches whose validation-workflow *testing*
  jobs (`pack-orb`, `review-orb`) are skipped; generation still runs. Defaults to `branch_patterns`. Set it explicitly when `branch_patterns` is `["*"]`:
  every PR must still be validated somewhere, so `update` refuses a skip pattern that matches every
  branch. `[]` skips nothing.
- `workflow`: the workflow the chain is added to. Created if it doesn't exist.
- `file`: the CI file (relative to the CI directory) containing that workflow. Defaults to
  `config.yml`. For a "PR merged" trigger it is usually a dedicated file, so ordinary
  push-triggered pipelines don't evaluate the workflow.
- `requires`: job name(s) already in `workflow` for the chain's first job to wait on. See below.

`[post_merge_check]` doesn't need `[record]`.

Typical settings:

| Repository | `branch_patterns` | `skip_branch_patterns` |
|---|---|---|
| A consumer whose orb changes with its own CLI | `["renovate/*"]` | (default) |
| gen-circleci-orb itself, where generation logic changes in ordinary PRs | `["*"]` | `["renovate/*"]` |

## Job ordering within the target workflow

By default the chain runs **last**: its first job requires every job already in the workflow, by
effective name (an explicit `name:` override when there is one, else the job reference). The
generator only edits its own jobs, never a job you own.

An override inside an inline flow mapping (`- job: {name: x, ...}`) isn't parsed; that job is
required by its bare reference, which fails loudly (a CircleCI "job not found" error) in the rare
case it matters.

Requiring every job has two drawbacks: a job excluded by its own `filters:` on a trigger stops the
chain running on that trigger, and a job meant to run *after* the chain can't be told apart. Set
`requires` to name only the job(s) that should come first:

```yaml
workflows:
  update_prlog:
    jobs:
      - toolkit/update_prlog:
          name: update-prlog-on-main
      # >>> gen-circleci-orb (managed — edits overwritten by 'gen-circleci-orb update')
      # ... the chain, requires: [update-prlog-on-main] via [post_merge_check].requires
      # <<< gen-circleci-orb
      - toolkit/label:
          name: label-oldest-renovate-pr
          requires: [post-merge-review-orb]   # the chain's last job
```

`update` inserts the managed block right after the last job named in `requires`, so a hand-added
trailing job stays after it. Without `requires`, the auto-detect default would require the
trailing job too; since that job requires the chain, CircleCI would reject the circular
`requires:`.

## Release gate

```toml
[ci]
release_workflow = "release"
release_gate_before = "approve-release"
```

With `release_gate_before` set, `update` manages `release.yml` as well:

- It adds `release-gate-build-binary` → `release-gate-check-orb` (`generate --check` against the
  release commit) → `release-gate-pack-orb` → `release-gate-review-orb` to `release_workflow`,
  plus the `gen-circleci-orb` and `orb-tools` orb pins they need.
- It never edits your own jobs. You wire the gate in once, by adding `release-gate-review-orb` to
  the `requires:` of the job named by `release_gate_before` (normally the approval):

  ```yaml
      - approve-release:
          type: approval
          requires: [calculate-versions, release-gate-review-orb]
  ```

  Until you do, `update` and `update --check` fail, naming the job and the line to write. They
  fail the same way if the named job isn't in the workflow.

The approval then can't be given until the orb about to be released has been checked, packed and
reviewed, and a failure stops the release before anything is published. The gate uses the
generator pinned in CI, run against a freshly built binary; it never records.

If the workflow itself isn't found, `update` warns and adds nothing.

## Migrating from `[post_merge_regen]`

`[post_merge_regen]` relocated regen+record to a post-merge workflow that committed to `main`.
Branch protection rejects those pushes (and pcu used to report them as successful,
jerus-org/pcu#1089), so the regenerated source never landed. It is deprecated:

- It is read as `[post_merge_check]`, with its `branch_patterns` also used as the skip patterns,
  and emits the check-only chain. `update` warns until you rename it.
- Rename the section to `[post_merge_check]`; the keys are the same. Having both is an error.
- `update` replaces the old managed jobs (`post-merge-regenerate-orb`, with `allow_main_record` and
  the signing context) with the check-only chain, and moves the validation-side skip from
  `pre-steps` halts to branch filters.
- If a hand-added job required `post-merge-regenerate-orb`, point it at `post-merge-review-orb`.
- `generate --allow-main-record` (the orb job's `allow_main_record` parameter) is deprecated and
  warns when used.
- `init`'s `--post-merge-regen*` flags are aliases of the `--post-merge-check*` flags.

## Verifying it live

1. Merge a PR on a branch that doesn't match `branch_patterns`: the chain's jobs halt with "Not a
   qualifying branch".
2. Merge a qualifying PR that doesn't change the orb: the chain runs green and nothing is
   committed to `main`.
3. Open a skip-pattern PR that alters generated output (e.g. a generator pin bump):
   `regenerate-orb` regenerates and records the orb on the PR branch, `pack-orb` and `review-orb`
   are skipped, and Renovate keeps updating the PR. After the merge the chain tests the result.
4. Run a release: the gate jobs pass before the approval is offered, then the orb publishes.

## Renovate and the recorded commit

With `[record]` enabled, the regenerated orb is committed onto the PR branch, Renovate's included.
By default Renovate treats a commit by any other author as a manual edit and stops updating the PR.
Add the record bot's commit email (the value of `[record].user_email_env`, which only your CI knows)
to `gitIgnoredAuthors` in each repository's Renovate config:

```json
{ "gitIgnoredAuthors": ["<bot email>"] }
```

Entries may be exact RFC5322 email strings, globs or regexes. `generate` prints a reminder while
auto-record and a Renovate branch pattern are both configured; it can't check the setting itself.
See [Renovate's `gitIgnoredAuthors`](https://docs.renovatebot.com/configuration-options/#gitignoredauthors).

## See also

- [`[record]` — auto-record the regenerated orb](configuration-guide.md#record--auto-record-the-regenerated-orb)
- [Configuration Guide](configuration-guide.md)
- [gen-circleci-orb#462](https://github.com/jerus-org/gen-circleci-orb/issues/462): the design
- [gen-circleci-orb#328](https://github.com/jerus-org/gen-circleci-orb/issues/328): the original
  Renovate-freeze problem
