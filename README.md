# Fourlokos

## Granja y completo italiano

En las praderas aparecen **gallinas**: son pasivas, ponen **huevos** con el tiempo y dejan
**pollo crudo** y, a veces, plumas al morir. Las semillas se consiguen rompiendo pasto alto.
Planta tomate, girasol, trigo y brotes de palto; sus etapas avanzan con los *random ticks* del
mundo y las cosechas maduras se recolectan con clic derecho para que vuelvan a crecer.

En el horno puedes convertir semillas de girasol en **aceite de girasol** y pollo crudo en
**pollo cocido**. La mesa permite fabricar **mayonesa** (huevo + aceite), **pan** (tres trigos)
y el **completo italiano** (pan, vienesa, palta, tomate y mayonesa), que restaura mucha vida.

Repositorio del equipo Fourlokos de la hackathon Build Games with Codex Santiago.

Juego de planetas de bloques (voxel) en Rust + Bevy: un sol con 3 planetas generados por semilla,
gravedad esférica, inventario y modos survival/creative. Arranca en un menú 2D ("Start" / "Cerrar").

## Requisitos

- [Rust](https://rustup.rs) (cargo). En Windows hacen falta las Build Tools de Visual Studio (C++);
  en Linux, las librerías de Bevy (alsa, udev, wayland/X11).

## Compilar y correr

```sh
cargo run -p fourlokos            # semilla 1337
cargo run -p fourlokos -- 42      # otra semilla
cargo test
```

## Estructura

```
crates/orbit_core/   # simulación sin motor (planetas, bloques, jugador); con tests
crates/client/       # juego en Bevy (menú de inicio, mundo, cielo, UI)
schematics/          # estructuras de ejemplo
```

## Criaturas, NPCs y diálogo

El estado y la IA viven en `orbit_core` (`mob.rs`, `boss.rs`, `dialogue.rs`; sin motor, con tests y todo
el azar por `rng`); el cliente (`mobs.rs`, `dialogue.rs`, `story.rs`) los dibuja y los conecta al juego.

- **Slime**: pasivo (salta por la pradera, evita el agua) hasta que le pegas; entonces te persigue y daña
  por contacto (2 HP, con cooldown). 8 HP; tu golpe es el de la espada en mano (4/5/6/7, puño y pico 1).
  Suelta 0-2 **Slimeball** (1-3 si lo mató una espada). Solo aparecen en survival, en planetas Meadow, máx. 6.
- **Slime King**: jefe siempre hostil (200 HP, barra arriba con su nombre a menos de 40 bloques). Ataques
  avisados (se agacha y brilla): aplastamiento (6), embestida y, bajo el 50 % de vida (fase 2), más rápido,
  onda más ancha e invocación de slimes pequeños. Se invoca con un **Slime Crown Core** (mesa de crafteo:
  8 Slimeball en anillo alrededor de 1 Iron Ingot): clic derecho en el suelo; no hay dos a la vez.
  Suelta 6-10 Slimeball.
- **Bodoque** (planeta natal, cerca del spawn) y **Tulio** (en *Tulio's Moon*, la luna del planeta natal,
  que siempre existe): invulnerables, miran al jugador si está cerca y si no pasean despacio. Clic derecho
  abre el diálogo: teclas 1-3 o clic para responder, Enter muestra todo el texto, Esc cierra. Cada uno da un
  regalo único por mundo (Stone Pickaxe / 3 Slimeball).
- `/summon <slime|slime_king|bodoque|tulio>` para depurar. Demos: `ORBIT_DEMO=mobs|npc|moon|boss`.
- Se guarda `npcs.bin` junto al mundo (regalos dados, jefe derrotado, dónde está cada NPC); los slimes y el
  jefe no persisten.
- Los modelos de Bodoque, Tulio y la corona grande del King son **placeholders** de cubos
  (`crates/client/src/npc_models.rs`, marcados con `placeholder: replace with the real model`).
  El slime usa `assets/models/slime.gltf` (animaciones `idle` y `bounce`).

## Modelos 3D con la skill `voxel-model`

La skill genera una imagen de referencia con la API de imágenes de OpenAI, la aprueban entre ustedes y luego
el agente (Claude Code o Codex) construye el modelo en Blockbench: huesos, cubos, texturas y animaciones. El
resultado queda en `art/<slug>/` (`<slug>.bbmodel`, `<slug>.gltf` y `reference.png`, la imagen aprobada) y se
versiona con git. Los borradores van a `art/<slug>/drafts/` y no se suben.

### Instalación (una vez por persona, Windows)

1. **Python 3.9 o más nuevo** (python.org; marcar "Add python.exe to PATH"). Comprobar con `python --version`.
   La skill solo usa la biblioteca estándar; `pip install pillow` es opcional (solo para `reference_map.py`).
2. **Blockbench de escritorio** (no la versión web): instalador `Blockbench_<versión>.exe` en
   <https://github.com/JannisX11/blockbench/releases/latest> (sitio oficial: <https://www.blockbench.net>).
   Probado con la 5.2.2.
3. **Plugin MCP de Blockbench** ([jasonjgardner/blockbench-mcp-plugin](https://github.com/jasonjgardner/blockbench-mcp-plugin), v1.10.1):
   en Blockbench, **File > Plugins > Load Plugin from URL** y pegar
   `https://jasonjgardner.github.io/blockbench-mcp-plugin/mcp.js`. El servidor queda en
   `http://localhost:3000/bb-mcp` (puerto y ruta en Settings > General). Blockbench debe estar abierto, con el
   plugin cargado, cada vez que se use la skill. El repo ya declara ese servidor en `.mcp.json` (Claude Code,
   pide aprobarlo la primera vez) y en `.codex/config.toml` (Codex, solo si confían en el proyecto). Si en Codex
   `codex mcp list` no muestra `blockbench`, agregarlo una vez: `codex mcp add blockbench --url http://localhost:3000/bb-mcp`.
4. **Clave de la API de OpenAI** (`OPENAI_API_KEY`) con crédito. Nunca va en el repo ni en el chat:
   - Windows: *Inicio > "Editar las variables de entorno para tu cuenta" > Nueva* (nombre `OPENAI_API_KEY`).
     Evita escribirla en la consola, donde queda en el historial. Abrir una terminal nueva después.
   - Linux/macOS: `export OPENAI_API_KEY=...` en `~/.bashrc` / `~/.zshrc` (en fish: `set -Ux OPENAI_API_KEY (wl-paste)`).
   - **Codex**: por defecto filtra las variables con `KEY` en el nombre y bloquea la red de su sandbox, así que
     el doctor puede decir "falta la clave" aunque tu terminal la tenga. En `~/.codex/config.toml` agrega
     `[shell_environment_policy]` con `ignore_default_excludes = true` y `[sandbox_workspace_write]` con
     `network_access = true`, y reinicia Codex.
   - **Linux con Blockbench en Flatpak**: el servidor MCP sigue en `localhost:3000`, pero Blockbench solo puede
     escribir en carpetas XDG (Descargas, Documentos); la skill exporta ahí y luego mueve los archivos a `art/<slug>/`.
5. Comprobar todo (no gasta crédito) desde la raíz del repo:
   ```sh
   python .agents/skills/voxel-model/scripts/voxel_doctor.py
   ```
   Cada comprobación fallida trae una línea `fix` con qué hacer.

### Usarla

Con Blockbench abierto, desde la raíz del repo: en **Claude Code** `/voxel-model un golem de musgo`; en
**Codex** `$voxel-model un golem de musgo`. El agente irá pidiendo su OK en cada paso (referencia, geometría,
texturas, animaciones). Al terminar: `git add art/<slug>` y commit.

Si editan la skill, la fuente es `.agents/skills/voxel-model/`; luego `python tools/sync_skills.py` actualiza
la copia de Claude Code (`.claude/skills/`). Tests de la skill (sin red ni costo):
`python -m unittest discover -s .agents/skills/voxel-model/tests`.
