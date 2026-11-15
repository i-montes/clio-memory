# clio-memory estable: uno solo, privado y con un sueño barato

Fecha: 2026-11-15 · Estado: borrador para aprobar · Versiones objetivo: v0.9.1 → v0.13.0 → v1.0.0

Este documento es el diseño. Los planes de implementación son cinco, uno por entrega, en `docs/superpowers/plans/2026-11-15-estable-{1..5}-*.md`. Si un plan y este documento chocan, manda este documento.

## Intención

Iván revisó el proyecto el 2026-11-15 y pidió, con sus palabras:

- **Uno solo.** «Solamente hay un worker, solamente hay un modelo, solamente hay una base de datos, solamente hay un MCP.» Con cinco sesiones abiertas había cinco modelos y cinco procesos saturando la máquina. «No necesito que se activen varios MCP; si dejara de ser un MCP y pasara a ser una skill, también podría funcionar.»
- **Sin telemetría de Microsoft.** Que quede apagada por defecto «cuando nosotros lo descarguemos o cuando clio-memory lo utilice».
- **El sueño, cuando no se usa.** En vez de de noche: por ejemplo, tras 30 minutos sin uso. Y **barato en tokens**.
- **Temis en lugar de Jev** como modelo de decisión, servido desde su servidor por URL.
- Una versión **estable y lista para usar**, con un plan exacto que otro modelo pueda ejecutar.

## Lo que se midió (2026-11-15, v0.9.0, Ryzen 5700X de 16 hilos)

| Hecho | Medida |
|---|---|
| RAM por sesión MCP que busca o guarda | ~500 MB (pico 640–770) |
| RAM que queda tras soltar el modelo a los 60 s | 250–320 MB hasta que el proceso muere |
| 1 / 3 / 5 / 8 sesiones a la vez | 0,5 / 1,5 / 2,5 / 4,0 GB |
| Rendimiento total con 3 o más sesiones | ~13 búsquedas/s (no sube con más procesos) |
| 3 sesiones frías guardando a la vez | 1 rechazada («otro proceso está escribiendo»); con 5, 3 |
| Búsqueda tibia con 3 lugares / con 13 lugares | 42 ms / 106–148 ms |
| Telemetría de ONNX Runtime 1.31.1 | conexión HTTPS a Microsoft ~7 s tras cargar el modelo; `with_telemetry(false)` **no** la evita; `ORT_DISABLE_TELEMETRY=1` sí |
| Exportar e importar la memoria real | `import` la rechaza: los ids purgados se reutilizaron |
| Último sueño completo | 2026-11-09 (el equipo se apaga de noche) |
| Temis en `https://api.imontes.dev/temis` | `temis-2.4.0`, ~0,5 s por llamada, 0 tokens de LLM |

## Objetivos

1. Ningún proceso de clio-memory abre una conexión que no sea la que la persona configuró (Temis, las descargas de `setup`).
2. Con N sesiones de agente hay **un** proceso con el modelo y **un** escritor; las sesiones cuestan como mucho unos MB cada una, o nada.
3. El sueño corre cuando la memoria lleva 30 minutos sin uso y hay algo que revisar, a cualquier hora, y por sí solo no gasta tokens de LLM.
4. Las decisiones sobre pares de recuerdos las toma Temis por HTTP; si Temis no está, el sueño sigue con reglas y vectores.
5. Lo que se exporta se puede importar; lo que se purga no deja rastro de su texto ni de su título.
6. Cada entrega deja el programa funcionando y se publica por separado.

## No objetivos

- Compilar un ONNX Runtime propio sin telemetría. Se usa el oficial con la variable que la apaga.
- MCP por HTTP. Pierde el proyecto por carpeta y depende de cada agente.
- Cambiar Clio. Clio vive en otro repositorio; aquí se lista lo que tendrá que cambiar (sección «Qué cambia para Clio»).
- Que el sueño escriba textos unidos sin un modelo de lenguaje. Unir reescribiendo sigue siendo un paso opcional del anfitrión (Clio).
- Windows.

