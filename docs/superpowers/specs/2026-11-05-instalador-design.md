# Instalador de clio-memory: una línea, binarios compilados y una TUI con la marca de Clio

Fecha: 2026-11-05 · Estado: borrador para aprobar · Versión objetivo: v0.6.0

## Intención

Hoy instalar clio-memory pide tener Rust (`cargo install --git … --features model2vec,granite`), y después correr a mano `setup model` y `setup claude-code`. Además, solo sabe conectarse a Claude Code. Iván quiere:

- **Una sola línea**, `curl -fsSL <raw…>/install.sh | sh`, que instale el binario que sirve para la máquina. Si no hay binario para esa máquina, falla con un mensaje claro y no instala nada.
- **Ningún paso de entorno a mano.** Ya está acordado (memoria m16): clio-memory se baja y prepara solo lo que necesita, o llega compilado. Solo se descarga aparte lo pesado y externo (el modelo), y eso también es automático.
- **Un instalador bonito**, con la identidad de Clio. clio-memory es una extensión de Clio, el agente de Iván. Por eso el sello que se ve es **Clio**, y «memory» es lo que se instala.
- **Conectar la memoria a uno, a varios o a todos los agentes que haya:** Claude Code, OpenCode, Codex, Gemini CLI y Antigravity (el IDE y su CLI `agy`).

## Objetivos

1. Al etiquetar `vX.Y.Z`, un workflow verifica, compila y publica sin intervención binarios para 4 plataformas: Linux x86_64, Linux aarch64, macOS arm64 y macOS x86_64.
2. `install.sh` (POSIX `sh`) hace esto: detecta la plataforma, baja el binario de la última versión publicada (o de la que se pida), verifica su sha256 y, si hay `gh`, su atestación, lo deja en `~/.local/bin/clio-memory` y le pasa el control a `clio-memory setup`.
3. `clio-memory setup` (sin subcomando) abre la TUI. En ella se elige a qué agentes conectar la memoria, qué modelo de vectores usar y si se agrega la línea de instrucciones. Luego instala todo eso y el sueño nocturno.
4. La misma TUI sirve después para cambiar lo elegido. Correr la línea otra vez actualiza el binario y conserva lo elegido.
5. `clio-memory uninstall` deshace lo que hizo el setup.

## No objetivos

- Windows. No está entre las plataformas pedidas.
- Paquetes de distribución (Homebrew, AUR, .deb). La línea de `curl` es el camino; `cargo install` queda como alternativa documentada.
- Clio. Clio usa clio-memory como biblioteca, fijada por commit, y no pasa por este instalador.
- Actualizarse solo en segundo plano. Para actualizar se corre la línea otra vez.
- Una TUI de pantalla completa. Se eligió la opción A, el asistente en línea.

## Antiobjetivos (si pasa esto, falló aunque «funcione»)

- **Pedirle a la persona que instale algo**, como Rust, `gh`, `minisign`, ONNX Runtime o un paquete del sistema. Las únicas herramientas que se asumen son `sh`, `curl` y `uname`, más `sha256sum` o `shasum`, y `tar` si el binario va comprimido. Linux y macOS traen todas.
- **Instalar algo distinto de lo que se verificó**, o dejar a medias un binario que no pasó la verificación.
- **Tocar configuraciones ajenas.** No se tocan otros servidores MCP, otras skills ni otras líneas de los archivos de instrucciones. Algo llamado `clio-memory` que no es esto no se reemplaza sin el sí de la persona, o sin `--replace`.
- **Que un agente vea la skill de clio-memory dos veces** porque lee varias carpetas que comparte con otros.
- **Borrar la memoria** (`memory.db`) al desinstalar, salvo que se pida explícitamente.
- **Una TUI que se rompa sin terminal**: con `curl | sh` en un servidor, en CI o con la salida redirigida.

## Lo que decidió Iván

