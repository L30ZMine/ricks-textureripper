#!/usr/bin/env sh
# Enable the `stripcomments` clean filter for this clone so that *.rs comments
# are removed from the copy stored in git (see .gitattributes). Run once per
# clone, from anywhere inside the repo. Requires `python` on PATH.
set -e
root="$(git rev-parse --show-toplevel)"
git config filter.stripcomments.clean "python \"$root/tools/strip_rs_comments.py\""
# Identity on checkout — comments are not (and cannot be) re-added.
git config filter.stripcomments.smudge cat
echo "Enabled stripcomments clean filter. Apply to all tracked files with:"
echo "    git add --renormalize ."
