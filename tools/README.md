# tools

## Comment stripping (clean filter)

`*.rs` files are stored in git **without comments** while your working tree
keeps them. This is done with a git *clean* filter, not `.gitignore` (gitignore
can only exclude whole files, not content inside them).

### Pieces
- `.gitattributes` — maps `*.rs` to the `stripcomments` filter.
- `tools/strip_rs_comments.py` — a Rust-aware scanner that removes `//`, `///`,
  `//!`, and `/* */` (nested) comments. It does **not** touch `//` or `/* */`
  inside string / byte-string / raw-string / char literals, and leaves
  lifetimes (`'a`) alone. The output is what gets committed, so it only ever
  removes comments — never code.
- `tools/setup-git-filters.sh` / `.ps1` — registers the filter command in this
  clone's **local** git config (the command is intentionally not committed).

### Enable (once per clone)
```sh
sh tools/setup-git-filters.sh        # or:  pwsh tools/setup-git-filters.ps1
```
Requires `python` on PATH. Without this step, git uses an identity filter and
comments pass through unchanged (nothing breaks).

### Apply to already-tracked files
The filter only runs when a file is staged. To rewrite every tracked `*.rs`
through it at once:
```sh
git add --renormalize .
git commit -m "Strip comments from committed sources"
```

### Things to know
- **Clones are comment-free.** The smudge filter is identity, so a fresh clone
  (or any `git checkout` / `git restore` of a `.rs`) gets the comment-free
  version. Keep a working copy if you want the commented source locally.
- **Solo by design.** The filter command lives in local config, so collaborators
  must run the setup script too; otherwise their commits keep comments.
- Run the test/preview manually any time:
  `python tools/strip_rs_comments.py < src/app.rs | less`