- **La forma:** `curl … | sh`. Si la máquina es compatible, se instala; si no, falla.
- **Las plataformas:** Linux x86_64, macOS arm64, Linux aarch64 y macOS x86_64.
- **El estilo:** la opción A, un asistente en línea al estilo clack. Los pasos van uno debajo de otro, unidos por una línea, y lo hecho queda arriba como registro.
- **La marca:** el encabezado dice **Clio** (ámbar, invertido) y luego **memory**. La maqueta aprobada empieza así:

  ```
  ┌  ▌Clio▐ memory · la memoria de tu agente · v0.6.0
  │
  ◇  Esta máquina · Linux x86_64 · NVIDIA RTX 4060
  │
  ◇  clio-memory instalado · ~/.local/bin · sha256 ✓
  │
  ◆  ¿A qué agentes le conecto la memoria?
  │  ◼ Claude Code   encontrado en ~/.claude
  │  ◼ OpenCode      encontrado en ~/.config/opencode
  │  ◻ Todos los que encuentre
  └  espacio marca · enter sigue
  ```

  Cierra con `└  Listo. Abre <agentes>: la memoria ya está ahí.` y, debajo, una línea en ceniza: `clio-memory doctor revisa todo · clio-memory setup cambia lo que elegiste`.

- **La paleta**, que es la del canal de YouTube de Iván. En la terminal se usa color real (truecolor):

  | Nombre | Hex | Uso en la TUI |
  |---|---|---|
  | hueso | `#E6E6E0` | texto principal |
  | cenizaClara | `#B8BCC4` | texto de apoyo (detalles, cifras) |
  | ceniza | `#8A8F98` | secundario: la línea guía, las ayudas, lo no marcado |
  | borde | `#2A2E35` | la parte vacía de las barras de progreso |
  | ámbar | `#FFB13D` | lo que espera respuesta (◆, la selección) y el sello «Clio» |
  | hielo | `#7FD6FF` | lo que está pasando (spinner, barra de descarga) |
  | verdeSuave | `#7FA88A` | paso hecho (◇, ✓) |
  | rojoApagado | `#B4554B` | error (✖) |

  La TUI no pinta el fondo: usa el de la terminal. Grafito (`#0D0F12`) y panel (`#13161B`) quedan para lo que se vea fuera de la terminal.

  Las fuentes (Space Grotesk y JetBrains Mono) no se pueden aplicar en una terminal, porque cada una usa la suya. Valen para cualquier página o imagen del proyecto, como el README o el SVG de arquitectura, que quedan fuera de este trabajo.

- **Los agentes:** Claude Code, OpenCode, Codex, Gemini CLI y Antigravity (`agy` y el IDE). La persona puede elegir uno, varios o «todos los que encuentre».

## Decisiones mías (Iván puede cambiarlas)

### Confianza en el binario y releases sin intervención

Iván pidió que publicar una versión no requiera nada de él más allá de decidir que sale. Eso quiere decir: sin PR de sumas que fusionar, sin release en borrador que publicar y sin secretos que crear o renovar.

Este es el criterio de confianza que queda, con sus límites dichos:

