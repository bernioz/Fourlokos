# Progresión jugable — primera versión

## Objetivo

Convertir el sandbox actual de planetas, construcción y viaje espacial en una aventura corta con tres mundos conectados por objetivos claros. La progresión debe enseñar a sobrevivir, construir y viajar sin quitar la libertad de construir que ya tiene Fourlokos.

## Estado actual del repositorio

El juego ya genera planetas y biomas, cuevas, líquidos y vetas de carbón, cobre, hierro, oro, redstone y diamante. También tiene bloques para cohetes, pads, estaciones, elevadores espaciales, viaje orbital y warp. El cohete actual se arma con bloques y requiere al menos un motor; para despegar necesita un pad natural plano de 3×3 u otro pad compatible.

En el código revisado todavía no hay un sistema de recetas/progresión, entidades de fauna ni reproducción de audio. Los árboles actuales son árboles de madera; palta, tomate, trigo, gallinas, huevo, comida y los recursos tecnológicos propuestos abajo son contenido nuevo. La primera versión debe añadir estos sistemas sin asumir que ya existen.

## Bucle general

1. Explorar el mundo y recolectar comida y materiales.
2. Fabricar herramientas y una base sencilla.
3. Encontrar el recurso distintivo del mundo y usarlo para mejorar el vehículo/equipo.
4. Preparar y completar el viaje al siguiente mundo.
5. Traer recursos nuevos de vuelta: los mundos anteriores siguen siendo útiles.

Los tres mundos son la campaña guiada para la primera versión. El universo procedural actual puede seguir disponible como modo libre; la campaña usa tres destinos definidos y recetas conocidas para que el avance sea comprobable.

## Mundo 1 — Mundo Verde (inicio)

**Identidad:** mundo templado y habitable, con praderas, bosque y costa. Es el espacio para aprender a sobrevivir y construir.

**Fauna y flora obligatorias:** árboles de palta; plantas de tomate cultivables; gallinas que pongan huevos. Añadir trigo para hacer pan. Las gallinas deben aparecer en grupos pequeños, poner huevos en el mundo y poder mantenerse dentro de corrales sencillos. Palta y tomate se cosechan y vuelven a crecer; así el completo no depende de hallazgos únicos.

**Recursos:** madera, hojas, piedra, carbón, cobre e hierro; agua; semillas de trigo y tomate; paltas; huevos. El hierro y el cobre abren la primera tecnología. El carbón sirve de combustible de fundición.

**Criaturas v1:** gallina (pasiva, suelta plumas y pone huevo); slime pequeño (amenaza de combate sencilla, suelta gel). Mantener pocos comportamientos: caminar, huir/atacar, recibir daño y soltar objeto. El slime existente en `art/` puede servir como primer modelo, pero todavía hay que conectarlo al juego.

**Objetos/comida:** hacha y pico básicos, horno, mesa de trabajo, cerca/corral, semillas, harina o trigo molido, pan, huevo, tomate, palta, carne/salchicha, aceite, mayonesa y completo italiano.

**Meta de mundo:** construir una estación de trabajo y un cohete funcional, conseguir combustible y despegar hacia Mundo 2.

**Receta estrella — completo italiano:** pan (dos mitades), salchicha cocida, tomate, palta y mayonesa. Para mantener el alcance de la primera versión, la salchicha se fabrica con carne de una criatura pasiva introducida en este mundo (cerdo) o, si no se alcanza a hacer fauna adicional, con una receta provisional de carne cultivada/comprada. La mayonesa se hace con huevo + aceite. La receta debe consumir una unidad de cada ingrediente y restaurar bastante hambre; no es un requisito para viajar, pero sí el objetivo de cocina emblemático y una forma de dar uso real a gallinas, cultivos y aceite.

**Cadena agrícola mínima:** semilla de tomate → planta → tomate; semilla de trigo → trigo → harina → pan; palta → árbol → cosecha recurrente; gallina → huevo; huevo + aceite → mayonesa. Para el aceite v1, prensar semillas de girasol (añadir girasol como cultivo secundario) o usar una receta simplificada de huevo + aceite vegetal fabricado con semillas.

## Mundo 2 — Mundo Rojo (intermedio)

**Identidad:** planeta seco y volcánico, con cañones, cuevas calientes y tormentas de polvo. El jugador llega preparado, pero debe instalar una base temporal y gestionar agua/temperatura.

**Fauna y flora:** escarabajo de ceniza (hostil, lento, suelta quitina); cactus y una planta resistente con semillas. Sin fauna compleja ni agricultura completa en esta versión.

**Minerales:** hierro y cobre siguen presentes; añadir azufre y cristal de basalto como recursos distintivos. Carbón escaso. El azufre y la quitina sirven para filtros y protección térmica; el cristal de basalto se usa en el núcleo de salto. Reutilizar vetas/blocks de basalto existentes cuando sea posible y añadir solo los drops/ítems que requiera la progresión.

**Objetos:** filtro de polvo, cantimplora/celda de agua, traje térmico, carga de combustible y núcleo de salto v1. El equipo reduce daño ambiental, no lo elimina por completo.

**Meta de mundo:** localizar una veta de cristal de basalto, extraerla con herramienta de hierro y fabricar el estabilizador de salto. Ese componente habilita el viaje al Mundo 3.

## Mundo 3 — Mundo Azul (final de la campaña v1)

**Identidad:** mundo frío, con océanos congelados, cuevas profundas y cristales luminosos. Es la prueba final y recompensa la exploración con un recurso avanzado.

**Fauna y flora:** criatura cristalina pequeña (hostil, suelta fragmentos); ave polar pasiva que puede dar plumas. Flora escasa bajo hielo. Si el presupuesto es ajustado, lanzar solo la criatura cristalina.

