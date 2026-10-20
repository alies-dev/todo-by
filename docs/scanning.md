# What gets scanned

Every text file git would track. `todo-by` walks the tree with ripgrep's directory walker, so the rules are git's, even outside a repository.

- **Ignore files.** `.gitignore` is honored with full git semantics: nested files, negation, `**` globs, and `.git/info/exclude`. The `exclude` config key covers whatever `.gitignore` does not.
- **Dotfiles.** Dotfiles and dotted directories are scanned like any other, `.github/workflows` among them, because git tracks them.
- **Version control metadata.** `.git`, `.hg`, `.svn`, and `.jj` are never walked, at any depth, and naming one (or anything inside it) on the command line does not reach it either.
- **Binary files** are skipped.
- **Symlinks** the walk runs into are skipped, because it never follows one.

## Paths named on the command line

A named path is where the walk starts, not something it found, so two rules change:

- `.gitignore` does not apply to it: naming an ignored file scans that file, and naming an ignored directory scans it in full.
- A symlink is followed to its target.

Use `todo-by --files` to list exactly which files a run would scan.