- **Releases inmutables.** El repositorio tiene activada la opción *immutable releases* de GitHub. Una vez publicado un release, ni sus assets ni su etiqueta se pueden cambiar ni borrar. Así, el binario y las sumas de una versión quedan fijos para siempre. Para colar un binario falso habría que publicar una versión nueva, y eso exige escribir en el repositorio y pasar el workflow. Activar la opción es un solo cambio de configuración (`gh api`). Lo hace el constructor con la cuenta de Iván y lo deja documentado. GitHub, además, firma una atestación del release.
- **Sumas en el release.** Cada release lleva `SHA256SUMS`. `install.sh` baja el binario y las sumas de la misma versión y compara. Eso detecta un archivo dañado o cortado. Que el archivo sea legítimo lo garantiza la inmutabilidad, no la suma.
- **Atestaciones de procedencia.** El workflow firma cada binario con `actions/attest-build-provenance`, que lo liga al commit y al workflow que lo construyó. Si la máquina tiene `gh`, `install.sh` corre `gh attestation verify`. **[Decisión mía]** Si `gh` dice que la atestación **no corresponde**, la instalación se detiene. Si `gh` no puede verificar (no está, no tiene sesión o no hay red), se sigue con una línea en ceniza que lo dice.
- **El límite, dicho claro.** Sin un humano en medio, la confianza termina en la cuenta de GitHub de Iván y en el workflow. Quien controle eso puede publicar una versión. Las sumas escritas en el repositorio y fusionadas a mano habrían agregado una revisión humana, pero Iván eligió no tener ese paso. Se deja anotado en `docs/releases.md`.

**`install.sh`** vive en `main` y no lleva versión ni sumas. Toma la última versión publicada desde `https://github.com/i-montes/clio-memory/releases/latest/download/…`, que es una redirección y no gasta la cuota de la API. Con `CLIO_MEMORY_VERSION=vX.Y.Z` se fija una versión. Para cambiar el script se pasa por un PR normal, así que el CI (`shellcheck` y pruebas) siempre corre sobre él.

**El flujo de un release** (`.github/workflows/release.yml`, al empujar una etiqueta `v*`):

1. Comprueba que la etiqueta coincide con la versión de `Cargo.toml`. Si no coincide, falla sin publicar nada.
2. Corre las mismas verificaciones del CI sobre ese commit: `fmt`, `clippy -D warnings`, `test --all-features`, `cargo deny`, `cargo audit`, `gitleaks` y `shellcheck`. Si alguna falla, no se publica.
3. Compila las 4 plataformas y prueba cada binario donde se puede ejecutar (`clio-memory --version` y una instalación de prueba con `install.sh` contra los archivos recién construidos).
4. Firma la procedencia y publica el release, ya no en borrador, con los binarios, `SHA256SUMS` y notas generadas a partir de los commits.

Si algo falla a mitad, no queda ningún release publicado. Como un release inmutable no se puede corregir, la versión mala se arregla publicando la siguiente. Los permisos del workflow son los mínimos por trabajo: `contents: write` solo en el que publica, e `id-token: write` y `attestations: write` solo en el que firma. No usa secretos más allá del `GITHUB_TOKEN`.

Lo único que hace una persona es empujar la etiqueta, y eso sigue siendo cuando Iván lo pide (memoria m2).

### Binarios

- Se compilan con las features `model2vec,granite`. `ort` está en `load-dynamic`, así que el binario no enlaza ONNX Runtime; `setup model` lo baja después.
- **En Linux se usa glibc, no musl**, porque ONNX Runtime es una biblioteca glibc que se carga en tiempo de ejecución. El binario debe arrancar en distribuciones con **glibc 2.28 o más nueva**: se compila contra una glibc vieja (por ejemplo con `cargo-zigbuild`) o en un contenedor viejo. El constructor elige cómo.
- Cada binario va como archivo suelto (`clio-memory-<versión>-<target>`) o en un `.tar.gz`; lo elige el constructor. La suma que se verifica es la del archivo que se baja.
- **macOS x86_64:** ONNX Runtime 1.31 no publica versión para Intel Mac, así que ahí no hay Granite. La TUI ofrece potion como modelo recomendado y explica por qué en una línea. Granite no aparece como opción.

### `install.sh`

