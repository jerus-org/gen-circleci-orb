# Advanced configuration guide

The [Configuration Guide](configuration-guide.md) covers the `gen-circleci-orb.toml` that
`init` writes and the simple ways to tune generation. This guide covers the advanced feature
that goes beyond one-job-per-subcommand: **composing a single, goal-oriented job** from the
tool's own commands plus other steps.

Read this when the default orb is not the shape your consumers want.

## Three ways to use a generated orb

1. **Simple / default.** `generate` emits one command and one job per subcommand. For a simple
   CI this is adequate and sufficient — consumers add the orb and use the jobs directly.

2. **Compose in the consumer's own config.** The generated **commands** are reusable, so a
   consumer can hand-wire several of them into a custom job, or sequence the generated jobs in a
   workflow with standard CircleCI features (`requires`, pre-/post-steps, approval gates). The
   composition effort then lives in *every* consumer's config.

3. **Advanced config — consolidate once.** The orb **owner** moves that composition into
   `gen-circleci-orb.toml`, so the orb ships a single complex job. Every consumer gets one clean
   job instead of re-wiring the steps themselves.

This guide is about the third option.

## When advanced composition is worth it

Reach for a composed job when either is true:

- **The work is one activity to the end-consumer**, not a visible series of steps. If a consumer
  thinks "generate and publish my MCP server" rather than "prime, then generate, then compile,
  then publish, then commit back," a single job matches how they think.
- **The job must combine, at job level, with steps the tool does not provide** — checking out
  the repo, attaching a workspace, securing generated artifacts back into the repo, or publishing
  them to a GitHub release. These live in the same job so they share the same checkout, workspace,
  and credentials.

### The price: you own the construct

A composed job is **not** derived from `--help`. When you author one you take on its maintenance:
its parameters, its step order, and the values wired into each step are yours to keep correct as
the underlying tool evolves. The simple per-subcommand jobs self-maintain (they regenerate from
`--help` on every build); a composed job does not. That trade — owner effort once, in exchange for
a simpler surface for every consumer — is the whole point, but it is a real cost. Prefer the
simple jobs until a genuine single-activity or combine-with-critical-steps need appears.

## Composed jobs: `[[job_group]]`

A `[[job_group]]` assembles a job from the tool's generated **commands** (not its jobs) plus
optional built-ins, third-party-orb steps, and custom `run` steps. There are two authoring modes.

### Simple mode

List command names in `steps`; shared parameters are auto-detected and wired through. This is
what `gen-circleci-orb config add-job-group` writes.

```toml
[[job_group]]
name = "check_and_report"
description = "Validate then report in one job."
steps = ["validate", "report"]
# params = ["orb_path"]   # optional: pin the shared parameter set explicitly
```

### Rich mode

Declare the job's parameters explicitly and give an ordered list of steps. Use this to build a
goal-oriented job. When `step` is present it takes precedence over `steps`.

**Parameters** — one `[[job_group.parameter]]` per job-level parameter:

```toml
[[job_group.parameter]]
name = "orb_path"
param_type = "string"          # default: string
default = "orb/src/@orb.yml"
description = "Path to the orb source @orb.yml."
```

**Steps** — one `[[job_group.step]]` each, in order. Exactly one discriminant field per step:

| Field | Meaning |
|-------|---------|
| `builtin = "checkout"` | A built-in step (`checkout`, `attach_workspace`) |
| `command = "prime"` | Invoke one of the tool's generated commands. Values come from `[job_group.step.with]`; omit `with` to wire parameters through by name |
| `orb = "some-orb/setup"` | Invoke a third-party orb command, values from `[job_group.step.with]` |
| `run = "Set up git"` + `script` + `[job_group.step.environment]` | A custom `run` step: `run` is the step name, `script` the shell body, `environment` the env block |

Two step targets are worth knowing about:

- **`builtin = "attach_workspace"`** also injects two job parameters automatically —
  `attach_workspace` (boolean) and `workspace_root` (string, also prepended to `PATH`). That is why
  the example below references `<< parameters.workspace_root >>` without declaring it.