**Minerales:** hierro y carbón en profundidad, diamante/redstone ya existentes; añadir cristal azul como recurso exclusivo de Mundo 3. El diamante y el cristal azul se usan para el núcleo estable; redstone para el circuito de control.

**Objetos:** abrigo aislante, antorcha mejorada, taladro de cristal y núcleo estable. El núcleo permite activar un portal/estación de retorno y completa la campaña; no requiere generar un cuarto mundo.

**Meta final:** explorar una cueva de hielo, obtener cristal azul, fabricar el núcleo estable y activar el retorno a la estación de origen. Mostrar un cierre de campaña y dejar el universo abierto para seguir construyendo.

## Requisitos para pasar de un mundo a otro

| Tramo | Requisitos del jugador | Requisitos del vehículo | Resultado |
|---|---|---|---|
| Mundo 1 → Mundo 2 | Banco de trabajo, horno y materiales para preparar combustible | Cohete ensamblado con helm, al menos un engine, pad despejado y carga de combustible | Llegada al Mundo Rojo |
| Mundo 2 → Mundo 3 | Traje térmico y filtro; extraer cristal de basalto | Instalar núcleo de salto/estabilizador en el cohete y cargar combustible | Llegada al Mundo Azul |
| Mundo 3 → cierre v1 | Abrigo aislante; obtener cristal azul y redstone | Fabricar el núcleo estable y activarlo en la estación/portal de retorno | Cierre de campaña y modo libre |

La comprobación del viaje debe ser explícita en UI: mostrar los materiales que faltan, dónde se consiguen y qué receta los usa. No bloquear el modo creativo. Para evitar softlocks, el combustible y los materiales de reparación deben obtenerse en el mismo mundo o poder traerse del anterior; el cohete debe poder regresar desde cada destino.

## Alcance de implementación recomendado

**Imprescindible para la primera versión:**

- Catálogo pequeño de ítems no bloque (comida, semillas, huevo, combustible, materiales nuevos) y recetas.
- Inventario compatible con los nuevos ítems; persistencia de objetos y cultivos en partida guardada.
- Agricultura de tomate y trigo, árboles cosechables de palta, girasol para aceite y gallinas que pongan huevos.
- Hambre simple y comida que la restaure, incluido el completo italiano.
- Tres destinos de campaña fijos, registro de objetivos completados y desbloqueo de recetas/viajes entre ellos.
- Dos o tres criaturas con navegación y combate básicos; empezar por gallina y slime.
- Audio esencial de UI, pasos, minería/colocación, gallina, cocina y viaje.

**Dejar fuera de v1:** crianza genética, aldeanos, cadenas industriales largas, decenas de mobs, diálogos extensos, doblaje, música dinámica compleja y simulación realista de gases/temperatura.

## Audio del juego

No encontré reproducción de audio ni una carpeta de sonidos en el cliente revisado. La dependencia de Bevy puede habilitar audio, pero falta crear el sistema, añadir archivos y disparar eventos desde las acciones del juego.

**Paquete mínimo de audio v1:**

- Ambiente en loop, distinto para cada mundo (pradera, polvo/volcán, hielo), bajo en la mezcla.
- Efectos cortos: pasos por material, romper/poner bloque, UI/menú, horno/cocina, huevo al caer, cacareo, daño de slime/escarabajo, cohete y warp.
- Una pieza musical breve de menú y una capa ambiental por mundo; sin diálogo ni voces.
- Control de volumen general, música, ambiente y efectos; silenciar funciona también al pausar.

**Cómo producirlo:** grabar Foley propio (madera, piedra, metal, viento, utensilios de cocina) y procesarlo; crear capas ambientales con síntesis sencilla; encargar o componer tres loops cortos si se quiere identidad musical. También se pueden usar recursos CC0/licencia compatible, guardando fuente, licencia y atribución en `crates/client/assets/audio/README.md`. Normalizar y exportar efectos cortos en OGG o WAV; preferir OGG para loops. No usar música comercial ni voces generadas sin derechos claros.

**Integración técnica propuesta:** activar el soporte de audio de Bevy si no viene en las features actuales; cargar sonidos como assets; emitir eventos de gameplay (`BlockBroken`, `ItemCrafted`, `CreatureCall`, `RocketLaunch`, `WarpStarted`) y mapearlos a AudioPlayer desde un plugin `audio`. Mantener música/ambiente como canales con volumen independiente y efectos con variación leve de tono/volumen para evitar repetición robótica. En v1 basta un sistema sencillo de mezcla por distancia y tipo de superficie; no hace falta middleware externo.

## Decisiones que conviene fijar antes de programar

1. ¿La campaña reemplaza el mundo procedural inicial o se elige desde el menú junto al modo libre? Recomendación: añadir “Campaña” y conservar “Mundo libre”.
2. ¿El cohete existente se desbloquea desde el inicio o requiere fabricarse con recetas? Recomendación: está disponible como construcción, pero sus motores y combustible sí requieren progresión.
3. ¿El completo italiano da una ventaja temporal (más hambre restaurada) o es principalmente un logro/receta cultural? Recomendación: recuperar hambre y dar un pequeño bonus de energía, sin ser obligatorio para viajar.

## Orden de trabajo

1. Definir IDs de ítems, drops y recetas; validar guardado/inventario.
2. Implementar cultivos, árbol de palta y gallina; cerrar la receta del completo italiano.
3. Añadir hambre y UI mínima de recetas/objetivos.
4. Implementar gates de campaña y destinos fijos, aprovechando cohete y warp actuales.
5. Añadir fauna restante y recursos distintivos de mundos 2 y 3.
6. Integrar audio de acciones esenciales y controles de mezcla.
7. Probar una partida completa desde el spawn hasta el retorno, además del guardado/carga en cada mundo.
