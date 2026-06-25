# Enable the `stripcomments` clean filter for this clone so that *.rs comments
# are removed from the copy stored in git (see .gitattributes). Run once per
# clone. Requires `python` on PATH.
$ErrorActionPreference = "Stop"
$root = (git rev-parse --show-toplevel).Trim()
git config filter.stripcomments.clean "python `"$root/tools/strip_rs_comments.py`""
# Identity on checkout — comments are not (and cannot be) re-added.
git config filter.stripcomments.smudge cat
Write-Host "Enabled stripcomments clean filter. Apply to all tracked files with:"
Write-Host "    git add --renormalize ."
