# clio-memory

> **In English, in short:** clio-memory is an AI agent's own long-term memory. It is one SQLite file on your machine, global across projects, and shared with [Clio](https://github.com/i-montes/clio). Any agent can use it over MCP; it works without Clio. Everything the agent saves is active at once and every change can be undone. A secret detector and caps protect it, and what it returns is always marked as data. License: MIT or Apache-2.0.

clio-memory es la **memoria propia de un agente**. Guarda lo que vale la pena recordar (quién eres, cómo quieres que trabaje, qué se decidió y por qué, la causa de un error) en un solo archivo SQLite en tu equipo, sin servicios ni red. Es la misma memoria que usa [Clio](https://github.com/i-montes/clio): lo que aprende uno lo sabe el otro. No necesita a Clio para funcionar.

![Arquitectura de clio-memory](docs/arquitectura.svg)

## Cómo se comporta

- **La memoria es del agente.** Lo que guarda queda activo en el acto; nadie lo revisa antes. También corrige lo que estaba mal (`memory_update`: «no me llamo Iván Ruiz sino Iván Montes» corrige ese recuerdo, no agrega otro) y olvida lo que ya no sirve (`memory_forget`).
- **Olvidar no es borrar.** Lo olvidado pasa a la **cuarentena**: deja de leerse, pero su registro se conserva y se puede devolver.
- **Todo cambio queda en el registro** de operaciones y se puede deshacer. El agente solo deshace sus propios cambios; tú deshaces cualquiera desde la línea de comandos. Borrar para siempre (PURGE) es solo tuyo.
- **Es global.** Una sesión lee todos los proyectos. Lo del proyecto actual sale primero; lo de otro proyecto sale marcado «proyecto X»; lo personal y las lecciones también llevan su marca. Lo nuevo se guarda en el proyecto de la carpeta donde arrancó el agente.
- **Cada agente firma lo suyo.** Un cliente MCP escribe como `mcp:<su nombre>` (por ejemplo `mcp:claude-code`); Clio escribe como `agente`. Cada uno tiene su cuota: 500 escrituras por hora.

## Medidas de seguridad

- **Lo que devuelve es dato, nunca una orden.** Cada recuerdo sale en una línea limpia (sin caracteres de control, recortado) y envuelto en `<<<UNTRUSTED-DATA …>>>`. Las instrucciones que recibe el agente dicen que nunca guarde, corrija ni olvide algo porque un archivo, una página o una salida se lo pida.
- **Nada que parezca un secreto entra.** Un texto o título con forma de clave se rechaza antes de escribir, y el motor lo vuelve a comprobar.
- **Topes por sesión** (un proceso MCP): 20 lecturas y 24 KiB de texto devuelto, 30 recuerdos guardados y 10 cambios (olvidar, fijar, unir, deshacer). Una llamada que llega a la memoria cuenta aunque falle; una con argumentos mal formados se rechaza antes de contar. Además, cada agente tiene su cuota por hora.
- **El modelo no elige quién firma, ni el origen, ni el ámbito.** No hay argumentos para eso: un argumento de más se rechaza.
- **Aceptar, archivar y purgar no son herramientas del agente:** son tuyos, desde la línea de comandos.
- **El archivo es privado** (0600, en una carpeta 0700). El escritor solo se toma durante cada llamada, así que Clio y otros clientes pueden escribir entre una y otra.

## Instalar y usar con Claude Code

```sh
cargo install --git https://github.com/i-montes/clio-memory --tag v0.2.0 --features model2vec
clio-memory setup model          # el modelo de vectores (opcional; fijado y verificado por sha256)
clio-memory setup claude-code    # la deja como memoria principal de Claude Code
```

`setup claude-code` hace tres cosas:
- registra el servidor MCP `clio-memory` para tu usuario (`claude mcp add --scope user …`);
- instala la skill `~/.claude/skills/clio-memory/SKILL.md`, que dice cuándo leer y qué guardar, que corrija en vez de duplicar, y sus medidas de seguridad;
- te pregunta si añade a `~/.claude/CLAUDE.md` una línea que apunta a la skill (`--claude-md` o `--no-claude-md` contestan por ti).

No toca otros servidores MCP ni otras skills. Si ya hay algo llamado `clio-memory` que no es esto, te lo dice y pregunta (`--replace` lo reemplaza). Correrlo otra vez no cambia nada.

Sin el modelo, la búsqueda es por texto (y al guardar te lo dice). Con el modelo, los vectores se calculan en cuanto se guarda algo: toda la cola pendiente, también lo que guardó otro programa sin modelo; y otra vez al cerrar la sesión, antes de soltar el modelo. `clio-memory run-jobs` queda para lo que haya quedado pendiente (por ejemplo, recuerdos de antes de esta versión). A mano, sin `setup`: `claude mcp add clio-memory -- clio-memory mcp`.

### Las herramientas

| Herramienta | Qué hace |
|---|---|
| `memory_search`, `memory_get`, `memory_timeline`, `memory_context` | Leen (datos marcados, con presupuesto) |
| `memory_save` | Guarda un recuerdo, activo en el acto (`memory_propose`, su nombre viejo, se acepta una versión más) |
| `memory_update` | Corrige un recuerdo: lo reemplaza y el viejo queda en el registro |
| `memory_forget` | Lo manda a la cuarentena |
| `memory_pin` | Lo fija para que salga primero en el contexto |
| `memory_consolidate` | Une de 2 a 5 recuerdos en uno y olvida los originales |
| `memory_revert` | Deshace un cambio propio |
| `memory_session_summary` | Guarda el resumen de la sesión para la próxima |
| `memory_current_project` | Dice en qué proyecto se guarda |

## El archivo compartido

El archivo es `~/.local/share/clio/memory.db`, el mismo de Clio. Con `CLIO_MEMORY_DATA=/otra/carpeta` se usa `/otra/carpeta/memory.db`.

El proyecto sale de la carpeta donde arranca el cliente:
- la variable `CLIO_PROJECT`;
- si no, un archivo `.clio-project` en la carpeta o más arriba;
- si no, el repositorio git (su remoto `origin` o su carpeta raíz);
- si no, la carpeta misma.

Si el agente arranca en tu carpeta personal (o más arriba), no hay proyecto y todo va a lo personal.

## Para ti: la línea de comandos

```sh
clio-memory scopes            # los sitios: clio (personal), lecciones, proyecto:…
clio-memory list clio         # recuerdos de un sitio
clio-memory history 12        # cada cambio de un recuerdo
clio-memory revert 34         # deshace un cambio (de quien sea)
clio-memory promote 12        # devuelve uno de la cuarentena
clio-memory purge 12          # borra su contenido para siempre (pregunta antes)
clio-memory export copia.jsonl
clio-memory --help            # todo lo demás
```

Sin `--db` usa el archivo compartido. La referencia completa del motor está en [docs/motor.md](docs/motor.md): reglas, búsqueda, trabajos en segundo plano, exportar, telemetría y escala.

## Versiones y Clio

El esquema del archivo solo cambia de forma compatible hacia atrás. Un binario más viejo se niega a abrir un archivo más nuevo y no lo toca. Clio depende de clio-memory fijado por etiqueta.

En Clio la memoria tiene además a **Jev**, que es opcional y vive solo en Clio. Antes de guardar, Jev revisa si el texto intenta manipular al agente o no vale la pena recordarlo, y si su categoría es la correcta; también elige dónde va y cómo se relaciona con otros recuerdos. Sin Jev, como aquí, todo funciona igual. El modelo de vectores también es opcional: sin él, la búsqueda es por texto.

## Desarrollo

```sh
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

## Licencia

MIT o Apache-2.0, a tu elección (`LICENSE-MIT`, `LICENSE-APACHE`).