- POSIX `sh`, limpio en `shellcheck`.
- Detecta la plataforma con `uname -s` y `uname -m`. Si no es ninguna de las 4, sale con un error y la lista de plataformas que sí hay, y no crea nada.
- Usa una carpeta temporal propia y la limpia al salir.
- Instala de forma atómica: baja, verifica y solo entonces hace `mv` sobre el binario anterior.
- Instala en `~/.local/bin`, o en `$CLIO_MEMORY_BIN_DIR` si está definida. **[Decisión mía]** Si esa carpeta no está en el `PATH`, no se edita ningún archivo de la shell. El cierre de la TUI muestra la línea exacta que hay que agregar, según la shell (`$SHELL`), y la ruta completa para correr `doctor` mientras tanto. El MCP y el timer usan la ruta absoluta, así que funcionan igual.
- Luego hace `exec "$bin" setup --from-installer <banderas>`, pasándole las banderas que recibió.
- **Teclado:** con `curl | sh`, la entrada estándar es el script, no el teclado. Por eso el setup lee la terminal desde `/dev/tty`. Si no hay `/dev/tty` utilizable, entra en modo no interactivo (ver abajo).
- La parte del script es sobria (unas líneas con el mismo sello y los mismos colores, sin dependencias). Lo bonito lo dibuja el binario.

### La TUI de `clio-memory setup`

- **Biblioteca:** `cliclack`, el estilo clack para Rust, con un tema propio que aplica la paleta. Debe pasar `cargo deny` con las licencias que ya están permitidas; si añade alguna, se justifica en `deny.toml` como las demás. Si `cliclack` no deja aplicar la paleta exacta, el constructor puede usar otra biblioteca o dibujarlo a mano. El aspecto de la maqueta manda, no la biblioteca.
- **Colores:** truecolor cuando `COLORTERM` es `truecolor` o `24bit`; si no, la aproximación de 256 colores. Sin color cuando `NO_COLOR` está definida o la salida no es una terminal.
- **Los pasos, en orden:**
  1. Encabezado con el sello.
  2. «Esta máquina»: sistema, arquitectura y GPU NVIDIA si la hay.
  3. «clio-memory instalado», solo cuando viene del instalador (`--from-installer`).
  4. **Agentes**, con selección múltiple. Solo aparecen los agentes encontrados en la máquina, cada uno con el lugar donde se encontró, y la opción «Todos los que encuentre». Si no hay ninguno, lo dice y sigue: el binario y el modelo sirven igual, y luego se puede correr `clio-memory setup`.
  5. **Modelo de vectores**, con selección única:
     - Granite 311M, recomendado, en GPU si la hay (1,25 GB);
     - potion, liviano (unos 500 MB);
     - ninguno (búsqueda por texto).

     Cada opción muestra su tamaño.

     **[Decisión mía]** No se guarda ninguna preferencia de modelo: el binario sigue usando el mejor que haya en disco (Granite, si no potion, si no texto). Por eso, elegir uno «menor» que el instalado ofrece **borrar** el mayor (Granite, junto con su runtime y las bibliotecas de CUDA), con confirmación y mostrando cuánto espacio libera. Si la persona dice que no, se queda el que estaba y la TUI lo dice. Al volver a correr la TUI, aparece elegido el mejor modelo instalado. En modo no interactivo, `--model` nunca borra nada, porque las banderas solo agregan.
  6. **Línea de instrucciones:** «¿Agrego una línea en el archivo de instrucciones de cada agente que apunte a la skill?». Por defecto sí. Muestra los archivos que tocaría.
  7. Ejecución, con un spinner por agente y una barra de progreso en hielo mientras baja el modelo.
  8. **Sueño nocturno:** se instala sin preguntar, como hoy en `setup claude-code`, y queda una línea que lo dice.
  9. Cierre.
- **Cuando algo falla a mitad,** el paso queda con ✖ en rojoApagado y una línea con el motivo, y los demás pasos siguen. Por ejemplo, si un agente falla, los otros se instalan igual. El código de salida es distinto de cero si algo falló.
- **Al volver a correrla,** los agentes ya conectados aparecen marcados y el modelo instalado aparece elegido. Desmarcar un agente lo desconecta (MCP, skill y línea), después de confirmar.

### Modo no interactivo

