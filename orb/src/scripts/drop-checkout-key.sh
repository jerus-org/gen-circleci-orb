fp="${SSH_FINGERPRINT#SHA256:}"; fp="${fp#MD5:}"
if [ -n "$fp" ] && { ssh-add -l -E md5 || ssh-add -l; ssh-add -l -E sha256; } 2>/dev/null | grep -qiF -- "$fp"; then
  ssh-add -d ~/.ssh/id_rsa.pub || true
else
  echo "Write key not loaded (a forked pull request build has none); keeping the checkout key."
fi