## Antiobjetivos (si pasa esto, falló aunque «funcione»)

- Que ONNX Runtime se cargue alguna vez con la telemetría encendida, también cuando lo usa un programa anfitrión.
- Que apagar o perder el demonio deje a un agente sin memoria: siempre hay camino dentro del propio proceso.
- Que el sueño frecuente gaste tokens cada vez: el paso con modelo de lenguaje sigue siendo, como mucho, uno cada 20 horas.
- Que un recuerdo se pierda porque Temis se equivocó: lo que Temis decide solo manda a cuarentena (reversible 30 días).
- Que el token de Temis quede en la línea de órdenes, en el registro, en un informe o en la memoria.
- Que una memoria existente (la de Iván, con ids reutilizados y títulos de purgados) quede sin poder abrirse, exportarse o importarse.

## Enfoques considerados

### Cómo llega la memoria a cada agente

| Enfoque | Qué es | A favor | En contra |
|---|---|---|---|
| A. Demonio + cliente MCP delgado | `clio-memory mcp` sigue, pero solo reenvía cada llamada a un proceso único | Cero cambios en cómo se conecta cada agente; funciona en agentes con sandbox (el MCP corre fuera) | Sigue habiendo un proceso por sesión (≈6 MB, sin modelo ni base) |
| B. Demonio + skill y CLI, sin MCP | El agente corre `clio-memory m …` en su shell, guiado por la skill | Cero procesos por sesión | En agentes cuyo shell corre en sandbox (Codex por defecto) la orden no puede llegar al socket ni a la base; cada agente pide permiso por orden si no se le autoriza; las herramientas ya no aparecen como tales |
| **C. Los dos sobre el mismo demonio** | El demonio atiende un protocolo de una petición; lo usan el cliente MCP delgado y los verbos de la CLI | Cada agente usa el camino que le funciona; se llega a «sin MCP» donde se puede y sin romper donde no | Dos modos de conexión en `setup` |

**Elegido: C, en dos entregas.** Primero el demonio con el cliente MCP delgado (v0.10): resuelve el consumo sin cambiar nada de la conexión. Después la skill con la CLI (v0.11), que pasa a ser el modo por defecto en Claude Code y OpenCode; Codex, Gemini CLI y Antigravity siguen por el cliente MCP delgado hasta que Iván verifique en cada uno que la orden corre sin fricción, y se cambian con una bandera.

### Quién despierta el sueño

- *El demonio, con un reloj interno.* Sabe exactamente cuándo no hay uso, pero tendría que quedarse vivo 30 minutos reteniendo memoria solo para eso, y no cubre a quien usa la biblioteca sin demonio.
- **Elegido: el timer que ya existe, a toda hora.** Corre cada 30 minutos todo el día; el sueño decide solo si le toca (30 minutos sin uso y algo que revisar). El uso por lecturas se anota en un archivo (`sleep/last-use`) que escribe quien atiende las llamadas. No necesita el demonio y recupera solo los días en que el equipo estuvo apagado.

### Cómo se apaga la telemetría

- *Solo la API (`with_telemetry(false)`).* Medido: el runtime abre igual su base de eventos y se conecta.
- *Un ONNX Runtime compilado sin telemetría.* Lo único que la quita del binario, pero obliga a compilar y publicar el runtime para cuatro plataformas.
- **Elegido: `ORT_DISABLE_TELEMETRY=1` antes de cargar la biblioteca, y negarse a cargarla sin ella.** El binario se vuelve a ejecutar a sí mismo con la variable puesta (`exec`, sin `unsafe`); la biblioteca no inicia ONNX Runtime si la variable no está. Así vale también para Clio.

### Cómo se abarata el sueño

- *Seguir uniendo con el modelo de lenguaje cada vez que duerme.* Con sueños frecuentes multiplica el gasto.
- **Elegido: tres capas.** (1) Reglas y vectores, gratis, en cada sueño. (2) Temis decide sobre los pares parecidos: si uno contiene al otro, el contenido se dobla sin escribir nada nuevo; si además ve una contradicción, el par no se dobla y queda para revisar. Cuesta llamadas a su servidor, no tokens. (3) Unir reescribiendo con el modelo de lenguaje queda como paso del anfitrión, como mucho una vez cada 20 horas, con su tope de tokens.

