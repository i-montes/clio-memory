# Cómo se publica una versión

Publicar una versión es empujar una etiqueta. No hay PR de sumas que fusionar, ni release en borrador que publicar, ni secretos que crear o renovar: el workflow `.github/workflows/release.yml` hace el resto.

## Publicar

1. Subir `version` en `Cargo.toml` (y `Cargo.lock`) en un PR normal y fusionarlo en `main`.
2. Si `skill/SKILL.md` cambió desde la versión anterior, que en ese PR (o antes) el sha256 de la skill anterior esté en `KNOWN_SKILLS` (`src/setup/skill.rs`), con un comentario de la versión que la publicó, y que `CURRENT_SKILL_SHA` (en las pruebas de ese archivo) sea el de la nueva. La prueba `the_skill_is_the_pinned_one` falla en cuanto la skill cambia y dice los dos hashes. Sin esto, la skill que ya está instalada en las máquinas se leería como ajena: no se actualizaría y cada agente quedaría en ✖.
3. Desde `main`, ya actualizado:

   ```sh
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

La etiqueta, sin la `v`, debe ser igual a `version` en `Cargo.toml`. Si no lo es, el workflow falla antes de compilar nada.

## Notas de versiones

### v0.13.0

Los bloques fijos de la persona van primero en `memory_context`, en su propia sección, y un agente ya no puede corregir, olvidar, unir ni deshacer su camino hasta cambiar lo que un bloque muestra.

- **Primero los bloques, en su sección.** `memory_context` empieza por los bloques `identidad`, `reglas` y `preferencias` del lugar personal, en ese orden, cada uno bajo su línea `## identidad`, `## reglas` o `## preferencias`, dentro de una sección `<<<PERSON-SET memory …>>>` … `<<<END-PERSON-SET …>>>` que va antes del marco `UNTRUSTED-DATA` de lo demás. Cada recuerdo sale entero; la sección tiene su propio tope de 5000 bytes (lo que no cabe queda fuera entero, nunca cortado) y lo demás conserva el suyo de 4096. Un bloque vacío no imprime nada; sin bloques llenos no hay sección. Otro bloque (por ejemplo `vigilancia`) no lleva línea propia, y un recuerdo `externo` que la persona puso en un bloque no entra en la sección: sale con lo demás, como dato.
- **Lo que un bloque muestra es de la persona.** Un agente que corrige (`memory_update`, o un recuerdo nuevo con el mismo tema), olvida (`memory_forget`), une (`memory_consolidate`) o deshace un cambio suyo (`memory_revert`) sobre un recuerdo que cualquier bloque lista o muestra hoy (la versión vigente, si la persona lo corrigió) recibe «ese recuerdo está en un bloque fijo de la persona: solo ella lo cambia» y no cambia nada; en una unión, ninguna de las otras fuentes se olvida. Una decisión del juez tampoco lo reemplaza, y en una importación de recuerdos sueltos esa línea se rechaza y las demás siguen. La persona conserva todos sus poderes.
- **La skill, las instrucciones del servidor MCP y la descripción de `memory_context`** le dicen al agente qué es esa sección: `## identidad` y `## reglas`, quién es y cómo trabaja (las adopta); `## preferencias`, quién es la persona. La skill cambia: `clio-memory update` la pone al día.

### v0.12.1

El sueño toma la llave de Temis del baúl de Clio.

- **Primero el baúl, después el archivo.** Si las dos variables `CLIO_MEMORY_TEMIS_URL` y `CLIO_MEMORY_TEMIS_TOKEN` están puestas, mandan como antes. Si no, `clio-memory sleep` (y `doctor`, y `setup temis --check`) le piden la llave al baúl de Clio (`$XDG_RUNTIME_DIR/clio/vault-gateway.sock`): el secreto `temis`, pedido como la herramienta `jev` hacia el servidor de la URL. La URL es la de `temis.json` o, sin archivo, `https://api.imontes.dev/temis`. El baúl solo la entrega si coincide con su destino (`jev:api.imontes.dev`). Si no hay baúl, está cerrado, se niega o no responde en 2 s, se usa `temis.json` como antes. Ningún error de ese paso dice por qué, así que ninguno puede llevar la llave.
- **Para qué**: con la llave en el baúl ya no hace falta `temis.json`, y `clio doctor` deja de fallar por la regla S-18 (un valor del baúl copiado en un archivo de la carpeta de datos compartida).

Después de etiquetar: `clio-memory update`, comprobar que el sueño encuentra a Temis sin el archivo y, entonces, borrar `~/.local/share/clio/temis.json` (o `clio-memory setup temis --remove`).