Sin terminal, o con `--yes`, no se pregunta nada. Se usan las banderas y, donde falten, estos valores:

| Bandera | Valor por defecto |
|---|---|
| `--agents claude-code,opencode,codex,gemini,antigravity` o `--agents all` | todos los encontrados |
| `--model granite\|potion\|none` | granite si la plataforma lo permite; si no, potion |
| `--instructions` / `--no-instructions` | sí |
| `--replace` | no |

Cada paso imprime una línea simple, sin animación. Con la línea de `curl`, las banderas van después de `sh -s --`.

**[Decisión mía]** En modo no interactivo, las banderas solo **agregan**: `--agents codex` conecta Codex y no desconecta a nadie, y `--no-instructions` no quita una línea que ya estaba. Quitar algo se hace desde la TUI o con `clio-memory uninstall --agents <lista>`. Así, ningún script borra configuración sin que nadie lo vea.

### Los agentes

**Principio:** si el agente tiene su propio comando para registrar un MCP, se usa ese comando, igual que hoy con `claude mcp add`. Si no lo tiene, se edita su archivo de configuración tocando solo la entrada `clio-memory` y conservando el resto. Siempre se escribe primero un archivo temporal y luego se renombra.

En todos los casos, el MCP se registra con la **ruta absoluta** del binario y el argumento `mcp`.

**Cómo se detecta cada agente:** por su programa en el `PATH` o por su carpeta de configuración.

| Agente | Se detecta por | MCP (alcance de usuario) | Skill | Línea de instrucciones |
|---|---|---|---|---|
| Claude Code | `claude` | `claude mcp add --scope user clio-memory -- <bin> mcp` (como hoy) | `~/.claude/skills/clio-memory/SKILL.md` | `~/.claude/CLAUDE.md` |
| OpenCode | `opencode` o `~/.config/opencode/` | `~/.config/opencode/opencode.json` → `mcp.clio-memory = {type: "local", command: [<bin>, "mcp"], enabled: true}` | `~/.config/opencode/skills/clio-memory/SKILL.md` (ver duplicados) | `~/.config/opencode/clio-memory.md`, enlazado en `instructions` de `opencode.json` (ver nota) |
| Codex | `codex` o `~/.codex/` | `codex mcp add clio-memory -- <bin> mcp` (escribe `~/.codex/config.toml`; respeta `CODEX_HOME`) | `~/.agents/skills/clio-memory/SKILL.md` | `~/.codex/AGENTS.md` |
| Gemini CLI | `gemini` | `gemini mcp add -s user clio-memory <bin> mcp` (escribe `~/.gemini/settings.json`) | `~/.gemini/skills/clio-memory/SKILL.md` | `~/.gemini/GEMINI.md` |
| Antigravity | `agy`, `antigravity` o `~/.gemini/config/` | `~/.gemini/config/mcp_config.json` → `mcpServers.clio-memory = {command: <bin>, args: ["mcp"]}` | `~/.gemini/config/skills/clio-memory/SKILL.md` | `~/.gemini/GEMINI.md` (la comparte con Gemini CLI: se escribe una vez) |

Las rutas salen de la documentación oficial, revisada el 2026-11-05:

- opencode.ai/docs (mcp-servers, skills, rules, config);
- learn.chatgpt.com/docs (Codex: mcp, build-skills, agents-md);
- geminicli.com/docs (mcp-server, skills, gemini-md);
- antigravity.google/docs (mcp, skills, rules, getting-started).

El constructor debe **verificarlas en la máquina** con la versión instalada de cada agente antes de fijarlas en el código. En la máquina de Iván hay OpenCode 2.0.26, Codex 0.159.2, Gemini CLI 0.67.0 y antigravity-ide 2.5.5; `agy` no está instalado. Lo que la documentación no aclara queda en «Lo que queda al constructor».

