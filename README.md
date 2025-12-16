# jj-ryu
<img width="366" height="366" alt="image" src="https://github.com/user-attachments/assets/1691edfc-3b65-4f8d-b959-71ff21ff23e5" />

Stacked PRs for [Jujutsu](https://jj-vcs.github.io/jj/latest/). Push bookmark stacks to GitHub and GitLab as chained pull requests.

## What it does

```
       [feat-c]
    @  ○○○○ Add logout       ──►   PR #3: feat-c → feat-b
    │
       [feat-b]
    ○  ○○○○ Add sessions     ──►   PR #2: feat-b → feat-a
    │
       [feat-a]
    ○  ○○○○ Add auth         ──►   PR #1: feat-a → main
    │
  trunk()
```

Each bookmark becomes a PR. Each PR targets the previous bookmark (or trunk). When you update your stack, `ryu` updates the PRs.

## Install

```sh
# npm (includes prebuilt binaries)
npm install -g jj-ryu

# or with npx
npx jj-ryu

# cargo
cargo install jj-ryu
```

Binary name is `ryu`.

**macOS:** If you see "ryu can't be opened", run:
```sh
xattr -d com.apple.quarantine $(which ryu)
```

## Quick start

```sh
# View your bookmark stacks
ryu

# Submit a stack as PRs
ryu submit feat-c

# Preview what would happen
ryu submit feat-c --dry-run

# Sync all stacks
ryu sync
```

## Usage

### Visualize stacks

Running `ryu` with no arguments shows your bookmark stacks:

```
$ ryu

Bookmark Stacks
===============

Stack #1: feat-c

       [feat-c]
    @  yskvutnz e5f6a7b8 Add logout endpoint
    │
       [feat-b] ↑
    ○  mzwwxrlq a1b2c3d4 Add session management
    │
       [feat-a] ✓
    ○  kpqvunts 7d3a1b2c Add user authentication
    │
  trunk()

1 stack, 3 bookmarks

Legend: ✓ = synced with remote, ↑ = needs push, @ = working copy

To submit a stack: ryu submit <bookmark>
```

### Submit a stack

```sh
ryu submit feat-c
```

This will:
1. Push all bookmarks in the stack to remote
2. Create PRs for bookmarks without one
3. Update PR base branches if needed
4. Add stack navigation comments to each PR

Output:

```
Submitting 3 bookmarks in stack:
  - feat-c
  - feat-b
  - feat-a (synced)

[push, create PRs, update stack comments...]

Successfully submitted 3 bookmarks
Created 2 PRs
```

### Stack comments

Each PR gets a comment showing the full stack with status indicators:

```
* 🟢 feat-c #13
* 🟢 feat-b #12 👈
* 🟣 feat-a #11

---
This stack is managed by jj-ryu.
```

GitHub/GitLab auto-link PR references and show status: 🟢 open, 🟣 merged, 🔴 closed, ⚫ draft.

Comments update automatically when you re-submit.

### Dry run

Preview without making changes:

```sh
ryu submit feat-c --dry-run
```

### Sync all stacks

Push all stacks to remote and update PRs:

```sh
ryu sync
```

## Authentication

### GitHub

Uses (in order):
1. `gh auth token` (GitHub CLI)
2. `GITHUB_TOKEN` env var
3. `GH_TOKEN` env var

For GitHub Enterprise, set `GH_HOST`:

```sh
export GH_HOST=github.mycompany.com
```

### GitLab

Uses (in order):
1. `glab auth token` (GitLab CLI)
2. `GITLAB_TOKEN` env var
3. `GL_TOKEN` env var

For self-hosted GitLab, set `GITLAB_HOST`:

```sh
export GITLAB_HOST=gitlab.mycompany.com
```

### Test authentication

```sh
ryu auth github test
ryu auth gitlab test
```

## Workflow example

```sh
# Start a feature
jj new main
jj bookmark create feat-auth

# Work on it
jj commit -m "Add user model"

# Stack another change on top
jj bookmark create feat-session
jj commit -m "Add session handling"

# View the stack
ryu

# Submit both as PRs (feat-session → feat-auth → main)
ryu submit feat-session

# Make changes, then update PRs
jj commit -m "Address review feedback"
ryu submit feat-session

# After feat-auth merges, rebase and re-submit
jj rebase -d main
ryu submit feat-session
```

## CLI reference

```
ryu [OPTIONS] [COMMAND]

Commands:
  submit  Submit a bookmark stack as PRs
  sync    Sync all stacks with remote
  auth    Authentication management

Options:
  -p, --path <PATH>  Path to jj repository
  -V, --version      Print version
  -h, --help         Print help
```

### submit

```
ryu submit <BOOKMARK> [OPTIONS]

Arguments:
  <BOOKMARK>  Bookmark to submit

Options:
  --dry-run          Preview without making changes
  --remote <REMOTE>  Git remote to use (default: origin)
```

### sync

```
ryu sync [OPTIONS]

Options:
  --dry-run          Preview without making changes
  --remote <REMOTE>  Git remote to use (default: origin)
```

### auth

```
ryu auth github test    # Test GitHub auth
ryu auth github setup   # Show setup instructions
ryu auth gitlab test    # Test GitLab auth
ryu auth gitlab setup   # Show setup instructions
```

## License

MIT