## Diseño

### 1. Telemetría (v0.9.1)

- `clio_memory::private_runtime() -> Result<(), String>`: si `ORT_DISABLE_TELEMETRY` vale `1`, no hace nada; si no, reemplaza el proceso por sí mismo con la variable puesta. Solo vuelve si no pudo.
- El binario la llama al empezar `main`. Un anfitrión (Clio) debe llamarla al empezar el suyo.
- `granite::env()` no inicia ONNX Runtime si la variable no vale `1`: devuelve un error que nombra la variable, y la memoria sigue con el modelo estático o por texto. Además pasa `with_telemetry(false)`.
- Las pruebas y ejemplos del repositorio corren con la variable puesta por `.cargo/config.toml`.
- Prueba de aceptación (Linux, con el modelo): un `clio-memory mcp` que ya vectorizó no tiene abierto ningún archivo `onnxruntime.db` y su entorno lleva la variable.

### 2. Integridad (v0.9.1)

- **Ids.** El siguiente id es uno más que el mayor id que haya existido (recuerdos vivos y purgados), no el mayor vivo. `import` acepta que un id purgado vuelva a nacer en una operación posterior cuyo cambio no tiene «antes»: son los archivos que ya existen.
- **Purga.** Al purgar se vacían también el título y la categoría de todas las instantáneas del recuerdo. Para los archivos que ya tienen purgados con título, el sueño los limpia al despertar y `doctor` lo marca hasta entonces. `import` limpia en vez de rechazar.

### 3. El sueño por inactividad (v0.9.1)

- El timer corre cada 30 minutos, todo el día (systemd `OnCalendar=*:0/30`; launchd `StartInterval` 1800).
- Duerme si: nadie escribió ni leyó en 30 minutos, **y** hay algo que revisar (operaciones ajenas después del cursor, pendientes, reglas nuevas) **o** pasaron 24 horas desde el último sueño completo (copia y purga del día).
- Las lecturas cuentan como uso: quien atiende una llamada anota la hora en `<datos>/sleep/last-use`, como mucho una vez por minuto.
- El paso con modelo de lenguaje (`Merger`) corre como mucho una vez cada 20 horas (`last_merge` en el estado); los demás sueños dejan esos grupos pendientes.
- Los sueños de un mismo día suman en un solo informe y comparten la copia del día.
- `doctor` falla, en vez de informar, si hay cambios sin revisar y el último sueño completo tiene más de 3 días.

### 4. Un solo proceso: el demonio (v0.10.0)

- `clio-memory daemon`: un proceso por carpeta de datos. Escucha en `<datos>/daemon.sock` (0600, carpeta 0700; `CLIO_MEMORY_SOCK` la cambia). Lo garantiza único un candado `flock` en `<datos>/daemon.lock`.
- **Nadie lo instala ni lo arranca a mano.** El primer cliente que no lo encuentra lo lanza, separado de su grupo de procesos, y reintenta. Sale solo tras 10 minutos sin peticiones: es lo único que devuelve toda la RAM.
- **Protocolo: una petición por conexión**, una línea JSON de ida y una de vuelta:
  - `call`: cliente, sesión, carpeta, herramienta y argumentos → el texto de la respuesta.
  - `embed`: textos → vectores (para anfitriones como Clio).
  - `status`, `shutdown`.
  - Cada petición lleva la versión del binario. Si el cliente es más nuevo que el demonio, el demonio contesta `stale` y sale; el cliente lanza el suyo y reintenta. El más nuevo gana.
