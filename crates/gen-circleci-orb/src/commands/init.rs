use anyhow::Result;
use std::path::PathBuf;

use crate::{ci_patcher, commands::generate::Generate};

pub const DEFAULT_DOCKER_ORB_VERSION: &str = "3.0.1";
/// The gen-orb-mcp orb version to pin when `--mcp` is enabled.
/// Update this when a new gen-orb-mcp orb release is published.
pub const DEFAULT_GEN_ORB_MCP_ORB_VERSION: &str = "0.1.14";

/// Wire orb generation into an existing repo's CI configuration.
#[derive(Debug, clap::Args)]
pub struct Init {
    /// Name of the binary to introspect (must be on PATH).
    #[arg(long)]
    pub binary: String,

    /// CircleCI namespace(s) to publish the orb under as a public orb (repeatable).
    /// Must be set correctly on first init — visibility cannot be changed after the orb is created.
    #[arg(long = "public-orb-namespace")]
    pub public_orb_namespaces: Vec<String>,

    /// CircleCI namespace(s) to publish the orb under as a private orb (repeatable).
    /// Each listed namespace gets `--private` in its `circleci orb create` command.
    /// Must be set correctly on first init — visibility cannot be changed after the orb is created.
    #[arg(long = "private-orb-namespace")]
    pub private_orb_namespaces: Vec<String>,

    /// Name of the build/validation workflow to patch.
    #[arg(long)]
    pub build_workflow: String,

    /// Name of the release workflow to patch.
    #[arg(long)]
    pub release_workflow: String,

    /// Job in the build workflow that regenerate-orb should require.
    #[arg(long)]
    pub requires_job: Option<String>,

    /// Tag prefix used by `toolkit/release_crate` for the crate (e.g. `gen-orb-mcp-v`).
    /// Used to filter the `orb-release:` workflow trigger in config.yml and to normalise
    /// `CIRCLE_TAG` for `orb-tools/publish`.
    #[arg(long)]
    pub crate_tag_prefix: String,

    /// Job in the release workflow after which the generated release jobs
    /// (build-binary-release, pack-orb-release, build-container, ensure-orb-registered)
    /// should be gated. This is the sole mechanism for specifying where the generated
    /// jobs plug into the existing pipeline topology.
    #[arg(long)]
    pub release_after_job: String,

    /// Output directory for the generated orb source (relative to repo root).
    #[arg(long, default_value = "orb")]
    pub orb_dir: String,

    /// Path to the .circleci/ directory.
    #[arg(long, default_value = ".circleci")]
    pub ci_dir: PathBuf,

    /// circleci/orb-tools version to pin in generated CI.
    #[arg(long, default_value = "12.3.3")]
    pub orb_tools_version: String,

    /// circleci/docker orb version to pin in generated CI.
    #[arg(long, default_value = DEFAULT_DOCKER_ORB_VERSION)]
    pub docker_orb_version: String,

    /// Docker Hub (or registry) namespace for the built container image.
    #[arg(long)]
    pub docker_namespace: String,

    /// CircleCI context name holding Docker Hub credentials.
    #[arg(long, default_value = "docker-credentials")]
    pub docker_context: String,

    /// CircleCI context name holding orb publishing credentials.
    #[arg(long, default_value = "orb-publishing")]
    pub orb_context: String,

    /// Version of the jerus-org/gen-circleci-orb orb to pin in generated CI.
    /// Defaults to the version of this binary (orb and crate are released together).
    #[arg(long, default_value = env!("CARGO_PKG_VERSION"))]
    pub gen_circleci_orb_version: String,

    /// Wire in gen-orb-mcp MCP server generation + publish after orb publish.
    #[arg(long)]
    pub mcp: bool,

    /// jerus-org/gen-orb-mcp orb version to pin when --mcp is enabled.
    #[arg(long, default_value = DEFAULT_GEN_ORB_MCP_ORB_VERSION)]
    pub gen_orb_mcp_version: String,

    /// CircleCI context providing push authority for MCP server publish and save steps.
    /// Only used when --mcp is enabled.
    #[arg(long, default_value = "pcu-app")]
    pub mcp_context: String,

    /// Show planned changes without modifying any files.
    #[arg(long)]
    pub dry_run: bool,
}

impl Init {
    pub fn run(&self) -> Result<()> {
        let namespaces: Vec<String> = self
            .public_orb_namespaces
            .iter()
            .chain(self.private_orb_namespaces.iter())
            .cloned()
            .collect();

        // Step 1: generate orb source files
        tracing::info!("Generating orb source into ./{}", self.orb_dir);
        let gen = Generate {
            binary: self.binary.clone(),
            namespaces: namespaces.clone(),
            output: PathBuf::from("."),
            orb_dir: self.orb_dir.clone(),
            install_method: crate::commands::generate::InstallMethod::Binstall,
            base_image: crate::commands::generate::DEFAULT_BASE_IMAGE.to_string(),
            home_url: None,
            source_url: None,
            dry_run: self.dry_run,
        };
        gen.run()?;

        // Step 2: patch CI configs
        let opts = ci_patcher::PatchOpts {
            binary: self.binary.clone(),
            namespaces,
            docker_namespace: self.docker_namespace.clone(),
            orb_dir: self.orb_dir.clone(),
            build_workflow: self.build_workflow.clone(),
            release_workflow: self.release_workflow.clone(),
            requires_job: self.requires_job.clone(),
            crate_tag_prefix: self.crate_tag_prefix.clone(),
            release_after_job: self.release_after_job.clone(),
            orb_tools_version: self.orb_tools_version.clone(),
            docker_orb_version: self.docker_orb_version.clone(),
            docker_context: self.docker_context.clone(),
            orb_context: self.orb_context.clone(),
            private_namespaces: self.private_orb_namespaces.clone(),
            gen_circleci_orb_version: self.gen_circleci_orb_version.clone(),
            mcp: self.mcp,
            gen_orb_mcp_version: self.gen_orb_mcp_version.clone(),
            mcp_context: self.mcp_context.clone(),
        };

        let summary = ci_patcher::apply_patches(&self.ci_dir, &opts, self.dry_run)?;
        for line in &summary {
            println!("{line}");
        }

        if self.dry_run {
            println!("(dry-run: no files written)");
        } else {
            println!("Done.");
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_DOCKER_ORB_VERSION;

    #[test]
    fn default_docker_orb_version_matches_registry() {
        // The CircleCI registry has circleci/docker@3.0.1 as latest.
        // 3.2.0 does not exist and causes "Cannot find circleci/docker@3.2.0" errors.
        assert_eq!(
            DEFAULT_DOCKER_ORB_VERSION, "3.0.1",
            "DEFAULT_DOCKER_ORB_VERSION must be the registry-available version"
        );
    }
}
