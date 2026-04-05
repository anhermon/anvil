# Tools

Reference guide for tools available to agents working on this project.
Brand Artist monitors `/claude-public/` for new skills and updates this file periodically.

---

## Asset Generation

| Tool | Purpose | Notes |
|------|---------|-------|
| SVG (inline) | Vector icons, logos, lockups | Preferred for all brand assets; scalable, no dependencies |
| Gemini image gen | AI reference images / mood boards | Board uses for concept direction; output `.png` as reference only |

---

## Brand Assets

All production assets live in `assets/`. Source of truth is the SVG files — do not commit rasterized versions as primary assets.

| File | Purpose | Last updated |
|------|---------|--------------|
| `assets/anvil.svg` | App icon — dark bg, ember glow | 2026-04-04 |
| `assets/anvil-icon.svg` | Standalone icon variant | 2026-04-04 |
| `assets/anvil-logo.svg` | Logo mark | 2026-04-04 |
| `assets/anvil-lockup.svg` | Horizontal lockup — icon + wordmark | 2026-04-04 |
| `assets/anvil-mascot.svg` | Brand character / mascot | 2026-04-04 |

### Design tokens

| Token | Value | Usage |
|-------|-------|-------|
| Background | `#1c1c1c` / `#151515` | Icon dark bg |
| Steel light | `#8a8a8a` | Anvil top face highlight |
| Steel mid | `#585858` | Anvil body mid |
| Steel dark | `#363636` | Anvil body shadow |
| Ember orange | `#ff7200` | Glow line center |
| Ember edge | `#ff5500` | Glow line edges |
| Ember fade | `#b03000` | Glow line outer fade |
| Brand cyan | `console::style().cyan()` | Terminal UI |

---

## CLI Skills

| Skill | Path | Purpose |
|-------|------|---------|
| svg-create | `.claude/skills/svg-create/` | SVG asset creation helper (deprecated by Brand Artist direct authoring) |

---

## Monitoring

`/claude-public/` — shared skill directory. Brand Artist checks this directory each heartbeat for new asset-generation or design skills and updates this table when relevant skills are found.

*Last checked: 2026-04-04 — directory not yet present in this environment.*
