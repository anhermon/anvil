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

### Icons & App

| File | Purpose | viewBox | Last updated |
|------|---------|---------|--------------|
| `assets/anvil.svg` | App icon — dark bg, ember glow, graceful horn | 200×200 | 2026-04-10 |
| `assets/anvil-icon.svg` | Compact icon variant (small UI use) | 64×64 | 2026-04-04 |
| `assets/anvil-light.svg` | Light-mode app icon — warm-white bg | 200×200 | 2026-04-10 |
| `assets/anvil-mono.svg` | Monochrome — transparent bg, `currentColor` fill | 200×200 | 2026-04-10 |
| `assets/anvil-favicon.svg` | Favicon — bold simplified silhouette | 32×32 | 2026-04-10 |

### Lockups

| File | Purpose | viewBox | Notes |
|------|---------|---------|-------|
| `assets/anvil-logo.svg` | **Primary lockup** — icon + wordmark, dark bg, fixed stroke `#d8d8d8` | 280×80 | Use on dark surfaces |
| `assets/anvil-lockup.svg` | **Alternative lockup** — icon + wordmark, `currentColor` strokes | 260×80 | Adapts to light/dark context |

### Marketing & Social

| File | Purpose | viewBox | Last updated |
|------|---------|---------|--------------|
| `assets/anvil-social.svg` | Open Graph / Twitter Card | 1200×630 | 2026-04-10 |
| `assets/anvil-github-avatar.svg` | GitHub org / profile avatar | 400×400 | 2026-04-10 |
| `assets/anvil-mascot.svg` | Brand character "Anvi" | 200×200 | 2026-04-10 |

### Design tokens

| Token | Value | Usage |
|-------|-------|-------|
| Background (dark) | `#1e1e1e` → `#111111` | App icon dark bg radial gradient |
| Background (light) | `#f5f5f0` → `#e8e8e2` | Light-mode icon bg |
| Steel highlight | `#909090` | Anvil top face, brightest |
| Steel mid-light | `#747474` | Anvil body upper |
| Steel mid-dark | `#555555` | Anvil body lower |
| Steel shadow | `#3a3a3a` | Anvil bottom / feet |
| Ember hot | `#ff9900` | Ember gradient center |
| Ember warm | `#ff7700` | Ember gradient mid |
| Ember edge | `#ff4400` | Ember gradient edge |
| Ember halo | `#ff5500` | Ambient halo fill |
| Hardy hole | `#111111` | Deep shadow socket |
| Mascot dark metal | `#252525` → `#444444` | Mascot body gradient |
| Mascot highlight | `#909090` | Mascot face stripe / eyebrows |
| Mascot pupil | `#0d0d0d` | Mascot eye pupils |
| Blush | `#E07070` opacity 0.35 | Mascot cheek circles |
| Brand orange (CLI) | `\033[38;5;202m` | Terminal ANSI (256-color) |

### Usage guidelines

- **Dark context** → use `anvil.svg`, `anvil-logo.svg`
- **Light context** → use `anvil-light.svg`; or `anvil-lockup.svg` (currentColor → set to dark)
- **Monochrome** (print, emboss, watermark) → use `anvil-mono.svg` with `color: #000` or `color: #fff`
- **Favicon** → use `anvil-favicon.svg` (also works as `.ico` by wrapping in an `<img>`)
- **Social preview** → `anvil-social.svg` export to PNG at 1200×630 for OG tags
- **GitHub** → `anvil-github-avatar.svg` export to PNG at 400×400 for org avatar
- **Mascot** → `anvil-mascot.svg` for docs, changelogs, or any place needing a friendly brand character

---

## CLI Skills

| Skill | Path | Purpose |
|-------|------|---------|
| svg-create | `.claude/skills/svg-create/` | SVG asset creation helper |

---

## Monitoring

`/claude-public/` — shared skill directory. Brand Artist checks this directory each heartbeat for new asset-generation or design skills and updates this table when relevant skills are found.

*Last checked: 2026-04-10 — directory not yet present in this environment.*
