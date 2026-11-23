# Fourlokos

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
