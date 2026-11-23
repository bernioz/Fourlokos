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