**Duplicados de la skill:** OpenCode lee `~/.claude/skills/`, `~/.config/opencode/skills/` y `~/.agents/skills/`. Gemini CLI lee `~/.gemini/skills/` y `~/.agents/skills/`. Codex lee `~/.agents/skills/`. Regla: **cada agente elegido debe ver la skill `clio-memory`, y en lo posible una sola vez**. El constructor escoge la ubicación mínima que lo cumpla para la combinación elegida y la prueba con cada combinación. Por ejemplo, si OpenCode y Claude Code están elegidos, basta con la de `~/.claude/skills/`. Si Gemini CLI y Codex están ambos, la de `~/.agents/skills/` sirve a los dos.

Hay un caso sin salida: Claude Code solo lee `~/.claude/skills/` y Codex solo lee `~/.agents/skills/`, pero OpenCode lee las dos. Si se eligen los tres, OpenCode ve dos copias. **[Decisión mía]** Se acepta: las dos copias son el mismo archivo, con el mismo nombre y contenido. El constructor comprueba cómo lo maneja OpenCode. Si uno pisa al otro, no hay daño. Si OpenCode da error, se avisa en `doctor` y el constructor vuelve con el hallazgo. No se toca la configuración de OpenCode para apagar sus lecturas de `~/.claude`.

**Instrucciones en OpenCode — [decisión mía]:** OpenCode solo lee `~/.claude/CLAUDE.md` si no existe `~/.config/opencode/AGENTS.md`, así que crear ese archivo dejaría de cargar todo lo demás que Iván tenga en `CLAUDE.md`. Por eso, en OpenCode la línea no va en ningún `AGENTS.md`. Va en un archivo propio, `~/.config/opencode/clio-memory.md`, y su ruta se agrega al arreglo `instructions` de `opencode.json`, que está documentado y se suma al resto. Así no depende de lo que se haga con Claude Code y no tapa nada. Al desconectar OpenCode se quitan las dos cosas.

