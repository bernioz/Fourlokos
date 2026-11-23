---
name: voxel-model
description: Turn an idea into a Minecraft-style voxel 3D model in Blockbench. Generates a turnaround reference image with OpenAI image generation (API key), iterates with the human until they approve it, then builds bones, cubes, textures and animations through the Blockbench MCP server and exports .bbmodel + .gltf. Use when the user wants a Blockbench, voxel, blocky or Minecraft-style 3D model, character or game asset.
---

# Voxel model workflow

You turn the human's idea into a **solid base model** in Blockbench that they will polish by hand.
The human decides at every gate. Never skip a gate, never treat silence as approval.

Reply in the human's language. Keep each message short: what you did, what you need from them.

## Paths and commands

- `SKILL_DIR` is the folder that holds this SKILL.md (`.agents/skills/voxel-model` for Codex,
  `.claude/skills/voxel-model` for Claude Code). **Run every command from the repository root.**
- Python: `python` (Windows: `python` or `py -3`; Linux/macOS: `python3` if `python` is missing). The
  scripts use only the standard library.
- Outputs go to `art/<slug>/` at the repo root and are committed to git: `<slug>.bbmodel`,
  `<slug>.gltf` and `reference.png` (the approved image). Drafts live in `art/<slug>/drafts/` (git-ignored).
- Every script prints exactly one JSON object on stdout. Read `ok`; on failure read `error` and `message`.
- Never print, log or write the OpenAI key anywhere: it lives only in the `OPENAI_API_KEY` environment variable.

## 0. Environment check

Run `python SKILL_DIR/scripts/voxel_doctor.py`. If `ok` is false, tell the human each failing check's
`fix` line and stop until they say it is fixed. Common fixes:
- `openai_api_key` / `openai_image_model` → they set `OPENAI_API_KEY` as the README says (then open a new terminal).
- `blockbench_mcp` → open Blockbench **desktop** with the MCP plugin loaded (README). If the MCP tools are
  missing from your session, ask them to reconnect the MCP server in their agent (`/mcp` in Claude Code;
  restart the Codex session) — the project already declares it in `.mcp.json` and `.codex/config.toml`.

## 1. Generate the reference

```bash
python SKILL_DIR/scripts/generate_reference.py "<idea, in English if possible>"
```
Takes about a minute and spends a few cents of the human's API credit (`--quality low` for rough drafts,
`--dry-run` to see the prompt without calling anything). It writes `art/<slug>/drafts/reference-vN.png`.
Errors: `missing_api_key` / `invalid_api_key` → step 0 fix; `insufficient_quota` → the account has no credit;
`moderation_blocked` → rephrase the idea; `model_not_found` / `no_image_model` → tell the human (the
organization may need verification on platform.openai.com); `network_error` / `api_error` → retry once, then tell them.

## 2. Show it and ask (GATE 1)

- Give them the image path and offer to open it (Windows `start "" "<path>"`, macOS `open`, Linux `xdg-open`; ignore failures).
- **Read the image yourself** so you can discuss it.
- Ask: do they like it, or what would they change?

## 3. Refine until approved

For each change request, translate it to a short English instruction and refine the latest draft:
```bash
python SKILL_DIR/scripts/generate_reference.py "<idea>" --continue "<image_path of the draft>" --feedback "<change>"
```
(It sends the draft back to the image model with the change, keeping the turnaround layout.) Show the new
version, ask again. Repeat until they clearly approve one version, then keep it:
```bash
python SKILL_DIR/scripts/generate_reference.py --approve "<approved draft path>"   # -> art/<slug>/reference.png
```
Only that approved image is the reference from here on.

## 4. Setup questions (GATE 2) — one message, with proposed defaults

1. **Format**: default `free` (Generic Model → exports glTF for Godot/Unity). Offer `bedrock` /
   `modded_entity` only if they target Minecraft.
2. **Texture resolution**: 64×64 default; 32×32 for very simple subjects; 128×128 for very detailed.
3. **Animations**: propose a list that fits the subject (e.g. `idle`, `walk`, `attack`, `death`;
   a slime gets `idle` + `bounce`). They may add or remove.

## 5. Part plan — geometry must be FAITHFUL to the reference

