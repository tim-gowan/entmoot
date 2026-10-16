# Local operator and maintainer guidance

[AGENTS](../AGENTS.md) owns always-applicable invariants. Use only the workflow
needed for the task; Rust contracts and generated schemas own fields.

- [entmoot-operate](skills/entmoot-operate/SKILL.md): source navigation, native
  readiness, open boundaries and local recovery.
- [entmoot-release](skills/entmoot-release/SKILL.md): package/docs verification,
  qualification and an authorized release handoff.
- [Test lanes](rules/entmoot-test-lanes.mdc): existing focused test ownership.
- [Format series](rules/entmoot-format-series.mdc), [unsigned commits](rules/commit-unsigned-end-of-turn.mdc).

These instructions introduce no new scheduler or approval layer. Preserve user
intent and authorization; local preparation does not imply successful publication.

## IDE / agent tool trees

Canonical content lives here under `.agents/`. `.cursor/`, `.codex/`, and
`.claude/` are gitignored. Where a harness supports this layout, links can reuse
the same files. First inspect existing paths; never replace a populated directory
or another link. The following commands apply only to absent paths. Codex uses
the root AGENTS.md entry point; these links alone do not enable rule discovery:

```text
# PowerShell (repo root)
New-Item -ItemType SymbolicLink -Path .cursor -Target .agents
New-Item -ItemType SymbolicLink -Path .codex -Target .agents
New-Item -ItemType SymbolicLink -Path .claude -Target .agents
```

```text
# POSIX
ln -s .agents .cursor
ln -s .agents .codex
ln -s .agents .claude
```
