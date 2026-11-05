# Qué toca el instalador en cada agente

clio-memory se conecta a cinco agentes: Claude Code, OpenCode, Codex, Gemini CLI y Antigravity (el IDE y su CLI `agy`). Esta página dice, agente por agente, qué archivos toca, qué comandos corre y qué deja como estaba. Todo sale del código de `src/setup/`; lo que no se pudo comprobar en una máquina real está marcado como «sin verificar».

Los comandos que lo hacen:

- `clio-memory setup` es el flujo completo: elige agentes, modelo y línea de instrucciones, y conecta, instala el modelo y el sueño. Es lo que `install.sh` lanza al terminar. Sus banderas están en [Modo sin terminal](#modo-sin-terminal).
- `clio-memory setup claude-code` conecta solo Claude Code, con el sueño nocturno.
- `clio-memory uninstall` y `clio-memory doctor` conocen a los cinco agentes.

Cómo se baja e instala el binario (`install.sh`) está en [releases.md](releases.md).

## En una tabla

| Agente | Se detecta por | Servidor MCP | Línea de instrucciones |
|---|---|---|---|
| Claude Code | `claude` en el `PATH`, o `CLIO_MEMORY_CLAUDE` tal como esté (no se comprueba que exista) | `claude mcp add --scope user clio-memory -- <bin> mcp` | `~/.claude/CLAUDE.md` |
| OpenCode | `opencode` en el `PATH`, o la carpeta `~/.config/opencode/` | se edita `~/.config/opencode/opencode.json` | `~/.config/opencode/clio-memory.md`, enlazado en `instructions` de `opencode.json` |
| Codex | `codex` en el `PATH` | `codex mcp add clio-memory -- <bin> mcp` | `~/.codex/AGENTS.md` (o `$CODEX_HOME/AGENTS.md`) |
| Gemini CLI | `gemini` en el `PATH` | `gemini mcp add -s user clio-memory <bin> mcp` | `~/.gemini/GEMINI.md` |
| Antigravity | `agy` o `antigravity` en el `PATH`, o la carpeta `~/.gemini/config/` | se edita `~/.gemini/config/mcp_config.json` | `~/.gemini/GEMINI.md` (la misma de Gemini CLI) |

`<bin>` es la ruta absoluta de este binario, y el servidor es siempre `<bin> mcp`. La skill va donde dice [la sección de la skill](#dónde-va-la-skill). Detectar un agente nunca lo ejecuta: solo mira el `PATH` y esas carpetas. Si el programa existe pero falla, ese paso queda en ✖ con el motivo y los demás agentes siguen.

## Lo que no toca

- Otros servidores MCP, otras skills y otras líneas de tus archivos de instrucciones.
- Un servidor o una skill llamados `clio-memory` que **no** son los nuestros: antes de cambiarlos pregunta («Codex ya tiene un servidor MCP llamado clio-memory. ¿Reemplazarlo?»), y con un no (o sin terminal para contestar) no escribe nada para ese agente. `--replace` los reemplaza sin preguntar.
- Lo que no se pueda leer: si el estado de un agente no se puede comprobar (su programa falla), su skill se deja como está.
- La memoria (`memory.db`) y los modelos, salvo que se lo pidas a `uninstall`.

Qué es «nuestro»:

- **Servidor MCP.** Es nuestro el que corre un binario llamado `clio-memory` con el único argumento `mcp`. Si ese binario es este, está al día. Si es otro (por ejemplo `~/.cargo/bin/clio-memory`, de una instalación vieja), se re-apunta a este sin preguntar. Si está desactivado (`"enabled": false`, o `"disabled": true`), se vuelve a escribir activado. Cualquier otra cosa llamada `clio-memory` es ajena.
- **Skill.** Es nuestra la que coincide, por sha256, con la de esta versión o con una publicada antes (v0.2.0, v0.4.0, v0.5.0): se actualiza o se mueve. Cualquier otra, incluida una que hayas editado, es ajena.
- **Claude Code.** Solo cuenta el alcance de usuario (`--scope user`). Una entrada `clio-memory` en el alcance local o de proyecto se lee como «no hay».

Correrlo otra vez conserva lo elegido. Sin preguntas (con `--yes` o sin terminal) y sin banderas, se quedan los agentes que ya estaban conectados y siguen instalados (uno que no lo estaba se conecta con `--agents`, y `setup` lo dice; uno que desinstalaste se salta aunque quede su configuración), el modelo que hay (si no había ninguno, no se baja nada) y las líneas de instrucciones como estaban. Cada paso dice «ya está» y no reescribe lo que ya está bien.

## Por agente

### Claude Code

- `claude mcp get clio-memory` para ver qué hay. Si hay algo nuestro con otra ruta, o algo ajeno que aceptaste reemplazar, corre `claude mcp remove --scope user clio-memory` y luego `claude mcp add --scope user clio-memory -- <bin> mcp`.
- La línea va al final de `~/.claude/CLAUDE.md`.
- Antes de nada, `setup claude-code` corre `claude --version`: sin `claude` no escribe nada, ni siquiera la skill. Con `CLIO_MEMORY_CLAUDE=/ruta/a/claude` usa ese programa en vez del del `PATH`.
- `setup claude-code` pregunta si añade la línea, salvo que ya esté o que le des `--claude-md` (sí) o `--no-claude-md` (no).
- Sin verificar contra el programa real: el formato de `claude mcp get` se leyó del binario de Claude Code 2.1.311 (2026-11-05), y no se registró nada en el Claude Code de la máquina.

### OpenCode

OpenCode no tiene un comando para registrar un MCP, así que se edita su configuración.

- Archivo: `~/.config/opencode/opencode.json`; si solo existe `opencode.jsonc`, ese.
- Entrada: `mcp.clio-memory = {"type": "local", "command": ["<bin>", "mcp"], "enabled": true}`. El comando es un arreglo, así que una ruta con espacios no se parte.
- La línea **no** va en un `AGENTS.md`. Según su documentación, OpenCode solo lee `~/.claude/CLAUDE.md` si no existe `~/.config/opencode/AGENTS.md`, y crearlo taparía lo que ya tengas. Por eso la línea va sola en `~/.config/opencode/clio-memory.md`, y la ruta de ese archivo se agrega al arreglo `instructions` de `opencode.json` (se suma a lo que ya hubiera; no hay una segunda clave `instructions`). Al desconectar, salen las dos cosas.
- Sin verificar: si OpenCode 2 sigue leyendo `opencode.json` y `instructions` como su documentación.

### Codex

- `codex mcp get clio-memory --json` para ver qué hay, y `codex mcp add clio-memory -- <bin> mcp` para registrarlo (`codex mcp remove clio-memory` para quitarlo). Es Codex quien escribe su `config.toml`; el instalador no lo toca. Se le pasa `CODEX_HOME` tal como esté en tu entorno.
- La línea va en `AGENTS.md` dentro de `CODEX_HOME` (o `~/.codex`). Si existe un `AGENTS.override.md` no vacío, va en ese, porque Codex lee el override en vez de `AGENTS.md`. Al quitarla, sale de los dos archivos.
- Verificado el 2026-11-05 con codex-cli 0.159.2: `codex mcp add` registra en el alcance global (de usuario), y `get --json`, `remove` y el error de un servidor que no existe se comportan como el código espera.

### Gemini CLI

- El estado se lee de `~/.gemini/settings.json` (`mcpServers.clio-memory`). Para registrarlo corre `gemini mcp add -s user clio-memory <bin> mcp`, y para quitarlo `gemini mcp remove -s user clio-memory`.
- La línea va en `~/.gemini/GEMINI.md`.
- Sin verificar: no se corrió `gemini mcp add` contra el programa real. Los comandos siguen su documentación.

### Antigravity

- Archivo: `~/.gemini/config/mcp_config.json`. Entrada: `mcpServers.clio-memory = {"command": "<bin>", "args": ["mcp"]}`. Un archivo vacío (0 bytes, como el que hay en la máquina donde se probó) cuenta como `{}`.
- La línea va en `~/.gemini/GEMINI.md`, el mismo archivo de Gemini CLI. Se escribe una vez, y al desconectar uno de los dos la línea se queda mientras el otro siga conectado.
- Sin verificar: la ruta de `mcp_config.json` y la de la skill salen de la documentación; `agy` no está instalado en la máquina donde se probó.

## Archivos de otros programas

Los JSON que se editan (`opencode.json`, `mcp_config.json`) se tratan con cuidado:

- Se lee el archivo entero, se cambia solo la entrada `clio-memory` y se escribe de vuelta en un archivo temporal de la misma carpeta que luego se renombra, así que nunca queda a medias.
- Las demás claves y servidores quedan iguales y en el mismo orden. Lo que sí cambia es el formato: el archivo se reescribe con sangría de 2 espacios y un salto de línea al final.
- Conserva los permisos del archivo, y si es un enlace simbólico (un repositorio de dotfiles) se edita donde vive. Uno nuevo queda en modo 0600.
- Si no cambia nada, no se escribe.
- Un archivo con comentarios o con comas de más no es JSON: se rechaza antes de escribir nada y te dice que lo edites a mano. Un archivo cuya raíz, o cuya tabla `mcp`/`mcpServers`, no es un objeto, también se rechaza («no se tocó»).
- Al desconectar se quita solo nuestra clave, y la tabla (`mcp`, `mcpServers`) si queda vacía.

Los archivos de instrucciones (`CLAUDE.md`, `AGENTS.md`, `GEMINI.md`) solo reciben una línea al final, y al quitarla solo salen las líneas iguales a la nuestra, sin contar los espacios de los bordes. Cada otro byte queda como estaba, y el archivo conserva sus permisos. Si al quitarla el archivo queda vacío (o solo con líneas en blanco), se borra: lo más probable es que lo haya creado `setup`. Si es un enlace simbólico, el enlace se queda y lo que apunta queda vacío.

La línea, igual para todos los agentes:

```
- Mi memoria principal es clio-memory (MCP): úsala según la skill clio-memory.
```

## Dónde va la skill

Los agentes comparten carpetas de skills: OpenCode lee las de Claude Code y `~/.agents/skills`, y Codex y Gemini CLI comparten esta última. Una copia por agente le mostraría la skill dos veces a alguno, así que el instalador pone la menor cantidad de copias que cubra a todos los agentes conectados, y las mueve cuando cambian.

La skill es una sola, igual para todos los agentes (`skill/SKILL.md`). Las carpetas, y quién lee cada una:

| Carpeta | La leen |
|---|---|
| `~/.claude/skills/clio-memory/` | Claude Code, OpenCode |
| `~/.config/opencode/skills/clio-memory/` | OpenCode |
| `~/.agents/skills/clio-memory/` | OpenCode, Codex, Gemini CLI |
| `~/.gemini/config/skills/clio-memory/` | Antigravity |

De los conjuntos de carpetas que cubren a todos, se elige el que deja a menos agentes con dos copias, luego el de menos carpetas, luego el que leen menos agentes en total, y al final el primero en el orden de la tabla. Con eso:

| Agentes conectados | Carpetas con la skill |
|---|---|
| Claude Code | `~/.claude/skills` |
| OpenCode | `~/.config/opencode/skills` |
| Claude Code y OpenCode | `~/.claude/skills` |
| Codex y Gemini CLI | `~/.agents/skills` |
| Claude Code, OpenCode y Codex | `~/.claude/skills` y `~/.agents/skills` |
| Gemini CLI y Antigravity | `~/.agents/skills` y `~/.gemini/config/skills` |
| Los cinco | `~/.claude/skills`, `~/.agents/skills` y `~/.gemini/config/skills` |

Codex y Gemini CLI solos van a `~/.agents/skills`, y Antigravity solo a `~/.gemini/config/skills`. Gemini CLI también lee `~/.gemini/skills` según su documentación, pero el instalador nunca escribe ahí: lo cubre con `~/.agents/skills` para no repetir la skill.

**El caso sin salida.** Claude Code solo lee `~/.claude/skills` y Codex (o Gemini CLI) solo `~/.agents/skills`, pero OpenCode lee las dos. Si conectas Claude Code, OpenCode y Codex (o Gemini CLI), OpenCode ve dos copias. Son el mismo archivo, con el mismo nombre y contenido, y no se toca la configuración de OpenCode para que deje de leer alguna. Sin verificar: no se probó cómo reacciona OpenCode a las dos copias; si alguna vez da un error, quita una de las dos conexiones, o la copia que sobra.

Al quitar un agente, la skill se retira de las carpetas que ya no hacen falta, pero solo si es la nuestra. Una skill ajena en una carpeta que no se usa se deja en paz. Si esa carpeta sí hace falta, no se sobrescribe sin tu sí o `--replace`: el paso queda en ✖ diciendo en qué ruta está.

Sin verificar: que cada agente lea de verdad esas carpetas. Las rutas salen de la documentación oficial de cada uno, revisada el 2026-11-05, y no de ejecutar los agentes.

## El sueño nocturno

Al conectar al menos un agente, también se instala el timer del [sueño](../README.md#el-sueño): en Linux, `~/.config/systemd/user/clio-memory-sleep.service` y `.timer`, activados con `systemctl --user daemon-reload` y `systemctl --user enable --now clio-memory-sleep.timer`; en macOS, `~/Library/LaunchAgents/com.clio-memory.sleep.plist`, cargado con `launchctl`. Si el sueño ya es de un `clio-memory` anterior, se re-apunta a este binario. Si es de otro programa (el de Clio, por ejemplo), se deja como está y el paso lo dice; `clio-memory setup sleep` lo reemplaza. Si el sistema no tiene un administrador de servicios de usuario, ese paso queda en ✖ y lo demás se instala igual.

## `clio-memory doctor`

Después de sus comprobaciones de siempre, `doctor` agrega una línea por cada agente que encuentra en la máquina, con las mismas reglas de detección de la tabla:

```
ok   agente claude-code: conectado
--   agente codex: no conectado
FAIL agente opencode: el MCP apunta a /home/tú/.cargo/bin/clio-memory
```

- `ok`: el servidor MCP es de este binario y la skill de esta versión está en una carpeta que ese agente lee.
- `--`: el agente está, pero la memoria no está conectada. No cuenta como fallo. También sale `--`, con el motivo (`no se pudo leer su estado: …`), cuando el estado del agente no se puede leer y el agente no ve nuestra skill: lo más probable es que nunca se haya conectado.
- `FAIL`, con el motivo:
  - `el MCP apunta a <ruta>` (otra instalación);
  - `el MCP está desactivado`;
  - `otro servidor MCP se llama clio-memory: …`;
  - `skill ajena en <ruta>`, `skill desactualizada en <ruta>` o `falta la skill`;
  - el error al leer el estado, si el agente ve nuestra skill (estaba conectado y ya no se puede comprobar).

Antes de las líneas de los agentes, si otro `clio-memory` va antes que este en el `PATH` (por ejemplo, uno viejo de `cargo install` en `~/.cargo/bin`), `doctor` lo dice en una línea `--`: `al escribir `clio-memory` corre <ruta>, no este (<ruta>): si es de `cargo install`, quítalo con `cargo uninstall clio-memory``. No cuenta como fallo. `setup` lo dice también al terminar.

Recién instalado, antes de que un agente guarde algo, `memory.db` todavía no existe. Entonces `doctor` no corre las comprobaciones del archivo: dice `--   todavía no hay memoria: se crea la primera vez que un agente la usa`, sigue con las líneas de los agentes y no lo cuenta como fallo.

Si algo está en `FAIL`, `doctor` termina con código 1. `--json` agrega `"agents"`, con `agent`, `ok` y `detail` de cada uno, y `"notes"` con las líneas `--` que no son de un agente (sin memoria todavía, otro `clio-memory` antes en el `PATH`); sin memoria, `"checks"` es `null`. No revisa la línea de instrucciones, que es opcional.

## `clio-memory uninstall`

```sh
clio-memory uninstall                      # todos los agentes, el sueño y el binario
clio-memory uninstall --agents codex,gemini   # solo esos agentes
clio-memory uninstall --models             # además, los modelos
clio-memory uninstall --data               # además, la memoria (pregunta antes)
```

Los nombres de `--agents`: `claude-code`, `opencode`, `codex`, `gemini`, `antigravity`.

**Qué quita, de cada agente:**

- el servidor MCP `clio-memory`, **solo si es el nuestro** (con otra ruta también: es una instalación vieja). Si otro servidor se llama así, no se toca y el paso lo dice;
- la línea de instrucciones. Sale aunque el servidor ya no estuviera, y con ella, en OpenCode, `clio-memory.md` y su entrada en `instructions`;
- la skill, de las carpetas donde esté, **solo si es la nuestra** (la de esta versión o una publicada). Si quitas solo algunos agentes con `--agents`, la skill se vuelve a poner donde la necesiten los que se quedan.

**Solo sin `--agents`** (todos los agentes):

- el timer del sueño, si es el de un `clio-memory` (el de Clio u otro programa se deja y se dice);
- al final, el binario, si está en la carpeta donde lo deja `install.sh`: `~/.local/bin` o `$CLIO_MEMORY_BIN_DIR`, si la definiste. Un binario en otro sitio (por ejemplo de `cargo install`) no se borra. `--models` y `--data` se hacen antes. **Si algo no se pudo quitar (también los modelos o la memoria), el binario se queda**, el paso lo dice, y puedes correr `uninstall` otra vez.

**Qué no hace:**

- No borra la memoria ni los modelos si no lo pides.
- No toca otros servidores MCP, otras skills ni otras líneas de tus archivos.
- No quita nada ajeno que se llame `clio-memory`.

**`--models`** borra la carpeta `~/.cache/clio-models` entera: Granite, potion y ONNX Runtime con sus bibliotecas. No pregunta. Esos modelos también los usa Clio: si los necesita después, se vuelven a bajar (`clio-memory setup model`, o el instalador de Clio).

**`--data`** borra para siempre estos archivos y nada más:

- `memory.db`, `memory.db-wal`, `memory.db-shm`, `memory.db.lock` y `memory.db.sleep.lock`, en la carpeta de la memoria (`~/.local/share/clio`, o `$CLIO_MEMORY_DATA`, o la de `--db`);
- la carpeta `sleep/` que está a su lado (el estado del sueño, sus informes y sus copias).

El resto de esa carpeta se queda. Importa porque es la carpeta de Clio, que guarda ahí sus propios archivos. Si `--db` no se llama `memory.db`, se niega. En una terminal pregunta antes de hacer nada (`¿Borro la memoria: … y las copias del sueño en …? No se puede deshacer.`); un no no cambia nada. Sin terminal exige `--yes`, y sin él sale con código 2 sin hacer nada.

Cada paso imprime una línea, `✔` o `✖`. Si alguno falló, el código de salida es 1. Si no había nada que quitar, dice `nada que quitar`.

## El `PATH`

`install.sh` deja el binario en `~/.local/bin` (o en `$CLIO_MEMORY_BIN_DIR`) y **no edita ningún archivo de tu shell**. Si esa carpeta no está en tu `PATH`, `clio-memory` no se encuentra con ese nombre hasta que la agregues:

```sh
export PATH="$HOME/.local/bin:$PATH"     # bash y zsh: en ~/.bashrc o ~/.zshrc
fish_add_path ~/.local/bin               # fish
```

Mientras tanto, corre `~/.local/bin/clio-memory doctor` con la ruta completa. Que falte en el `PATH` no rompe nada de lo conectado: los servidores MCP y el timer del sueño usan la ruta absoluta del binario. Lo que sí necesita el `PATH` son los programas de los agentes (`claude`, `codex`, `gemini`, `opencode`, `agy`), porque así se detectan y así se ejecutan.

## El modelo

El instalador no guarda ninguna preferencia de modelo. El binario usa el mejor que haya en disco: Granite, si no potion, si no búsqueda por texto. `clio-memory setup` te deja elegir (en una terminal) o usa `--model`; `clio-memory setup model` instala Granite y `setup model potion` el estático. Los detalles están en el [README](../README.md#el-modelo-de-vectores).

- Tamaños: Granite 1,28 GB es el modelo solo (con su tokenizador); encima se baja ONNX Runtime y, con una tarjeta NVIDIA, las bibliotecas de CUDA que el sistema no tenga. potion, 531 MB, no necesita nada más.
- Granite necesita ONNX Runtime, que `setup model` baja solo. Hay runtime publicado para Linux x86_64, Linux aarch64 y macOS arm64. En **macOS x86_64** no lo hay, así que ahí sirve potion, o texto.
- No hay forma de quitar un solo modelo: `uninstall --models` borra todos (ver arriba).
- En una terminal, elegir un modelo peor que el instalado ofrece borrar el mejor (y ONNX Runtime) y pregunta antes, con su tamaño y avisando que los modelos de `~/.cache/clio-models` también los usa Clio (si los necesita, se vuelven a bajar); un no lo deja. Sin preguntas (con `--yes` o sin terminal) nunca se borra un modelo, tampoco con `--model`.

## La firma de cada agente

Un cliente MCP escribe como `mcp:<su nombre>`, con el `clientInfo.name` que manda al iniciar. El nombre se pasa a minúsculas y conserva solo `a-z`, `0-9` y `-` (lo demás pasa a `-`), sin guiones repetidos ni en los bordes; se corta en 40 caracteres, y si no queda nada, firma `mcp:cliente`. Cada firma tiene su cuota de 500 escrituras por hora.

Qué manda de verdad cada agente se ve en la firma de un recuerdo guardado desde él (`clio-memory history <id>`). Con lo que hay hoy en la memoria compartida de esta máquina (2026-11-05):

| Agente | Firma | Estado |
|---|---|---|
| Claude Code | `mcp:claude-code` | verificado: 26 operaciones firmadas así |
| OpenCode | — | sin verificar |
| Codex | — | sin verificar |
| Gemini CLI | — | sin verificar |
| Antigravity | — | sin verificar |

Los otros cuatro quedan «sin verificar»: no hay ningún recuerdo guardado desde ellos en esa memoria, y no se corrieron los agentes para generarlo. Para comprobarlo, guarda un recuerdo desde el agente y mira `clio-memory history <id>`.

## Modo sin terminal

`clio-memory setup` pregunta solo en una terminal (stderr es una terminal) y sin `--yes`. Sin terminal, o con `--yes`, no pregunta nada: cada pregunta se contesta con la bandera que la responde y, si falta, con el valor por defecto. En una terminal, una bandera también contesta su pregunta. Sin terminal, o con `--yes`, cada paso sale como una línea (`✓` o `✖`) y no se dibuja nada.

| Bandera | Qué hace | Si falta (sin preguntas) |
|---|---|---|
| `--yes` | No pregunta nada, ni siquiera en una terminal | |
| `--agents LISTA` | Conecta solo esos agentes. `LISTA` son claves separadas por comas (`claude-code`, `opencode`, `codex`, `gemini`, `antigravity`), o `all` | todos los agentes que encuentra; si ya hay alguno conectado (otra corrida), los conectados |
| `--model granite\|potion\|none` | El modelo de vectores a instalar; `none` es la búsqueda por texto | el que ya está instalado; si no hay ninguno, Granite donde hay ONNX Runtime publicado (Linux x86_64, Linux aarch64, macOS arm64), si no potion; en otra corrida sin modelo, ninguno (no se baja nada) |
| `--instructions` / `--no-instructions` | Agrega (o no) la línea de instrucciones a cada agente conectado | sí en la primera instalación; en otra corrida, como estaba (no se agrega a quien no la tiene) |
| `--replace` | Reemplaza sin preguntar un servidor o una skill ajenos llamados `clio-memory` | no reemplaza: ese agente queda en ✖ |

`--from-installer` es interna: la pasa `install.sh`. Hace que `setup` diga dónde quedó el binario y, si su carpeta no está en el `PATH`, la línea que la agrega ([El `PATH`](#el-path)); no cambia qué se instala.

Qué quieren decir:

- **Sin preguntas, las banderas solo agregan.** Con `--yes` o sin terminal, nada se desconecta, ni se quita una línea de instrucciones, ni se borra un modelo. Para eso está `uninstall` (o, en una terminal, la TUI, que pregunta antes de desconectar o borrar).
- **`--agents` no desconecta a los demás.** Un agente de la lista que no está en la máquina queda en ✖ (`no se encontró …`) y la salida es distinta de cero; los demás se conectan igual. Un nombre que no es una clave es un error de uso (código 2) y no se hace nada. Sin ningún agente en la máquina y sin `--agents`, `setup` lo dice e instala igual el modelo.
- **`--model` nunca borra.** Con `--model potion` y Granite ya instalado, no hace nada y lo dice (Granite es el que se usa); con `--model none` tampoco borra lo instalado, y dice cuál sigue en uso. Con `--model granite` en macOS Intel, donde no hay ONNX Runtime publicado, el modelo se baja y el paso del runtime queda en ✖.
- **`--yes` junto a `--yes`** (la de `install.sh` más la tuya) vale una vez.
- Estas banderas son de `setup` solo: con `setup claude-code`, `setup model` o `setup sleep` son un error de uso. Lo que sí tienen esos subcomandos: `setup claude-code --replace`, `--claude-md` y `--no-claude-md`, que contestan por la línea de instrucciones.
- `uninstall --data` exige `--yes` sin terminal.

Por el `curl`, las banderas van después de `sh -s --`:

```sh
curl -fsSL https://raw.githubusercontent.com/i-montes/clio-memory/main/install.sh | sh -s -- --yes --agents codex,gemini --model potion
```

`install.sh` pasa sus argumentos a `setup` como están. Si hay una terminal (`/dev/tty`), `setup` la lee aunque el script venga por una tubería; si no la hay, agrega `--yes` por su cuenta. Un código de salida distinto de cero quiere decir que algún paso quedó en ✖ (las líneas lo dicen), y los demás pasos se hicieron.

## Variables de entorno que se leen

| Variable | Para qué |
|---|---|
| `HOME` | dónde están las configuraciones de los agentes (debe ser una ruta absoluta) |
| `CODEX_HOME` | la carpeta de Codex en vez de `~/.codex`; se le pasa a `codex` tal cual |
| `CLIO_MEMORY_CLAUDE` | el programa `claude` que se usa en vez del del `PATH` |
| `CLIO_MEMORY_BIN_DIR` | `install.sh`: dónde deja el binario (por defecto `~/.local/bin`); `setup` la lee para decir si está en el `PATH`, y `uninstall` para borrarlo de ahí |
| `CLIO_MEMORY_DATA` | la carpeta de `memory.db` |
| `CLIO_MEMORY_VERSION` | `install.sh`: la versión que se baja (`vX.Y.Z`); por defecto, el último release |
| `CLIO_MEMORY_DOWNLOAD_URL` | `install.sh`: la base de los releases, por defecto `https://github.com/i-montes/clio-memory/releases` (la prueba de humo del release usa `file://`) |
| `CLIO_MEMORY_SKIP_ATTESTATION` | `install.sh`: con cualquier valor no vacío, no intenta `gh attestation verify` |
| `CLIO_MEMORY_HAND_OVER` | `install.sh`, gancho de prueba: con `0` instala el binario sin correr `setup` (la prueba de humo del release) |
| `CLIO_MEMORY_TTY` | `install.sh`, gancho de prueba: el archivo que se lee como terminal, por defecto `/dev/tty` |
| `NO_COLOR` | sin colores, en `install.sh` y en la TUI |
| `SHELL` | `setup --from-installer`: para qué shell es la línea que agrega la carpeta del binario al `PATH` |
