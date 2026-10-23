# taixu Agent Skill

`skills/taixu` is a distributable Claude Code skill for **users of the taixu
library**. It is intentionally not loaded from the taixu repository's own
`.claude/` directory: copy it into the project (or user) where you build your
game.

## Install

Project-local:

```bash
mkdir -p .claude/skills
cp -R /path/to/taixu/skills/taixu .claude/skills/taixu
```

User-wide (available in every project):

```bash
mkdir -p ~/.claude/skills
cp -R /path/to/taixu/skills/taixu ~/.claude/skills/taixu
```

Or symlink it while developing against a local checkout:

```bash
mkdir -p .claude/skills
ln -s /path/to/taixu/skills/taixu .claude/skills/taixu
```

Then invoke `/taixu` or let Claude Code select it from its description.

## Contents

- `SKILL.md` — complete usage guide, recipes, contracts, locks and pitfalls.
- `references/` — bundled copies of the repository's `docs/` handbook and
  `CHANGELOG.md`, so the skill remains usable outside the taixu checkout.
- `scripts/check.sh` — maintainer verification (fmt, clippy, tests, doctests,
  docs, examples, reference sync).
- `scripts/sync-docs.sh` — maintainer tool to refresh `references/` from
  `docs/` (`--check` for CI).

The bundled references are generated from the taixu repository. Do not edit
them by hand; run `scripts/sync-docs.sh` after changing `docs/`.