### v0.12.0

El sueño decide con Temis, si lo configuraste.

- **Dos preguntas por par.** Con Temis configurado, el sueño le pregunta por los pares de recuerdos parecidos: si todo lo que dice uno está también en el otro (`se_conserva`, de 0 a 1, en los dos sentidos) y cómo se relacionan. Un recuerdo que otro conserva entero (≥ 0,80; sus cifras, palabras de código y nombres están en el que queda, y los dos dicen las mismas negaciones: «no», «nunca», «sin», «not»…) y sin contradicción (ningún `conflicts_with` con confianza ≥ 0,50) va a cuarentena con «sueño: contenido en mN»: se deshace con `revert` durante 30 días. Si Temis dice `conflicts_with` con confianza ≥ 0,50, no se dobla: el par sale en «para revisar». El sueño nunca escribe relaciones de Temis.
- **Topes**: 40 pares por sueño, 10 s por llamada y, si falla 3 veces seguidas (o no respondió ni una vez en ese sueño), los parecidos quedan para el próximo sueño con una sola alarma («Temis no respondió: los parecidos quedan para el próximo sueño»). Mientras Temis no responde, esos pares esperan al próximo sueño que corra de todos modos (al menos uno al día) en vez de despertarlo cada media hora. No se pregunta por un par de dos recuerdos de la persona o fijados, ni cuando ya no queda sitio en la cuarentena. Nunca se envía un texto que parezca un secreto, y cada texto se corta a 4000 caracteres. Un `--dry-run` no pregunta.
- **`clio-memory setup temis --url URL`**: el token entra por la entrada estándar (nunca como argumento) y no se guarda nada hasta que Temis responde con él. `--check` lo vuelve a preguntar (el nombre del modelo que Temis dice se muestra limpio y cortado a 80 caracteres), `--remove` lo quita y `--allow-http` admite `http` fuera de esta máquina. `CLIO_MEMORY_TEMIS_URL` y `CLIO_MEMORY_TEMIS_TOKEN` mandan sobre el archivo. Se guarda en `<datos>/temis.json` (0600). Se niega, con código 1, si lo corre un agente, salvo con `CLIO_MEMORY_AS_PERSON=1`.
- **`doctor`** dice `temis: configurado (URL)` o `no configurado`, sin llamar a Temis. **`uninstall --data`** borra también `temis.json`.
- **Sin Temis, nada cambia**: el sueño es el de v0.11.0.
- **Programas anfitriones (Clio)**: `clio_memory::temis::from_data(&datos)` se pasa a `sleep::run_judged`; `sleep::run` sigue igual.

Después de etiquetar: correr `clio-memory update` y luego `clio-memory setup temis --url https://api.imontes.dev/temis` con el token por la entrada estándar. La comprobación con el servicio real se hizo el 2026-11-16. `bench/temis/run.py` dio las mismas cifras que [results.md](../bench/temis/results.md), `setup temis` contra el servicio respondió `Temis listo: temis-2.4.0`, y un `sleep --now --dry-run` sobre una copia de la memoria terminó sin cambiar nada.

### v0.11.0

La memoria por skill y por línea de comandos, sin un proceso por sesión.

- **`clio-memory m <verbo>`**: las herramientas de memoria como órdenes que corre el agente (`context`, `search`, `get`, `timeline`, `save`, `update`, `consolidate`, `summary`, `forget`, `pin`, `revert`, `project`). Cada una es una llamada al demonio (o, sin demonio, una sesión dentro del proceso). Códigos de salida: 0 contestó, 1 la herramienta se negó, 2 orden mal escrita; el texto largo entra por `-` con un heredoc de comillas simples.
- **Quién llama**: `--agent`, `CLIO_MEMORY_AGENT`, el entorno (`CLAUDECODE`) o el programa de agente antecesor; firma `mcp:<agente>`, así que el historial no se parte.
- **Una skill para los dos modos**: usa las herramientas `memory_*` si existen y, si no, las órdenes. La línea de instrucciones es ahora `- Mi memoria principal es clio-memory: úsala según la skill clio-memory.`; la anterior (`… clio-memory (MCP): …`) se renueva en `setup` y `update`, sin dejar las dos, y la skill de v0.7.0 a v0.10.1 se sigue conociendo como nuestra.
- **`setup --mode skill|mcp|auto`**: cada agente se conecta por skill o por MCP y `setup` anota el modo en `<datos>/agents.json`. Por defecto, skill en Claude Code y OpenCode y MCP en Codex, Gemini CLI y Antigravity; si `clio-memory` no está en el `PATH` (o el primero no es este binario), Claude Code y OpenCode quedan por MCP y el paso dice cómo pasarlos a skill. En skill, Claude Code recibe la regla `Bash(clio-memory m:*)` en `~/.claude/settings.json`.
- **`doctor` y `uninstall`** conocen el modo skill: `conectado (skill)` o `conectado (MCP)`, y un `FAIL` si por skill falta `clio-memory` en el `PATH` o la skill.
- **Las órdenes de la persona** (`purge`, `promote`, `archive`, `revert`, `feedback`, `import`, `remember` sin `--actor`, `uninstall --data`…) se niegan, con código 1, si las corre un agente, salvo con `CLIO_MEMORY_AS_PERSON=1`. Evita el accidente; no es una barrera.