- El demonio tiene **un** `EmbedModel` y una tabla de sesiones por (cliente, sesión). Atiende las llamadas de herramientas de una en una.
- `clio-memory mcp` contesta él mismo `initialize`, `tools/list` y `ping`, y manda cada `tools/call` al demonio. No abre la base ni carga ONNX Runtime. Si no puede hablar con el demonio, atiende dentro de su proceso como hoy y lo dice una vez por stderr.
- **Vectores fuera del candado.** Guardar suelta el escritor antes de vectorizar; los vectores se calculan con una lectura y se guardan en una sección corta. Vale en todos los modos.
- **Presupuestos que se reponen.** Los topes por sesión (20 lecturas o 24 KiB, 30 guardados, 10 cambios) pasan a ser por ventana de 30 minutos.
- Banco: `bench/sessions/` mide N sesiones reales. Aceptación con 5 sesiones: un solo proceso con modelo, menos de 700 MB en total y ningún guardado rechazado.

### 5. Skill y CLI (v0.11.0)

- Verbos del agente: `clio-memory m <verbo> …` (`context`, `search`, `get`, `timeline`, `save`, `update`, `forget`, `pin`, `consolidate`, `summary`, `revert`, `project`; `unpin` llega con v0.13). Cada uno es una petición `call` al demonio (o dentro del proceso si no hay). El texto largo entra por la entrada estándar.
- Las órdenes que actúan con la autoridad de la persona (`purge`, `promote`, `archive`, `revert`, `remember` sin actor, `uninstall --data`…) se niegan cuando quien llama es un agente conocido, salvo que la persona ponga `CLIO_MEMORY_AS_PERSON=1`. No es una barrera de seguridad (un agente con shell puede ponerla): evita el accidente.
- Quién llama: `--agent`, si no `CLIO_MEMORY_AGENT`, si no el entorno (`CLAUDECODE`), si no el nombre de un proceso antecesor conocido; la sesión sale de `CLAUDE_CODE_SESSION_ID` o del pid de ese antecesor. El actor sigue llamándose `mcp:<agente>` para no partir el historial.
- Una sola skill sirve a los dos modos: usa las herramientas `memory_*` si existen y, si no, las órdenes.
- `setup` guarda el modo de cada agente en `<datos>/agents.json`. Por defecto: skill en Claude Code y OpenCode; MCP delgado en Codex, Gemini CLI y Antigravity. `--mode skill|mcp` lo cambia.
- En modo skill, a Claude Code se le agrega la regla de permiso `Bash(clio-memory m:*)`.

### 6. Temis (v0.12.0)

- Contrato verificado el 2026-11-15: `POST <url>/v1/decide` con `Authorization: Bearer <token>`, cuerpo `{"model":"temis-latest","state":{…},"questions":{…}}`, respuesta `{"answers":{"<id>":{"type":"noul","noul":0.79}}}` o `{"type":"choice","choice":"…","confidence":0.97,…}`. Errores: `{"detail":{"error_type","message"}}` con 400, 401, 413, 429 o 503.
- Configuración en `<datos>/temis.json` (0600): `url`, `token`, `model`. La pone `clio-memory setup temis --url URL` leyendo el token por la entrada estándar. Solo `https`, salvo `localhost`.
- La llamada se hace con `curl --config -` (el token nunca va en la línea de órdenes), 10 s de límite.
- Dos preguntas, con el texto exacto con que Temis fue entrenado:
  - `memory_preserved` — estado `{original, unido}`, pregunta `se_conserva` (noul): «Todo lo que dice `original` está también en `unido`, aunque sea con otras palabras.»
  - `memory_relation` — estado `{propuesta, otra}`, pregunta `relacion` (choice) con `conflicts_with`, `scoped`, `compatible`, `related`, `ninguna`.
