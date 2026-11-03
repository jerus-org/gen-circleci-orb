fp="${SSH_FINGERPRINT#SHA256:}"; fp="${fp#MD5:}"
if [ -n "$fp" ] && { ssh-add -l -E md5 || ssh-add -l; ssh-add -l -E sha256; } 2>/dev/null | grep -qiF -- "$fp"; then
  if ssh-add -d ~/.ssh/id_rsa.pub; then
    echo "Write key loaded: dropped the read-only checkout key from the agent."
  else
    echo "Write key loaded, but the read-only checkout key was not in the agent: nothing to drop."
  fi
else
  echo "Write key not loaded (CircleCI adds no SSH keys to forked pull request builds): keeping the checkout key."
fi
