gen-circleci-orb init \
  --binary "<< parameters.binary >>" \
  --namespace "<< parameters.namespace >>" \
  --build-workflow "<< parameters.build_workflow >>" \
  --release-workflow "<< parameters.release_workflow >>" \
  <<# parameters.requires_job >>--requires-job "<< parameters.requires_job >>"<</ parameters.requires_job >> \
  <<# parameters.release_after_job >>--release-after-job "<< parameters.release_after_job >>"<</ parameters.release_after_job >> \
  <<# parameters.orb_dir >>--orb-dir "<< parameters.orb_dir >>"<</ parameters.orb_dir >> \
  <<# parameters.ci_dir >>--ci-dir "<< parameters.ci_dir >>"<</ parameters.ci_dir >> \
  <<# parameters.orb_tools_version >>--orb-tools-version "<< parameters.orb_tools_version >>"<</ parameters.orb_tools_version >> \
  <<# parameters.docker_orb_version >>--docker-orb-version "<< parameters.docker_orb_version >>"<</ parameters.docker_orb_version >> \
  --docker-namespace "<< parameters.docker_namespace >>" \
  <<# parameters.docker_context >>--docker-context "<< parameters.docker_context >>"<</ parameters.docker_context >> \
  <<# parameters.orb_context >>--orb-context "<< parameters.orb_context >>"<</ parameters.orb_context >> \
  <<# parameters.mcp >>--mcp<</ parameters.mcp >> \
  <<# parameters.dry_run >>--dry-run<</ parameters.dry_run >>