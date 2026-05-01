gen-circleci-orb generate \
  --binary "<< parameters.binary >>" \
  --namespace "<< parameters.namespace >>" \
  <<# parameters.output >>--output "<< parameters.output >>"<</ parameters.output >> \
  <<# parameters.install_method >>--install-method "<< parameters.install_method >>"<</ parameters.install_method >> \
  <<# parameters.base_image >>--base-image "<< parameters.base_image >>"<</ parameters.base_image >> \
  <<# parameters.home_url >>--home-url "<< parameters.home_url >>"<</ parameters.home_url >> \
  <<# parameters.source_url >>--source-url "<< parameters.source_url >>"<</ parameters.source_url >> \
  <<# parameters.orb_dir >>--orb-dir "<< parameters.orb_dir >>"<</ parameters.orb_dir >> \
  <<# parameters.dry_run >>--dry-run<</ parameters.dry_run >>