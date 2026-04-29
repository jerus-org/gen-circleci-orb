use anyhow::Result;
use std::path::PathBuf;

use crate::{ci_patcher, commands::generate::Generate};

/// Wire orb generation into an existing repo's CI configuration.
#[derive(Debug, clap::Args)]
pub struct Init {
    /// Name of the binary to introspect (must be on PATH).
    #[arg(long)]
    pub binary: String,

    /// CircleCI namespace(s) to publish the orb under (repeatable).
    #[arg(long = "namespace", required = true)]
    pub namespaces: Vec<String>,

    /// Name of the build/validation workflow to patch.
    #[arg(long)]
    pub build_workflow: String,

    /// Name of the release workflow to patch.
    #[arg(long)]
    pub release_workflow: String,

    /// Job in the build workflow that regenerate-orb should require.
    #[arg(long)]
    pub requires_job: Option<String>,

    /// Job in the release workflow after which build-container should run.
    #[arg(long)]
    pub release_after_job: Option<String>,

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
    #[arg(long, default_value = "3.2.0")]
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

    /// Wire in toolkit/build_mcp_server after orb publish (requires jerus-org/circleci-toolkit).
    #[arg(long)]
    pub mcp: bool,

    /// Show planned changes without modifying any files.
    #[arg(long)]
    pub dry_run: bool,
}

impl Init {
    pub fn run(&self) -> Result<()> {
        let namespace = self.namespaces.first().cloned().unwrap_or_default();

        // Step 1: generate orb source files
        tracing::info!("Generating orb source into ./{}", self.orb_dir);
        let gen = Generate {
            binary: self.binary.clone(),
            namespaces: self.namespaces.clone(),
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
            namespace,
            docker_namespace: self.docker_namespace.clone(),
            orb_dir: self.orb_dir.clone(),
            build_workflow: self.build_workflow.clone(),
            release_workflow: self.release_workflow.clone(),
            requires_job: self.requires_job.clone(),
            release_after_job: self.release_after_job.clone(),
            orb_tools_version: self.orb_tools_version.clone(),
            docker_orb_version: self.docker_orb_version.clone(),
            docker_context: self.docker_context.clone(),
            orb_context: self.orb_context.clone(),
            mcp: self.mcp,
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