- Medido el 2026-11-15 con el servicio real sobre los 28 pares de `bench/recall` (`bench/temis/results.md`): con `se_conserva` ≥ 0,80 en algún sentido se doblan 8 de 12 paráfrasis y ninguno de los 8 pares que solo se parecen (el mayor queda en 0,36). Su `conflicts_with` no es de fiar: 3 falsos en 20 pares sin contradicción, dos de ellos con confianza de 0,91 y 0,96.
- En el sueño, para cada recuerdo nuevo y sus vecinos entre el piso de casi duplicados del modelo (0,86 con el cuantizado, 0,87 con el completo) y 0,97, del mismo lugar, tipo y confianza:
  - Si `se_conserva` ≥ 0,80 en un sentido, las cifras y nombres del contenido están en el que queda, y `relacion` no dice `conflicts_with`: el contenido va a cuarentena («sueño: contenido en mN») y sus relaciones pasan al otro.
  - Si `se_conserva` ≥ 0,80 pero `relacion` dice `conflicts_with` (confianza ≥ 0,50): no se dobla; el par sale en «para revisar».
  - En cualquier otro caso no se hace nada. **El sueño no anota relaciones de Temis.**
- Tope de 40 pares por sueño; lo demás queda pendiente. Si Temis falla tres veces seguidas, el paso se deja para el siguiente sueño con una alarma.
- Antes de enviar un texto se comprueba que no parezca un secreto.

### 7. Endurecimiento (v0.13.0)

- **Anclas:** herramienta y verbo `unpin`; el contexto muestra las anclas del proyecto, de la persona y de las lecciones, no las de otros proyectos; como mucho 5 por lugar.
- **Detector de secretos:** deja de rechazar rutas, URL de commits, nombres de rama y de clase; reconoce contraseñas en cadenas de conexión, en `CLAVE=valor`, bloques PGP, `glpat-`, cabeceras `Authorization`.
- **Olvido:** el aviso dice la verdad («se borra a los 30 días si nadie lo restaura») y el README también.
- **Proyecto:** dos carpetas con el mismo nombre y distinto repositorio no comparten proyecto.
- **Salida:** se quitan los caracteres invisibles que hoy solo ve el sueño, y se desarman los parecidos al marco y a la cabecera de un recuerdo.
- **Vectores viejos:** el sueño borra los de modelos que ya no se usan.
- **CI:** un trabajo que baja Granite y corre la ruta real (vectorizar, MCP de ida y vuelta, sin telemetría).

## Lo que decidió Iván

- Telemetría de Microsoft apagada por defecto.
- El sueño corre cuando la memoria no se usa (30 minutos), no de noche, y debe ser barato en tokens.
- Temis en lugar de Jev como modelo de decisión, por URL.
- Uno solo: un worker, un modelo, una base; acepta dejar de ser MCP y ser una skill.
- El plan lo ejecutará otro modelo.

## Lo que decidió Claude (para que Iván lo confirme o lo cambie)

1. El enfoque C en dos entregas, y los modos por defecto de cada agente (sección 5).
2. El timer a toda hora en vez de un reloj dentro del demonio.
3. Negarse a cargar ONNX Runtime sin la variable, en vez de solo intentar apagar la telemetría.
4. Temis solo dobla (a cuarentena, reversible 30 días); nunca reemplaza, purga ni anota relaciones. Umbral 0,80, medido en `bench/temis/results.md`; el umbral 0,90 que Clio usa con Jev doblaría 2 de 12 paráfrasis.
5. El aviso «Anoche (sueño): …» conserva su texto aunque el sueño ya no sea nocturno: cambiarlo rompe pruebas de Clio y no cambia lo que se hace.
6. El olvido a 30 días se mantiene y se dice; no se alarga.
7. El orden: v0.9.1 urgentes, v0.10 demonio, v0.11 skill, v0.12 Temis, v0.13 endurecimiento.

## Qué cambia para Clio (otro repositorio, fuera de estos planes)

- Al subir a v0.9.1: llamar a `clio_memory::private_runtime()` al empezar `main`; volver a instalar su timer (`clio memory setup sleep`) para que tome el horario nuevo; su `doctor` y sus textos de «noche» siguen valiendo.
- Al subir a v0.10: puede pedir los vectores al demonio (`clio_memory::daemon::RemoteEmbedder`) en vez de cargar Granite en cada proceso.
- Al subir a v0.12: su `Merger` sigue igual. Para que Clio pregunte a Temis en vez de a Jev basta con que su adaptador acepte la URL de Temis (hoy solo admite `api.typesafe.ai` o local): Temis sirve el mismo formato en `/v1/decide`.