- **`command = "set_https_remote"`** is available whenever `[orb].git_push_subcommands` is set (see
  the [Configuration Guide](configuration-guide.md#git_push_subcommands--subcommands-that-push-to-git)).
  You do not author it — declaring a push subcommand generates the shared command, and a composite
  can then reference it to prepare the git remote before its own push step.

## Worked example: gen-orb-mcp's `build_mcp_server`

[gen-orb-mcp](https://circleci.com/developer/orbs/orb/jerus-org/gen-orb-mcp) publishes an orb
whose headline job, `build_mcp_server`, has **no matching CLI subcommand**. It is a rich
`[[job_group]]` that primes prior-version snapshots, generates and compiles the MCP server,
publishes the binary to a GitHub release, and commits the generated artifacts back — one job the
consumer runs once. It is assembled from gen-orb-mcp's own commands (`prime`, `generate`,
`publish`, `save`) plus a couple of built-ins, the `set_https_remote` command, and a small setup
script.

> Identifiers below that are not visible on the public registry — context names, secret env-var
> names, docker namespaces — are shown as pseudonyms (`MY_…`, `my-…`). Substitute your own.

```toml
[[job_group]]
name = "build_mcp_server"
description = "Prime, generate, compile, publish and commit back the MCP server."

[[job_group.parameter]]
name = "binary_name"
description = "Orb binary name; names the generated MCP server and release asset."

[[job_group.parameter]]
name = "tag_prefix"
description = "Git tag prefix for VERSION extraction and prime scoping (e.g. my-tool-v)."

[[job_group.parameter]]
name = "orb_path"
default = "orb/src/@orb.yml"
description = "Path to the orb source @orb.yml."

[[job_group.parameter]]
name = "earliest_version"
description = "Earliest orb version to include when priming prior-version snapshots."

# --- ordered steps ---

[[job_group.step]]
builtin = "checkout"

[[job_group.step]]
builtin = "attach_workspace"          # injects attach_workspace + workspace_root parameters

[[job_group.step]]
command = "set_https_remote"          # exists because save is a push subcommand (see Config Guide)

[[job_group.step]]
run = "Set up git and environment"
script = '''
# Prefer the freshly-built binary if the workspace attached one, else use the image's.
if [[ -f "${WORKSPACE_BIN_PATH}/${NAME}" ]]; then
  chmod +x "${WORKSPACE_BIN_PATH}/${NAME}"
  echo "export PATH=${WORKSPACE_BIN_PATH}:\$PATH" >> "$BASH_ENV"
fi
echo "export VERSION=${CIRCLE_TAG#${TAG_PREFIX}}" >> "$BASH_ENV"
'''

[job_group.step.environment]
NAME = "<< parameters.binary_name >>"
TAG_PREFIX = "<< parameters.tag_prefix >>"
WORKSPACE_BIN_PATH = "<< parameters.workspace_root >>"

[[job_group.step]]
command = "prime"

[job_group.step.with]
orb_path = "<< parameters.orb_path >>"
tag_prefix = "<< parameters.tag_prefix >>"
earliest_version = "<< parameters.earliest_version >>"

[[job_group.step]]
command = "generate"

[job_group.step.with]
format = "binary"
generate_name = "<< parameters.binary_name >>"
orb_path = "<< parameters.orb_path >>"
output = "/tmp/mcp-server"
force = "true"
tag_prefix = "<< parameters.tag_prefix >>"

[[job_group.step]]
command = "publish"

[job_group.step.with]
publish_name = "<< parameters.binary_name >>"
input = "/tmp/mcp-server"

[[job_group.step]]
command = "save"

[job_group.step.with]
paths = "prior-versions,migrations"
sign = "true"
```

The consumer invokes it as a single job:

```yaml
- gen-orb-mcp/build_mcp_server:
    binary_name: gen-orb-mcp
    tag_prefix: my-tool-v
    earliest_version: "1.0.0"
    context: [my-release-context]      # supplies signing + release credentials
```

Everything a consumer would otherwise wire by hand — checkout, workspace, git setup, and the four
tool commands with their exact `with:` values — is collapsed into one job the owner maintains.

## Silence the jobs a consumer should not run alone

Once you ship a composite, whether to *also* expose a subcommand's standalone job comes down to
one test: **could a consumer validly combine the jobs in a way the composite does not already
offer?** If yes, expose it. If no, suppressing it keeps the surface clean and stops a consumer
wiring a workflow that cannot work.

Suppress a job with `generate_job = false` — this drops the standalone **job** but **keeps the
command** (and its script), so the composite can still use it:

```toml
[subcommand.prime]
generate_job = false            # only meaningful as a step of build_mcp_server

[subcommand.publish]
generate_job = false            # only meaningful as a step of build_mcp_server

[subcommand.save]
generate_job = false            # only meaningful as a step of build_mcp_server

[subcommand.build]
generate_job = false            # redundant: generate --format binary already compiles
```

`prime`, `publish`, and `save` are plumbing — they only make sense as steps of `build_mcp_server`.
`build` fails the test for a different reason: it is not a step of the composite at all (which
compiles via `generate --format binary`), and `generate --format binary` already produces a binary
on its own, so a separate `build` job adds no distinct scenario. What survives is the clean surface
gen-orb-mcp actually ships: the `build_mcp_server` composite plus `generate`, `validate`, `diff`,
and `migrate`.

Suppressing `save`'s job does **not** break the composite: `generate_job = false` keeps the `save`
command, and `set_https_remote` is generated from `[orb].git_push_subcommands` regardless of
whether the `save` job exists. (Reach for `interactive = true` instead only when you want to drop
the command and script as well — which a composite referencing that command could not then use.)

## Curated step names: `[subcommand.*] label`

Inside a composed job the run-step names default to each command's short `--help` line, then its
bare name. Set a `label` for readable step names:

```toml
[subcommand.prime]
label = "Prime prior versions and migrations"

[subcommand.generate]
label = "Generate and compile MCP server"
```

## Escape hatches: `[[extra_job]]` and `custom_files`

- **`[[extra_job]]`** embeds a fully hand-written job by raw YAML when even rich composition is not
  enough:

  ```toml
  [[extra_job]]
  name = "smoke_test"
  yaml = """
  executor: default
  steps:
    - checkout
    - run: ./smoke.sh
  """
  ```

- **`custom_files`** authorises hand-authored orb files (commands/jobs/scripts the generator does
  not produce) so the prune step keeps them. By default the generator treats the orb directory as
  entirely its own: the files it should contain are exactly the ones it generates from the tool's
  subcommands plus the ones declared in the config (job groups, extra jobs). Anything else it finds
  in those directories is treated as an orphan and deleted on the next `generate` — so any file you
  hand-write must be listed in `custom_files`, by path relative to the orb root, to survive:

  ```toml
  [orb]
  custom_files = ["src/commands/build_container.yml", "src/jobs/build_container.yml"]
  ```

## See Also

- [Configuration Guide](configuration-guide.md) — the `gen-circleci-orb.toml` basics
- [gen-orb-mcp orb on the CircleCI registry](https://circleci.com/developer/orbs/orb/jerus-org/gen-orb-mcp) — the published orb this example builds
