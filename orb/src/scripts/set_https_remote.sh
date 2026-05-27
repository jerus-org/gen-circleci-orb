# CircleCI's checkout step injects this rule into ~/.gitconfig:
#   url."ssh://git@github.com".insteadOf = https://github.com
# This causes git (and libgit2 used by pcu) to transparently rewrite every
# HTTPS GitHub URL back to SSH, so git remote set-url has no observable effect
# on the effective URL. Remove the rule before setting the remote URLs.
git config --global --unset-all "url.ssh://git@github.com.insteadOf" 2>/dev/null || true
HTTPS_ORIGIN="https://github.com/${CIRCLE_PROJECT_USERNAME}/${CIRCLE_PROJECT_REPONAME}.git"
git remote set-url origin "${HTTPS_ORIGIN}"
git remote set-url --push origin "${HTTPS_ORIGIN}"