## Secuencia y criterio de 1.0

| Entrega | Plan | Lo que deja |
|---|---|---|
| v0.9.1 | `…-estable-1-urgentes.md` | Sin telemetría, exportación restaurable, purga limpia, sueño por inactividad, `update` fijado al release, búsqueda por título y una vectorización por consulta |
| v0.10.0 | `…-estable-2-demonio.md` | Un modelo y un escritor para todas las sesiones |
| v0.11.0 | `…-estable-3-skill.md` | Memoria por skill y CLI, sin proceso por sesión donde se pueda |
| v0.12.0 | `…-estable-4-temis.md` | El sueño decide con Temis, sin tokens |
| v0.13.0 | `…-estable-5-endurecer.md` | Anclas, secretos, olvido, proyecto, salida, CI con el modelo real |

v1.0.0 se etiqueta cuando: las cinco entregas están publicadas; `bench/sessions` cumple la aceptación; el banco de recall no baja de 97 % entre los 5 primeros; y pasó una semana de uso real de Iván con el sueño corriendo cada día sin alarmas.

## Hallazgos de la auditoría que quedan fuera de estas entregas

Verificados o leídos el 2026-11-15; ninguno impide usar la memoria. Quedan anotados para después de v1.0 o para cuando duelan:

- **Tamaño y escala.** Cada vector ocupa una página de desborde propia (~4,7 KB por 1,5 KB de datos; `src/sqlite/schema.rs:37`). `stats()` corre dos veces por guardado, `recent` y `timeline` ordenan un lugar entero sin índice, cada purga hace `VACUUM` de todo el archivo (`src/sqlite/mod.rs:1698`). No importa con cientos de recuerdos; sí desde unos 10.000.
- **Compatibilidad dentro del esquema 2.** Se agregaron `Category::Profile`, `OpKind::Forget` y `OpKind::Move` sin subir la versión: un binario anterior falla al leer esas filas (`src/model.rs:13`).
- **Importar es todo o nada con el detector de secretos.** Si el detector se vuelve más estricto, una exportación vieja puede no importarse (`src/sqlite/mod.rs:492`). El plan 5 lo afloja, pero no hay forma de saltarlo.
- **El uso no se registra desde las lecturas de un agente** (0 filas en `usage`): el refuerzo por uso y el «más usado» del sueño no ven esas lecturas (`src/engine.rs:2657`).
- **Revertir un olvido puede dejar dos recuerdos activos del mismo tema** (`src/engine.rs:2053`).
- **`CLIO_MEMORY_DATA` no llega al timer.** Quien la pone solo en su shell tiene el sueño mirando la carpeta por defecto (`src/setup/mod.rs:282`).
- **`setup` puede quitar un binario de `cargo install` al que otro agente sigue apuntando** (`src/bin/clio-memory/setup_flow.rs:445`); `update` mueve a `~/.local/bin` una instalación hecha en otra carpeta (`src/setup/mod.rs:42`).
- **Los archivos JSON de los agentes se editan sin copia de respaldo** y no admiten comentarios (`src/setup/config_file.rs`).
- **La descarga del modelo no se reanuda** y calcula el sha256 leyendo el archivo entero en memoria (`src/setup/mod.rs:497`).
- **El sueño podría doblar como repetidos dos textos largos que difieren en una palabra sin cifras** («activado» / «desactivado») a 0,97 o más: sospecha sin medir (`src/sleep/mod.rs:517`).
- **«Anoche (sueño): …»** conserva su texto aunque el sueño ya no sea nocturno.

## Lo que queda al constructor

- Los nombres internos que los planes no fijan, el orden de los `match`, los mensajes de error que no se citan literalmente.
- El texto final de la skill, respetando las reglas de seguridad que ya tiene.
- La forma exacta del informe de `bench/sessions/results.md`.
