set -- gen-circleci-orb set-default
set -- "$@" --subcommand "${SUBCOMMAND}"
set -- "$@" --param "${PARAM}"
set -- "$@" --default "${DEFAULT}"
"$@"