The human expects the model to match the image's proportions and shapes, not a generic Minecraft rig.
A skull that is taller than wide must be taller than wide; thin bones stay thin; a big weapon stays big.

**Measure, don't eyeball:**
1. Build a material map of the front view (and back/iso as needed) with the helper:
   `python SKILL_DIR/scripts/reference_map.py <image> <x0> <y0> <x1> <y1> [step]` (needs Pillow: `python -m pip install pillow`) — prints an ASCII grid
   (`B` bone/light, `w` warm brown/beige, `#` black, `c` dark, `R` red, `.` background) with pixel rows
   and columns, so you can read edges as coordinates.
2. Pick the scale from the character's total height: ground row (feet bottom) and top row →
   `px_per_unit = height_px / target_height_units` (≈32 units for a humanoid ~2 blocks tall).
3. Convert every part's pixel box to units relative to the character's center column and the ground.
   Write the measured numbers in the plan.
4. Reproduce what the silhouette shows: non-cubic heads, separate jaws, open garments, double bones
   (radius/ulna, tibia/fibula), wider joints (elbows, knees), hands with fingers, spikes, tatters.
5. **Model details as voxels, not just paint.** A flat cube with painted features is the bare minimum
   and reads as low effort. Carve and add geometry on a 0.5-unit grid (1 texel at 2 px/unit):
   recessed eye sockets (≈1 unit deep) with the eye at the back, protruding brows and cheekbones,
   nose holes, individual teeth, rib bars over a dark cavity, separate tatter strips of varying length,
   layered/stepped armor plates, knuckles/patellas/toes, vertebrae, spikes. Paint then adds shading,
   cracks and grain on top. Scale this to the subject (a slime needs little; a skeleton warrior needs a lot).
6. Turnaround views can disagree (a pose in front, another in back). Model the front view's pose and
   say which view you followed.

Write a compact plan and show it (continue unless they object):
- Bones as a tree: name, parent, pivot `[x,y,z]`. Names in English snake_case: `root`, `body`, `head`,
  `arm_left`, `arm_right`, `leg_left`, `leg_right`, plus extras (`scarf`, `tail`, `horn_left`…).
- Cubes per bone: `from`/`to` in Blockbench units.
Conventions: 16 units = 1 block; feet on y=0; model faces north (−Z) so "front" is −Z; centered on x=0;
left/right are the character's own left/right — facing −Z, the character's **right is +X**, which shows
on the viewer's LEFT in the front (north) view, exactly like the reference's front view.
Poses (raised arm, held weapon) go in group `rotation`; add a counter-rotation on a child group to keep
a held object upright. Verify signs with a screenshot. Pivots go **at the joint** (shoulder, hip, neck), not at
the part center — this is what makes animation work and is the hardest thing for the human to fix later.
Keep it blocky: prefer few, well-sized cubes over many tiny ones; use small cubes only for details
that read in the reference (eyes, moss tufts, scarf ends).

## 6. Build the geometry (Blockbench MCP)

- Many MCP tools are only enabled in certain modes or with a selection. Use `list_modes` / `set_mode`
  (`edit`, `paint`, `animate`) before mode-dependent tools. If a tool seems missing, call
  `get_capabilities` with `include_tools: true`.
- Read each tool's schema before first use; do not guess parameter names.
- Order: `create_project` (name = slug, chosen format) → `add_group` for each bone (with pivot and
  parent) → `place_cube` into its bone → `modify_cube` to adjust.

### No overlapping faces (z-fighting)

