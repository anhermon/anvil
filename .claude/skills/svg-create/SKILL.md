# SVG Creation Skill

Generate clean, professional SVG graphics — silhouettes, icons, and logo lockups — using pure SVG path geometry with gradients, filters, and layered depth.

## When to use

Trigger this skill when asked to:
- Create or redesign an SVG icon, logo, or mascot
- Produce a silhouette from a reference shape description
- Evaluate why an existing SVG looks poor and fix it

---

## Core Design Process

### 1. Identify the silhouette skeleton

Before drawing, identify the 5–8 structural points that define the outline of the shape. For complex objects (anvil, animal, tool), sketch the profile in terms of:
- **Widest span** (x-axis extremes)
- **Tallest span** (y-axis extremes)
- **Key inflection points** — where the outline changes direction significantly

Rule: a recognizable silhouette requires every major anatomical feature to be proportionally represented. If a feature defines the object (e.g., an anvil horn, a hammer head), it must occupy at least 25–35% of the relevant axis.

### 2. Choose a viewBox

| Use case | Recommended viewBox |
|----------|---------------------|
| App icon (square) | `0 0 200 200` |
| Icon (small) | `0 0 64 64` or `0 0 32 32` |
| Logo lockup (wide) | `0 0 260 80` |
| Emblem (medium) | `0 0 120 80` |

Leave a ~8% margin around the shape so it doesn't clip.

### 3. Trace the outline as compound paths (not rectangles)

Use **clockwise compound `<path d="...">` elements** for all non-trivial shapes. Rectangles (`<rect>`) are only appropriate for work faces and flat surfaces that are genuinely rectilinear.

**Critical rule:** Organic shapes (bodies with waists, tapered horns, curved shoulders) MUST use bezier curves — never approximate with rectangles or polylines.

**Path command reference:**

| Command | Meaning |
|---------|---------|
| `M x,y` | Move to (start point) |
| `L x,y` | Line to |
| `Q cx,cy x,y` | Quadratic bezier (one control point) |
| `C c1x,c1y c2x,c2y x,y` | Cubic bezier (two control points) |
| `Z` | Close path |

**Horn / taper tips:**
- A tapered projection (e.g., anvil horn, beak, spike) should use `Q` bezier curves for both the top and bottom edges, with control points that bow slightly away from the center line.
- The tip ends at the intersection of the two bezier curves — both curves end at the same anchor point.
- Taper length should be ≥ 25% of total shape width to be visually prominent.

**Waist / organic body:**
- Use `C` (cubic) curves to create the narrowing waist of an anvil/hourglass shape — NOT `L` lines.
- A waist narrows from body width to ~60–70% of body width, then flares back out.

**Example: left-pointing horn (face at x=22–60, y=10–18; tip at x=4, y=14):**
```
M 4,14
Q 13,9 22,10     <- top edge: curves upward toward face
L 60,10          <- face top
...
L 22,18          <- face bottom-left
Q 13,19 4,14     <- bottom edge: curves downward back to tip
Z
```

**Example: anvil body with waist using C curves:**
```
M 55,88
L 162,88
L 162,100
C 158,100 154,106 150,116   <- right shoulder curve inward
L 148,132
L 156,146
L 156,164
L 112,164
L 108,148
L 92,148
L 88,164
L 44,164
L 44,146
L 52,132
L 50,116
C 46,106 42,100 38,100      <- left shoulder curve inward
L 38,88
Z
```

### 4. Add depth with gradients

For dark-themed icons, always use at least one linearGradient for the main body. Flat fills look amateurish.

**Steel/metal gradient (top-to-bottom, 4 stops):**
```
<linearGradient id="steel" x1="0" y1="0" x2="0" y2="1">
  <stop offset="0%"   stop-color="#909090"/>
  <stop offset="20%"  stop-color="#747474"/>
  <stop offset="60%"  stop-color="#555555"/>
  <stop offset="100%" stop-color="#3a3a3a"/>
</linearGradient>
```

**Work face (horizontal center-bright):**
```
<linearGradient id="face" x1="0" y1="0" x2="1" y2="0">
  <stop offset="0%"   stop-color="#6a6a6a"/>
  <stop offset="35%"  stop-color="#a0a0a0"/>
  <stop offset="65%"  stop-color="#a0a0a0"/>
  <stop offset="100%" stop-color="#6a6a6a"/>
</linearGradient>
```

**Ember/glow line (horizontal, fade-in/out at edges — 8 stops):**
```
<linearGradient id="ember" x1="0" y1="0" x2="1" y2="0">
  <stop offset="0%"   stop-color="#bb2200" stop-opacity="0"/>
  <stop offset="8%"   stop-color="#ff4400"/>
  <stop offset="30%"  stop-color="#ff7700"/>
  <stop offset="50%"  stop-color="#ff9900"/>
  <stop offset="70%"  stop-color="#ff7700"/>
  <stop offset="92%"  stop-color="#ff4400"/>
  <stop offset="100%" stop-color="#bb2200" stop-opacity="0"/>
</linearGradient>
```

### 5. Add glow with a two-layer filter system

For ember/heat effects, use TWO complementary filter layers:

**Layer A — ambient halo** (large, soft, behind the object):
```
<filter id="glow-halo" x="-80%" y="-80%" width="260%" height="260%">
  <feGaussianBlur stdDeviation="18"/>
</filter>
<ellipse cx="115" cy="74" rx="72" ry="22"
         fill="#ff5500" opacity="0.32" filter="url(#glow-halo)"/>
```