Después de etiquetar: correr `clio-memory update` y reiniciar las sesiones de agente abiertas. Claude Code y OpenCode pasan a modo skill en esa actualización (si `clio-memory` en el `PATH` es este binario; si no, se quedan por MCP): pierden su servidor MCP y ganan la regla de permiso (Claude Code) hasta que `setup --agents … --mode mcp` los devuelva a MCP.

Antes de cambiar a skill el modo por defecto de Codex, Gemini CLI o Antigravity, llenar la tabla de [La comprobación en cada agente](instalador.md#la-comprobación-en-cada-agente) en cada uno. El cambio se hace solo en `modes::default_mode` (`src/setup/modes.rs`), con su prueba.

### v0.10.1

Cierra lo que quedó pendiente de v0.10.0.

- **El agente nunca se queda esperando.** Un demonio que no contesta en 2 minutos (`CLIO_MEMORY_DAEMON_TIMEOUT`) es un error: esa llamada la atiende el proceso `mcp`, que vuelve a probar el demonio un minuto después y, si contesta, suelta su modelo.
- **Un guardado no frena a los demás agentes.** En el demonio, un guardado calcula solo su propio vector; el resto de la cola la calcula un solo hilo aparte, sin retener a nadie (los pendientes van del más nuevo al más viejo). En el demonio y en las sesiones de agente, el modelo ya no corre con el candado de escritor tomado (`run-jobs --model` y el `embed_queue` de Clio todavía sí).
- **El demonio dice lo que el agente necesita saber**: la nota del proyecto llega al registro del agente (`note`), `status` dice si guarda el modelo (`resident`) y una petición que falla por dentro se contesta con un error en vez de tumbarlo.
- **`status` no hace salir a un demonio anterior**; `daemon --status` y `doctor` dicen cuándo es de una versión anterior y, con `CLIO_MEMORY_NO_DAEMON=1`, que está `desactivado`. `setup`, `setup model` y `daemon --stop` avisan también cuando el demonio que detienen es de la versión anterior.
- **Ciclo de vida**: `--db memory.db` sin carpeta, un `exe` relativo, un demonio que no arranca (dice dónde está su `daemon.log`), el socket privado desde que se crea, un demonio tardío no recrea una carpeta borrada, una carpeta con nombre no UTF-8 y `uninstall` sin esperas de más.
- **Programas anfitriones**: `RemoteEmbedder` parte en varias peticiones un lote demasiado largo y explica mejor un rechazo del demonio.
- Los textos de los límites para el agente (`1 minute`) y los archivos temporales que deja un proceso muerto en `sleep/`.
- Después de etiquetar: correr `clio-memory update` (el `setup` final detiene el demonio de v0.10.0) y reiniciar las sesiones de agente abiertas.

### v0.10.0

- **Un solo proceso con el modelo.** `clio-memory mcp` ya no carga modelo ni base de datos: pasa cada llamada al **demonio**, un proceso por carpeta de datos que lanza el primer cliente y que se va a los 10 minutos sin uso. Con 5 sesiones, la memoria de los procesos baja de 2.480 MB a unos 560 MB (36 MB los `mcp` y unos 520 MB el demonio) y entre los agentes que atiende el demonio ya no hay guardados rechazados por el candado de escritor (la línea de comandos, Clio o el sueño todavía pueden tenerlo un momento). Ver [bench/sessions/results.md](../bench/sessions/results.md).
- `clio-memory daemon --status|--stop`, `CLIO_MEMORY_NO_DAEMON=1`, `CLIO_MEMORY_SOCK` y `CLIO_MEMORY_DAEMON_IDLE`. `setup` y `setup model` detienen el demonio en marcha; `doctor` lo menciona. Un socket es de una sola carpeta de datos.
- Después de fusionar y etiquetar: correr `clio-memory update` y **reiniciar las sesiones de agente abiertas** (las viejas siguen con su proceso de v0.9.x hasta cerrarse).

### v0.9.1

- **Sin telemetría.** ONNX Runtime nunca arranca con su telemetría encendida: el proceso se relanza con `ORT_DISABLE_TELEMETRY` puesta.
- **Ids purgados.** Un id purgado no se reutiliza nunca, y la importación acepta un id que renació después de la purga.
- **La purga borra el título y la categoría**, no solo el texto. El sueño y la importación limpian lo purgado antes, y `doctor` lo comprueba (`purged_scrubbed`).
- **`update`** usa el `install.sh` del último release, no el de `main`.
- **`recall`** encuentra una palabra que solo está en el título, y calcula el vector de la consulta una sola vez para todos los lugares.
- **El sueño corre cuando la memoria está inactiva**, a cualquier hora (el timer despierta cada 30 minutos). Las lecturas cuentan como uso, la fusión se hace como mucho una vez cada 20 horas (en Clio, que es quien une) y `doctor` da `FAIL` si lleva demasiado sin hacerse.

Después de fusionar y etiquetar: correr `clio-memory update`, reiniciar las sesiones de agente abiertas y, en Clio, llamar a `clio_memory::private_runtime()` al empezar `main` (si no, el Granite de Clio se rechaza y vuelve al modelo estático) y correr `clio memory setup sleep` para que el timer tome el horario nuevo.

## Qué hace cada trabajo

El workflow corre al empujar una etiqueta `v*`. Cada trabajo espera al anterior, así que si uno falla, los siguientes no corren.

1. **`version`**: compara la etiqueta con la versión de `Cargo.toml`.
2. **`check`**: llama a `ci.yml` (`workflow_call`) sobre el commit etiquetado. Son las mismas verificaciones de cualquier PR: `fmt`, `clippy -D warnings`, `test --all-features` en Ubuntu y macOS, `cargo deny`, `cargo audit`, `gitleaks` y `shellcheck install.sh`. En las etiquetas, `gitleaks` corre el binario de la CLI (v8.30.1, fijado por sha256) sobre todo el historial y respeta `.gitleaksignore`, porque `gitleaks-action` no escanea nada en un push de etiqueta sin commits nuevos.
3. **`build`**: compila con las features `model2vec,granite`. Cada binario sube como artefacto con el nombre de su asset.

   | Target | Runner | Cómo |
   |---|---|---|
   | `x86_64-unknown-linux-gnu` | `ubuntu-latest` | `cargo zigbuild` contra glibc 2.28 (`<target>.2.28`) |
   | `aarch64-unknown-linux-gnu` | `ubuntu-latest` | `cargo zigbuild` contra glibc 2.28 (`<target>.2.28`) |
   | `aarch64-apple-darwin` | `macos-latest` | `cargo build` |
   | `x86_64-apple-darwin` | `macos-latest` | `cargo build` (compilación cruzada) |
4. **`smoke`**: corre cada binario donde se puede y comprueba que `--version` dice `clio-memory <versión de la etiqueta>`.
   - `x86_64-unknown-linux-gnu` en `ubuntu-latest`, `aarch64-unknown-linux-gnu` en `ubuntu-24.04-arm` y `aarch64-apple-darwin` en `macos-latest`.
   - `x86_64-apple-darwin` solo si `arch -x86_64 true` funciona en el runner; si no, se salta y el resumen del trabajo lo dice. En esa plataforma solo se prueba `--version`, porque bajo Rosetta `install.sh` elegiría el binario de arm64.
   - En Linux comprueba también que el símbolo `GLIBC_` más alto (`objdump -T`) no pase de `GLIBC_2.28`.
   - Hace una instalación de prueba con `install.sh` sobre los binarios recién construidos (`CLIO_MEMORY_DOWNLOAD_URL=file://…`, `CLIO_MEMORY_VERSION`, `CLIO_MEMORY_SKIP_ATTESTATION=1` y `CLIO_MEMORY_HAND_OVER=0`, que instala sin correr `setup`) y corre `~/.local/bin/clio-memory --version`.
5. **`publish`**: copia `install.sh` del commit etiquetado, genera `SHA256SUMS` (de los binarios y de `install.sh`), firma la procedencia de los 4 binarios con `actions/attest-build-provenance` y crea el release con `gh release create "$TAG" clio-memory-* install.sh SHA256SUMS --generate-notes --verify-tag`, sin `--draft`. Es el único trabajo con `contents: write`, `id-token: write` y `attestations: write`; los demás solo leen. No usa secretos más allá del `GITHUB_TOKEN`.

Cada release lleva los 4 binarios (`clio-memory-x86_64-unknown-linux-gnu`, `clio-memory-aarch64-unknown-linux-gnu`, `clio-memory-aarch64-apple-darwin`, `clio-memory-x86_64-apple-darwin`), `install.sh` y `SHA256SUMS`.

## Si algo falla

- **Antes de `publish`:** no queda nada publicado. Se borra la etiqueta, se corrige y se etiqueta otra vez:

  ```sh
  git push --delete origin vX.Y.Z
  git tag -d vX.Y.Z
  ```

  Si la corrección es un commit nuevo, se vuelve a etiquetar ese commit con el mismo número.
- **En `publish`, antes de que exista el release** (falla la atestación o `gh`; `gh` borra su propio borrador si algo se corta): no queda nada publicado. Se borra la etiqueta y se vuelve a empujar la misma, como en el caso anterior.
- **En `publish`, después de crear el release:** el release es inmutable, así que no se corrige ni se borra. Se arregla con la siguiente versión (`vX.Y.Z+1`).

## Confianza en el binario, y su límite

- **Releases inmutables.** El repositorio debe tener activada la opción *immutable releases* de GitHub. Una vez publicado un release, ni sus assets ni su etiqueta se pueden cambiar ni borrar, así que el binario y las sumas de una versión quedan fijos. Para colar un binario falso habría que publicar una versión nueva, y eso exige escribir en el repositorio y pasar el workflow. GitHub, además, firma una atestación del release.
- **Sumas en el release.** `install.sh` baja el binario y `SHA256SUMS` de la misma versión y compara. Eso detecta un archivo dañado o cortado. Que el archivo sea legítimo lo garantiza la inmutabilidad, no la suma.
- **Atestaciones de procedencia.** El workflow firma cada binario con `actions/attest-build-provenance`, que lo liga al commit y al workflow que lo construyó. Si la máquina tiene `gh` con sesión y con `gh attestation` (2.49 o más nuevo), `install.sh` corre `gh attestation verify`: si falla, por la razón que sea (una atestación que no corresponde, o la red), la instalación se detiene y muestra debajo lo que dijo `gh`. Si `gh` no está, no tiene sesión o es más viejo que 2.49, sigue con una línea en gris que lo dice. Las descargas por https no aceptan una redirección que salga de https (`--proto-redir =https`).
- **El límite.** Sin un humano en medio, la confianza termina en la cuenta de GitHub de Iván y en el workflow. Quien controle eso puede publicar una versión. Las sumas escritas en el repositorio y fusionadas a mano habrían agregado una revisión humana, pero se eligió no tener ese paso.

## Configuración del repositorio de la que depende

### Immutable releases

Estado: **activado.** Verificado el 2026-11-15: `gh api repos/i-montes/clio-memory/immutable-releases` dice `"enabled": true` y v0.9.0 es inmutable.

Revisarlo (necesita permiso de administración):

```sh
gh api repos/i-montes/clio-memory/immutable-releases
# {"enabled":true,"enforced_by_owner":false}   <- activado (el estado de hoy)
# {"enabled":false,...}                         <- desactivado: volver a activarlo
```

Si alguna vez apareciera desactivado, se activa así (responde `204`):

```sh
gh api -X PUT repos/i-montes/clio-memory/immutable-releases
```

Desactivarlo sería `-X DELETE` sobre la misma ruta; no se debe hacer. Hay que mantenerlo activado: la inmutabilidad solo protege los releases publicados mientras lo está. `update` depende de ello, porque corre el `install.sh` del último release.

### Permisos de Actions

El workflow declara sus permisos en cada trabajo (`contents: write`, `id-token: write` y `attestations: write` solo en `publish`), así que no necesita que el permiso por defecto del repositorio sea de escritura. Hoy es de solo lectura, y así debe seguir:

```sh
gh api repos/i-montes/clio-memory/actions/permissions/workflow
# {"default_workflow_permissions":"read","can_approve_pull_request_reviews":false}
```

Lo que sí hace falta es que Actions esté habilitado y permita las acciones que usa el workflow (todas fijadas por SHA de commit, con su versión en un comentario).

## Verificación a mano

Con los archivos del release descargados en una carpeta:

```sh
sha256sum -c SHA256SUMS --ignore-missing
gh attestation verify clio-memory-x86_64-unknown-linux-gnu --repo i-montes/clio-memory
```

`--ignore-missing` evita que falle por los binarios de otras plataformas que no se bajaron. En macOS, `shasum -a 256 -c --ignore-missing SHA256SUMS` hace lo mismo que `sha256sum -c --ignore-missing`.
