# Gitr

A fast, native Git client written in Rust with [egui](https://github.com/emilk/egui).
Runs on macOS, Windows and Linux. It drives the installed `git`
executable, so it behaves exactly like git on the command line (hooks, credential helpers, config).

Inspired by Sourcetree and Fork.

## Features

- **Repository manager**: saved repositories, tabs, drag and drop a folder to open it, clone, init
- **Commit graph**: colored lanes, branch/tag badges, search by message, author, SHA or ref,
  and keyboard navigation
- **Commit details**: author/committer, refs, parents, message, inline diffs (Commit tab), a file
  list with a full diff (Changes tab), and a browsable tree of the files at that commit (File Tree tab)
- **Local changes**: stage or unstage files, **hunks and individual lines**, discard changes,
  resolve conflicts (ours/theirs/merge tool), commit, amend, commit and push
- **Diffs**: word-level highlighting, adjustable context, ignore whitespace
- **Branches, remotes, tags, stashes, submodules** in the sidebar, each with context menus:
  checkout, merge, rebase, create/rename/delete, fast-forward, push, apply/pop/drop stash
- **Operations**: fetch, pull (merge/rebase, autostash), push (upstream, force-with-lease, tags),
  stash, cherry-pick, revert, reset (soft/mixed/hard), and continue/abort/skip for a
  merge, rebase, cherry-pick or revert in progress
- Activity log of every git command, error notifications, dark/light themes, UI zoom

## Credentials

Gitr uses your normal git setup: credential helpers (macOS Keychain, Git Credential Manager), SSH keys
and `ssh-agent`. When git or ssh still needs something (a username, password/token, key passphrase or
confirmation of a new SSH host key), Gitr shows a small dialog. It is registered as `GIT_ASKPASS` /
`SSH_ASKPASS` for the git commands it runs. The answer is piped straight back to git; Gitr never
logs or saves it. Git itself may save working HTTPS credentials in your configured credential helper.

## Build & run

```bash
cargo run --release -- /path/to/repo
```

Linux needs the usual windowing dev packages (e.g. `libxkbcommon-dev libgtk-3-dev`).

### macOS app bundle

```bash
./bundle.sh               # dist/Gitr.app for this Mac's architecture
./bundle.sh --universal   # Apple Silicon + Intel
./bundle.sh --dmg         # also dist/Gitr-<version>.dmg
```

The bundle is ad-hoc signed; set `SIGN_IDENTITY` to a Developer ID identity (and notarize) to
distribute it. The icon comes from `assets/icon-1024.png`, rendered from `assets/logo.svg`.

## Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| ⌘/Ctrl O | Open repository |
| ⌘/Ctrl W | Close tab |
| Ctrl Tab / Ctrl ⇧ Tab | Next / previous tab |
| ⌘/Ctrl R, F5 | Refresh |
| ⌘/Ctrl F | Search commits |
| ⌘/Ctrl 1 / 2 | Local changes / all commits |
| ⌘/Ctrl Enter | Commit |
| ⇧⌘/Ctrl F, L, P, B | Fetch, pull, push, new branch |
| ↑ / ↓ | Move through commits |

## Development

`cargo test` runs the unit tests, including end-to-end hunk and line staging against a
temporary repository.

With `--features screenshot`, setting `GITR_SCREENSHOT=out.png` saves a window screenshot and exits
(see `src/devshot.rs`).

## License

[MIT](LICENSE)