**Layer B — crisp edge glow** (tight blur, on top, with SourceGraphic preserved):
```
<filter id="glow-edge" x="-20%" y="-600%" width="140%" height="1400%">
  <feGaussianBlur in="SourceGraphic" stdDeviation="4.5" result="blur"/>
  <feMerge>
    <feMergeNode in="blur"/>
    <feMergeNode in="blur"/>
    <feMergeNode in="SourceGraphic"/>
  </feMerge>
</filter>
<rect x="55" y="75" width="107" height="5" rx="2.5"
      fill="url(#ember)" filter="url(#glow-edge)"/>
```

**Why two layers?**
- The halo creates the ambient "hot object" feel (orange cloud above/around)
- The edge line creates the specific "glowing seam" look where metal meets heat
- Together they match the aesthetic of AI-generated forge/ember reference images

**Filter sizing rule:** set y and height to accommodate the blur spreading outside the element bounds. For stdDeviation="4.5" the blur spreads ~18px; for stdDeviation="18" it spreads ~72px.

### 6. Layer order (z-order)

Draw elements in this order (back to front):
1. Background shape (rect with background gradient)
2. Ambient halo ellipses (filtered, behind the object)
3. Main body path(s)
4. Surface detail paths (highlights, ledges, holes)
5. Edge outline strokes (depth/shadow)
6. Secondary ambient glow rects
7. Crisp ember/glow line (top layer)

### 7. Verify the output

Before declaring done, confirm all of the following:

- [ ] Valid XML — no unclosed tags, no duplicate xmlns
- [ ] Correct viewBox matches the intended dimensions
- [ ] Shape is recognizable at 32x32 (scale it down mentally)
- [ ] Every major anatomical feature is proportionally represented
- [ ] Horn/tapers use bezier curves (not triangles or stubby rects)
- [ ] Waist uses C curves (not straight lines)
- [ ] At least one gradient on the main body
- [ ] Two-layer glow system present (halo + edge) if showing heat/ember
- [ ] Self-contained — no href, no xlink:href to external files
- [ ] Filter bounds (x/y/width/height) are large enough to not clip the blur

---

## Anvil brand icon — reference implementation

The Anvil brand icon is a blacksmith's anvil in left-profile silhouette on a dark rounded-square background.

### Key proportions (200x200 viewBox)

| Feature | Coordinates | Notes |
|---------|-------------|-------|
| Background | rx="40" rounded rect | radialGradient #1e1e1e to #111111 |
| Work face | x=55 y=78 w=107 h=10 | linearGradient center-bright |
| Hardy hole | x=148 y=80 w=11 h=8 | #111111 fill |
| Horn attach | x=38-55, y=88-100 | bezier curves to tip at x~16, y=98 |
| Body top | y=88, spans x=38-162 | |
| Waist | C curves from y=100 to y=132 | narrows ~25% each side |
| Left foot | x=44-88, y=146-164 | |
| Right foot | x=112-156, y=146-164 | |
| Ember halo | cx=115 cy=74 rx=72 ry=22 | opacity=0.32, glow-halo filter |
| Ember line | x=55 y=75 w=107 h=5 | ember gradient + glow-edge filter |

### Color palette

| Token | Value | Usage |
|-------|-------|-------|
| #1e1e1e / #111111 | Background gradient | Near-black base |
| #909090 | Steel top highlight | Top of body gradient |
| #747474 | Steel mid-light | |
| #555555 | Steel mid-dark | |
| #3a3a3a | Steel shadow | Bottom of body gradient |
| #a0a0a0 | Face center | Work face horizontal gradient |
| #ff9900 | Ember hot | Center of ember gradient |
| #ff7700 | Ember warm | Mid ember |
| #ff4400 | Ember edge | Outer ember |
| #ff5500 | Halo fill | Ambient glow ellipse |

---

## Logo lockup — reference implementation

The lockup uses a 260x80 viewBox:
- Left: 62x72 rounded-square icon (clipped, scaled-down version of the app icon)
- Divider: vertical line at x=74, opacity=0.18
- Right: geometric stroke wordmark "anvil"

Wordmark specs:
- Cap height: 26px, baseline: y=56, start x=84
- stroke-width="3.8", stroke-linecap="round", stroke-linejoin="round"
- stroke="currentColor" (monochrome, adapts to dark/light contexts)
- Letter order: a (open bowl + stem), n (arch), v (chevron), i (stem + dot), l (tall stem)

---

## Common mistakes to avoid

| Mistake | Fix |
|---------|-----|
| Horn is a tiny stub triangle | Horn must be >= 25% of total width; use Q bezier curves for both edges |
| Waist uses straight lines | Use C curves; waist should narrow ~25% from body width |
| Body made of stacked rect elements | Use a single compound path d for organic unity |
| No gradient on body | Steel/metal always needs at least a 3-stop top-to-bottom gradient |
| Single flat glow filter | Use two layers: ambient halo (stdDeviation>=15) + crisp edge (stdDeviation<=5) |
| Ember gradient has uniform opacity | Fade to stop-opacity="0" at both ends for natural look |
| Filter clips the glow | Set filter x/y/width/height bounds large enough (~600% height for tight vertical glow) |
| Shape clips at edge | Add 6-10% margin inside viewBox |
| SVG renders blank | Check that fill is set on all elements; fill="none" on outer element overrides children |