**El texto de la skill:** hoy `skill/SKILL.md` habla de Claude Code («takes precedence over Claude Code's file-based memory»). Debe tener una versión que sirva para todos los agentes, sin nombrar uno en particular, o variantes por agente generadas desde una sola fuente. Lo decide el constructor. Las reglas de seguridad son las mismas para todos.

**La firma:** cada cliente MCP escribe como `mcp:<su nombre>`, con el `clientInfo.name` que mande al iniciar. Para cada agente, el constructor anota el nombre que manda de verdad. Si alguno no manda nombre, el comportamiento actual ante un nombre vacío debe seguir siendo seguro y quedar probado.

### Actualizar y desinstalar

- **Correr la línea otra vez** reemplaza el binario si la versión es distinta y abre la TUI con lo actual ya marcado. En modo no interactivo, sin banderas, conserva lo que ya estaba.
- **Rutas de MCP:** cada entrada apunta a la ruta absoluta del binario, que no cambia al actualizar porque se reemplaza en el mismo lugar. Si la entrada apunta a otra ruta, por ejemplo una instalación vieja de `cargo install` en `~/.cargo/bin`, la TUI lo dice y propone apuntarla al binario nuevo.
- **`clio-memory uninstall`** quita, de cada agente: la entrada MCP `clio-memory`, la skill (solo si es la nuestra, comparando su contenido con las que hemos publicado) y la línea de instrucciones. También quita el timer del sueño. No toca el modelo ni `memory.db` salvo con `--models` y `--data`; la segunda pregunta antes, y en modo no interactivo exige `--yes`. El binario se borra a sí mismo al final, si está en la carpeta donde lo dejó el instalador.

### `doctor`

`clio-memory doctor` agrega una línea por agente: conectado o no, si la ruta del MCP apunta a este binario, y si la skill está y es la nuestra.

### Documentación

Todo queda documentado en el mismo trabajo:

- **`docs/releases.md` (nuevo):**
  - cómo se publica una versión: subir la versión en `Cargo.toml`, etiquetar y empujar;
  - qué hace el workflow, paso a paso;
  - qué hacer si falla a mitad;
  - el modelo de confianza y su límite;
  - la configuración del repositorio de la que depende (releases inmutables, y que Actions pueda escribir en releases) y el comando exacto para revisarla o reactivarla;
  - cómo verificar a mano un binario (`sha256sum`, `gh attestation verify`).
- **`docs/instalador.md` (nuevo):**
  - qué hace `install.sh` y `clio-memory setup` con cada agente: los archivos exactos que toca y los comandos que corre;
  - las banderas del modo no interactivo;
  - qué hace y qué no hace `uninstall`;
  - las decisiones de este diseño que afectan a quien instala (duplicados de la skill, el PATH, el modelo).
- El README cambia «Instalar y usar con Claude Code» por «Instalar», con la línea de `curl`, las banderas del modo no interactivo, la tabla de agentes y `uninstall`. `cargo install` queda como alternativa para quien compile.
- `docs/motor.md` se toca solo si algo de lo que describe cambia.

## Restricciones

- **El flujo de trabajo de Iván (m2):**
  - rama y PR para cada cambio;
  - pruebas primero, y cada prueba nueva verificada por mutación;
  - verificar con `--all-features`, como el CI;
  - `fmt`, `clippy -D warnings` y `gitleaks` limpios;
  - fusiones, push a `main` y etiquetas solo cuando Iván lo pida.
- **CI:** el workflow de release fija sus acciones por SHA, como `ci.yml`, y tiene `permissions` mínimos por trabajo. `install.sh` pasa `shellcheck` en `ci.yml` y en el release.
- **Pruebas sin red y sin tocar el `$HOME` real:**
  - para cada agente, con un `HOME` temporal y programas falsos (`claude`, `codex`, `gemini`), como ya se hace con `CLIO_MEMORY_CLAUDE`;
  - para `install.sh`, con un servidor local o `file://` y binarios y sumas de prueba: plataforma no soportada, suma incorrecta (no instala nada), actualización, y ejecución sin `/dev/tty`;
  - la TUI separa la lógica (qué se elige y qué se hace) del dibujo, para que la lógica se pruebe sin terminal.
- **Los `.md` vivos se mantienen al día en el mismo PR (m3).** No se tocan registros históricos.
- `unsafe_code = "forbid"` y `unwrap_used = "deny"` siguen valiendo.

## Lo que queda al constructor

- Cómo compilar contra glibc 2.28: zigbuild, un contenedor u otra vía.
- Si los binarios van sueltos o en `.tar.gz`.
- La biblioteca de la TUI, si `cliclack` no alcanza para la paleta exacta.
- Dónde poner la skill para cada combinación de agentes, cumpliendo «una sola skill por agente».
- Si la skill es una sola o tiene variantes por agente.
- Los detalles que la documentación no aclara y que se resuelven probando:
  - si `codex mcp add` escribe siempre en el alcance de usuario;
  - si OpenCode 2 sigue leyendo `opencode.json` igual que en la documentación v1;
  - si OpenCode acepta `opencode.jsonc` global; si existe, se edita ese en vez de crear otro, conservando los comentarios o negándose con un mensaje claro;
  - el `clientInfo.name` de cada agente.
- Cómo se muestra el paso de un agente que no se encontró pero que la persona pidió con `--agents`. Debe fallar ese paso, no todo el setup.
- El orden de los PR. Una partición razonable: (1) workflow de release, `install.sh` y `docs/releases.md`; (2) agentes nuevos, `uninstall` y `docs/instalador.md`; (3) TUI. También puede ir todo en uno si queda revisable.
- Cómo correr en el CI la instalación de prueba de los binarios de macOS x86_64 y Linux aarch64: runners nativos si los hay, emulación, o solo `--version` donde no se pueda más.
