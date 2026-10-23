# taixu Agent Skill

`skills/taixu` is a distributable Claude Code skill for **users of the taixu
crate**. It is intentionally not loaded from the taixu repository's own
`.claude/` directory, and it does not require the taixu source tree: copy it
into the project (or user) where you build your game.

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

- `SKILL.md` — complete crate-usage guide, recipes, contracts, locks and pitfalls.
- `references/` — bundled API, JSON, state-machine, persistence, hosting, NPC,
  config, error, cookbook and changelog references. No upstream source needed.

The bundled references are generated from the taixu repository `docs/`.
Maintainer-only scripts live outside this distributable skill under
`scripts/` in the taixu repository. Do not edit `references/` by hand; run
`scripts/sync-skill-docs.sh` after changing `docs/`.