Two cubes must never have coplanar overlapping faces facing the same way (they flicker as the renderer
can't decide which draws on top). After every geometry change run `snippets/find_zfight.js` through
`risky_eval` (it checks in world space, so rotated bones count) and fix every hit with geometry, not
`inflate`: make the detail piece protrude ≥0.05–0.1 past the surface, pull hidden cores slightly inside,
or delete pieces fully enclosed by another. Re-run until `count` is 0.

## 7. Geometry checkpoint (GATE 3)

Use `set_camera_angle` for front, back and an isometric angle and `capture_screenshot` each time.
Compare against the reference yourself first and fix obvious mismatches. Then show the human the
screenshots next to the reference and ask what to correct. Iterate until they approve.

## 8. Textures (GATE 4)

1. Judge the subject's complexity from the reference and **announce the detail level with concrete
   features** before painting. Examples:
   - simple: "flat colors per part, 2–3 shades, eyes as pixels"
   - medium: "per-face shading, a few accent pixels, visible part edges"
   - detailed: "per-face shading, moss tufts and vines, stone cracks, glowing eyes, scarf folds"
   Texture detail scales with the subject; do not default to flat colors for a complex character.
2. Wait for them to confirm or adjust the level.
3. Size the texture from the surface area first (sum of cube faces): at 2 px per unit a humanoid with
   ~100 cubes needs 128×128 (`create_texture` with `uv_width/uv_height` 64). Tell the human if this
   changes the resolution they picked.
4. Pack per-face UVs with `snippets/pack_uvs.js` (shelf packer, pixel-aligned, no overlaps).
5. Paint procedurally through `risky_eval` + `texture.edit(canvas => …)`: classify each cube by name into
   a material (bone, cloth, leather, metal, wood…), fill each face with a small palette + face shading
   (up brighter, down darker), then add per-part details (eyes, cracks, grain, stitches, tattered
   transparent edges). Keep the painter as a function on `window` so re-paints after geometry fixes
   are one call. Native paint tools (`paint_fill_tool`, `draw_shape_tool`, `paint_with_brush`) are fine
   for touch-ups.
6. Screenshot checkpoint like step 7 (compare side by side with the reference); iterate until approved.

## 9. Animations (GATE 5 is the list from step 4)

Rig rules that make animation work (learned the hard way):
- **Hierarchy:** `add_group` treats `parent: "root"` as the PROJECT root. Name your top bone something
  else or pass its UUID, then verify the tree (`Outliner.root` via `risky_eval`) — every bone must hang
  from the top bone or moving it does nothing.
- **Bones rest at 0°.** If the reference pose has a raised arm, bake that rotation into the cubes
  (cube `origin` = joint, cube `rotation` = pose) and move child pivots to their posed world positions,
  so the bone itself has rotation 0 and animation axes are clean (x = forward/back, z = in/out).
- **Held items are rigid in the hand:** child of the forearm/hand bone, never animated on their own
  (except when dropped, e.g. in death).
- **Attacks read well as guard → arc → impact → hold → recover**, with intermediate keyframes so euler
  interpolation can't twist the limb. Offer several attack types (overhead smash, horizontal sweep,
  jab/thrust…). To hit exact poses, grid-search shoulder/elbow rotations so the weapon head reaches a
  target point/direction, keeping each pose close to the previous one.
- **Self-collision check:** sample every animation every 0.025 s and test the weapon's corner/mid
  points against every other cube's oriented box (transform points into each cube's local space). The
  weapon must never pass through the body: end follow-throughs in front of the torso, add an
  intermediate recovery key in front, and when a weapon is dropped, swing the arm away from the body
  and solve the weapon's fall so it rests on the floor beside the character.
- **Floor check:** sample every animation every 0.05 s and make sure the lowest vertex stays ≥ 0
  (call `scene.updateMatrixWorld(true)` first, or parent matrices are stale). Fix with root/club
  position keys.

`set_mode` `animate` → per animation: `create_animation` (loop for idle/walk/bounce, play-once for
attack/death) → `manage_keyframes` on the bones. Keep motions readable and blocky: idle = subtle body
bob + head sway; walk = opposite arm/leg swing around the joint pivots; attack = wind-up then strike.
Show a short description of each animation and offer to adjust timings.

## 10. Export

Export into the repo's `art/<slug>/` folder, with **absolute** paths (the MCP server writes where you tell it):
use `list_export_formats` then `export_model`:
- `project` codec → `art/<slug>/<slug>.bbmodel`
- `gltf` codec → `art/<slug>/<slug>.gltf` (only for formats that support it)

If Blockbench is a Flatpak (Linux) it can only write to XDG folders (Downloads, Documents…): export to
`$(xdg-user-dir DOWNLOAD)/<file>` and then move it into `art/<slug>/`.

Finish with a short summary: files written, bones, animations, and what you'd polish by hand first. Tell the
human the model lives in `art/<slug>/` (with `reference.png`) ready to `git add art/<slug>` and commit.
