# Instalador de clio-memory — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Con una línea (`curl -fsSL …/install.sh | sh`) se instala el binario de clio-memory que corresponde a la máquina, y una TUI con la marca de Clio conecta la memoria a Claude Code, OpenCode, Codex, Gemini CLI y/o Antigravity.

**Architecture:** Hay tres partes, y cada una cierra en su propio PR:

1. **Release e `install.sh`.** Un workflow de release, al empujar una etiqueta, verifica, compila, firma y publica binarios inmutables. `install.sh` es POSIX `sh`: baja el binario que toca, lo verifica y le pasa el control.
2. **La lógica de setup.** Vive en la biblioteca (`src/setup/`) como funciones puras o inyectables: detección, MCP, skill, instrucciones, orquestación, `uninstall` y `doctor`.
3. **La TUI.** `clio-memory setup` la dibuja con `cliclack` y la paleta del canal. Sin terminal, el mismo flujo imprime líneas planas.

**Tech Stack:** Rust 2024 (1.95), `cliclack` 0.5.7 (nueva), `serde_json` con `preserve_order` (nueva feature), POSIX `sh`, GitHub Actions, `cargo-zigbuild`, `actions/attest-build-provenance`.

**Spec:** `docs/superpowers/specs/2026-11-05-instalador-design.md`. Léelo antes de cada tarea: manda sobre este plan si chocan.

## Global Constraints

- Cada PR en su rama, desde `feat/installer`. Iván fusiona y etiqueta; nunca hacer push a `main` ni etiquetar sin que lo pida.
- TDD: la prueba primero. **Cada prueba nueva se verifica por mutación:** romper a mano la línea que la hace pasar, ver que falla y restaurar. Esto se anota en el mensaje del commit (`mutation-checked: <qué se rompió>`).
- Antes de cada commit: `cargo fmt --all --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings` y `cargo test --locked --all-features`. Además, `shellcheck install.sh` cuando se toca `install.sh`, y `gitleaks` limpio (el hook ya corre).
- `unsafe_code = "forbid"`, `unwrap_used = "deny"`. No usar `cargo +nightly`.
- Pruebas sin red y sin el `$HOME` real: `HOME` temporal y programas falsos (`claude`, `codex`, `gemini`) en un `PATH` de prueba.
- Textos para la persona en español con tildes; comentarios y nombres de código en inglés, con la densidad de comentarios del código vecino.
- Plataformas y nombres de assets, exactos:

  | `uname -s`/`-m` | target | asset |
  |---|---|---|
  | Linux x86_64 o amd64 | `x86_64-unknown-linux-gnu` | `clio-memory-x86_64-unknown-linux-gnu` |
  | Linux aarch64 o arm64 | `aarch64-unknown-linux-gnu` | `clio-memory-aarch64-unknown-linux-gnu` |
  | Darwin arm64 | `aarch64-apple-darwin` | `clio-memory-aarch64-apple-darwin` |
  | Darwin x86_64 | `x86_64-apple-darwin` | `clio-memory-x86_64-apple-darwin` |

  Cada release lleva además `SHA256SUMS`, con líneas `<sha256>  <asset>`.
- Linux: binarios glibc compatibles con **glibc 2.28** (zigbuild con `<target>.2.28`). Features del binario publicado: `model2vec,granite`.
- Paleta (truecolor):

  | Nombre | Hex |
  |---|---|
  | hueso | `#E6E6E0` |
  | cenizaClara | `#B8BCC4` |
  | ceniza | `#8A8F98` |
  | borde | `#2A2E35` |
  | ámbar | `#FFB13D` |
  | hielo | `#7FD6FF` |
  | verdeSuave | `#7FA88A` |
  | rojoApagado | `#B4554B` |

  La TUI no pinta el fondo.
- La línea de instrucciones, igual para todos los agentes: `- Mi memoria principal es clio-memory (MCP): úsala según la skill clio-memory.`
- El nombre del servidor MCP y de la skill es siempre `clio-memory`. El MCP se registra como `<ruta absoluta del binario> mcp`.
- Versión objetivo `0.6.0` (se sube en la última tarea).

## Review Focus

1. **Archivos de configuración ajenos con más cosas.** Por ejemplo, un `opencode.json` con otros servidores y claves, un `mcp_config.json` vacío de 0 bytes (así está en la máquina de Iván) o un `settings.json` con `mcpServers` de otros. Al conectar y desconectar, todo lo que no es `clio-memory` queda igual, en el mismo orden. Prueba en la Tarea 5.
2. **Instalaciones viejas.** La skill de v0.2.0 está instalada hoy en `~/.claude/skills/clio-memory/SKILL.md`, y un MCP puede apuntar a `~/.cargo/bin/clio-memory`. Ambos se reconocen como nuestros y desactualizados: se actualizan o se re-apuntan, nunca se tratan como ajenos. Pruebas en las Tareas 4 y 6.
3. **Correr dos veces, o cortar a mitad.** Sin binario a medias, sin líneas duplicadas en los archivos de instrucciones y sin una segunda entrada `instructions` en `opencode.json`. Pruebas en las Tareas 2, 7 y 8.
4. **Rutas con espacios** en `HOME` o en el binario. El MCP queda como arreglo de argumentos (`command` y `args`), nunca como una cadena partida. Prueba en la Tarea 6.
5. **El programa de un agente existe pero falla** (por ejemplo, `codex` sale con 1). Ese paso queda en ✖ con el motivo, los demás agentes se instalan y la salida es distinta de cero. Prueba en la Tarea 8.

---

## PR 1 — Release e `install.sh`

### Task 1: `clio-memory --version`

**Files:**
- Modify: `src/bin/clio-memory/args.rs` (`Parsed`, `parse`, `USAGE`)
- Modify: `src/bin/clio-memory/main.rs`
- Test: `tests/cli.rs`

**Interfaces:**
- Produces: `clio-memory --version` (y `-V`) imprime `clio-memory <CARGO_PKG_VERSION>` en stdout y sale con 0, sin abrir la base de datos. `install.sh` y el workflow dependen de esto.

- [ ] **Step 1: Escribir la prueba que falla.** `version_prints_the_crate_version`: correr el binario (como las otras pruebas de `tests/cli.rs`) con `--version` y con `-V`. Esperar stdout `format!("clio-memory {}\n", env!("CARGO_PKG_VERSION"))`, éxito, y que no se cree ningún archivo en un `CLIO_MEMORY_DATA` temporal.
- [ ] **Step 2: Ejecutarla.** `cargo test --all-features --test cli version_prints` → FALLA («unknown option --version»).
- [ ] **Step 3: Implementar.** Agregar `Parsed::Version` y aceptar `-V | --version` en el mismo `match` donde está `--help`. `main` lo imprime. Agregar `-V, --version` a la sección options de `USAGE`.
- [ ] **Step 4: Ejecutarla.** → PASA. Verificar por mutación: cambiar el texto impreso.
- [ ] **Step 5: Commit** `feat(cli): --version`.

### Task 2: `install.sh`

**Files:**
- Create: `install.sh` (raíz del repo, ejecutable)
- Test: `tests/install_sh.rs`

**Interfaces:**
- Consumes: `clio-memory --version` (Tarea 1), y `clio-memory setup --from-installer [--yes] [flags…]` (Tarea 11). Hasta que exista la Tarea 11, las pruebas usan un binario falso que registra sus argumentos.
- Produces: el contrato de variables de entorno que documenta `docs/instalador.md`:
  - `CLIO_MEMORY_VERSION=vX.Y.Z` fija la versión (por defecto, la última);
  - `CLIO_MEMORY_BIN_DIR` cambia la carpeta (por defecto `$HOME/.local/bin`);
  - `CLIO_MEMORY_DOWNLOAD_URL` es la base de descarga (por defecto `https://github.com/i-montes/clio-memory/releases`; las pruebas la apuntan a `file://`);
  - `CLIO_MEMORY_SKIP_ATTESTATION=1`;
  - `CLIO_MEMORY_HAND_OVER=0`, que instala sin hacer `exec` del setup (para el `smoke` del release; se documenta como tal).
  - Los argumentos tras `sh -s --` pasan tal cual a `setup`.

Comportamiento que fija el script:

1. `set -eu` y una carpeta temporal propia (`mktemp -d`), borrada con `trap … EXIT`.
2. Plataforma según la tabla de Global Constraints. En Darwin x86_64, si `sysctl -n sysctl.proc_translated` da `1` (Rosetta), se usa arm64. Cualquier otra cosa termina con el error de abajo y código 1, sin crear `BIN_DIR`.
3. URL de cada asset: `$BASE/latest/download/<asset>`, o `$BASE/download/$CLIO_MEMORY_VERSION/<asset>`. Bajar con `curl -fsSL --retry 3` el `SHA256SUMS` y el binario.
4. Calcular el sha con `sha256sum` o `shasum -a 256`; si no hay ninguno, error. La línea del asset debe existir en `SHA256SUMS` y coincidir. Si no coincide: `✖ La suma sha256 no coincide: no se instaló nada.` y código 1.
5. Atestación:
   - si `CLIO_MEMORY_SKIP_ATTESTATION` no está, existe `gh` y `gh auth status` sale con 0, entonces `gh attestation verify <archivo> --repo i-montes/clio-memory`;
   - si eso falla: `✖ La atestación de GitHub no corresponde a este binario: no se instaló nada.` y código 1;
   - si no se pudo intentar: una línea en ceniza, `· sin verificación de atestación (gh no está o no tiene sesión)`.
6. Si `BIN_DIR/clio-memory` existe con el mismo sha, no se reemplaza. Si no, `chmod 755` en la carpeta temporal y `mv` sobre el destino (atómico).
7. Le pasa el control al binario:
   - si `/dev/tty` se puede abrir (`(exec </dev/tty) 2>/dev/null`): `exec "$BIN" setup --from-installer "$@" </dev/tty`;
   - si no: `exec "$BIN" setup --from-installer --yes "$@"`.

Textos exactos (con color solo si stdout es terminal y no hay `NO_COLOR`; ámbar = `\033[38;2;255;177;61m`):

- encabezado: `┌  Clio memory · instalador`
- éxito de descarga: `◇  clio-memory <asset> · sha256 ✓`
- plataforma no soportada:
  - `✖  No hay binario para esta máquina · <uname -s> <uname -m>`
  - `   Hay para: Linux x86_64, Linux aarch64, macOS arm64, macOS x86_64.`
  - `   No se instaló nada.`

- [ ] **Step 1: Escribir las pruebas que fallan** en `tests/install_sh.rs`. Cada una corre `sh install.sh` con `HOME` temporal, `PATH` = carpeta de falsos + `/usr/bin:/bin`, y `CLIO_MEMORY_DOWNLOAD_URL=file://<tmp>/releases`, con los assets en `<tmp>/releases/latest/download/`. Los falsos que se ponen en el `PATH`: `uname`, que responde según las variables `FAKE_S` y `FAKE_M`, y `gh` cuando hace falta. El «binario» publicado es un script `sh` que agrega `$*` a un archivo de registro.
  - `unsupported_platform_installs_nothing`: `FAKE_S=FreeBSD FAKE_M=amd64` → código 1; stderr contiene `No hay binario para esta máquina · FreeBSD amd64` y `No se instaló nada.`; `~/.local/bin` no existe.
  - `wrong_sum_installs_nothing`: `SHA256SUMS` con otra suma → código 1, contiene `no coincide`, `~/.local/bin/clio-memory` no existe.
  - `installs_and_hands_over_without_tty`: Linux x86_64 → `~/.local/bin/clio-memory` existe con modo 755; el registro contiene `setup --from-installer --yes --agents codex` cuando se corre con `sh install.sh --agents codex` (stdin `/dev/null` y sin terminal de control: `setsid`).
  - `same_binary_is_not_replaced`: segunda corrida → el mtime del binario no cambia.
  - `pinned_version_uses_that_tag`: `CLIO_MEMORY_VERSION=v9.9.9`, con los assets en `releases/download/v9.9.9/` → instala.
  - `failed_attestation_installs_nothing`: `gh` falso con `auth status` → 0 y `attestation verify` → 1 → código 1, contiene `atestación`, no hay binario.
  - `gh_without_session_skips_attestation`: `gh auth status` → 1 → instala, y stdout contiene `sin verificación de atestación`.
  - `hand_over_can_be_skipped`: `CLIO_MEMORY_HAND_OVER=0` → binario instalado y registro vacío.
  - `rosetta_uses_arm64`: `FAKE_S=Darwin FAKE_M=x86_64` y un `sysctl` falso que imprime 1 → baja `clio-memory-aarch64-apple-darwin`.
- [ ] **Step 2: Ejecutarlas.** `cargo test --test install_sh` → FALLAN (no existe `install.sh`).
- [ ] **Step 3: Escribir `install.sh`** con el comportamiento de arriba. Una función por paso (`platform`, `fetch`, `check_sum`, `check_attestation`, `put`, `hand_over`).
- [ ] **Step 4: Ejecutar** `cargo test --test install_sh` → PASAN. `shellcheck install.sh` → limpio. Verificar por mutación cada prueba (por ejemplo, saltarse la comparación de suma).
- [ ] **Step 5: Commit** `feat: install.sh — one line, the right binary or nothing`.

### Task 3: Workflow de release, `shellcheck` en CI y `docs/releases.md`

**Files:**
- Create: `.github/workflows/release.yml`
- Modify: `.github/workflows/ci.yml` (trabajo `shell`: `shellcheck install.sh`)
- Create: `docs/releases.md`

**Interfaces:**
- Consumes: `--version` (Tarea 1), `install.sh` (Tarea 2) y los nombres de assets de Global Constraints.
- Produces: un release publicado e inmutable por cada etiqueta `v*`, con los 4 assets y `SHA256SUMS`.

`release.yml`, `on: push: tags: ['v*']`, con acciones fijadas por SHA como en `ci.yml`. Los trabajos:

1. **`check`** (`contents: read`):
   - la etiqueta sin `v` debe ser igual a `version` en `Cargo.toml` (`cargo metadata --no-deps --format-version 1 | jq`); si no, falla;
   - luego las mismas verificaciones que `ci.yml`: `fmt`, `clippy`, `test` en ubuntu y macOS, `deny`, `audit`, `gitleaks` y `shellcheck`.

   Se pueden reutilizar llamando a `ci.yml` con `workflow_call`: agregar ese disparador a `ci.yml`.
2. **`build`** (`needs: check`, `contents: read`), en una matriz:
   - `x86_64-unknown-linux-gnu` y `aarch64-unknown-linux-gnu`: en `ubuntu-latest`, con `cargo zigbuild --release --locked --features model2vec,granite --target <t>.2.28`;
   - `aarch64-apple-darwin` y `x86_64-apple-darwin`: en `macos-latest`, con `cargo build`.

   El binario se renombra al asset y se sube como artefacto.
3. **`smoke`** (`needs: build`):
   - corre `--version` y comprueba que coincide con la etiqueta en `ubuntu-latest` (x86_64), `ubuntu-24.04-arm` (aarch64) y `macos-latest` (arm64);
   - en x86_64-apple-darwin, solo si `arch -x86_64 true` funciona en el runner; si no, se salta y lo dice en el resumen;
   - en Linux, comprueba también `objdump -T <bin> | grep -o 'GLIBC_[0-9.]*' | sort -V | tail -1` ≤ `GLIBC_2.28`;
   - además, una instalación de prueba con `install.sh`: los artefactos en una carpeta con `SHA256SUMS`, `CLIO_MEMORY_DOWNLOAD_URL=file://…`, `CLIO_MEMORY_SKIP_ATTESTATION=1`, y la orden de setup cambiada por `--version`.

     Para lo último se usa `CLIO_MEMORY_HAND_OVER=0` (Tarea 2) y luego se corre `~/.local/bin/clio-memory --version`.
4. **`publish`** (`needs: smoke`, `contents: write`, `id-token: write`, `attestations: write`):
   - `sha256sum clio-memory-* > SHA256SUMS`;
   - `actions/attest-build-provenance` sobre los 4 binarios;
   - `gh release create "$TAG" clio-memory-* SHA256SUMS --title "$TAG" --generate-notes --verify-tag`, sin `--draft`.

`docs/releases.md` (en español) cubre:
- cómo publicar: subir `version` en `Cargo.toml` por PR, fusionar, `git tag vX.Y.Z && git push origin vX.Y.Z`;
- qué hace cada trabajo;
- si falla antes de `publish`, no queda nada publicado: se borra la etiqueta (`git push --delete origin vX.Y.Z`), se corrige y se etiqueta otra vez; si falla en `publish` tras crear el release, se arregla con la siguiente versión, porque es inmutable;
- el modelo de confianza y su límite, copiado del spec;
- la configuración del repo de la que depende, con el comando para revisarla:
  - *immutable releases* activado;
  - Actions con permiso de escritura donde lo pide el workflow;
- verificación a mano: `sha256sum -c`, `gh attestation verify <bin> --repo i-montes/clio-memory`.

- [ ] **Step 1: Escribir `release.yml` y el trabajo `shell` de `ci.yml`.** Validar la sintaxis con `python3 -c 'import yaml,sys; yaml.safe_load(open(sys.argv[1]))' .github/workflows/release.yml` y con `gh workflow view` tras empujar la rama. No hay pruebas automáticas posibles más allá de eso: la prueba real es el primer release.
- [ ] **Step 2: Probar zigbuild localmente para Linux x86_64.** `cargo zigbuild` no está instalado. Instalarlo en el sistema de desarrollo solo si Iván lo aprueba; si no, confiar en el `smoke` del CI. Anotar cuál de las dos se hizo.
- [ ] **Step 3: Escribir `docs/releases.md`.**
- [ ] **Step 4: Activar los releases inmutables en el repo.** Primero averiguar el endpoint exacto en la documentación de GitHub REST: es `PUT /repos/{owner}/{repo}/immutable-releases` si existe con ese nombre. Antes de correrlo, decirle a Iván en una línea qué se va a activar. Luego verificarlo con el `GET` correspondiente y anotar el comando en `docs/releases.md`.
- [ ] **Step 5: Commit** `ci: release on tag — checked, built for 4 platforms, attested, immutable`. Abrir el PR 1 (`gh pr create`) con el resumen y el pie de atribución.

---

## PR 2 — Agentes, `uninstall` y `doctor`

### Task 4: Módulo `setup/`, agentes, detección y skill

**Files:**
- Move: `src/setup.rs` → `src/setup/mod.rs` (con `git mv`, sin cambios de contenido en este paso)
- Create: `src/setup/agents.rs`, `src/setup/skill.rs`
- Modify: `skill/SKILL.md`
- Test: `tests/setup_agents.rs`

**Interfaces:**
- Produces (en `clio_memory::setup::agents`):
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)] pub enum AgentId { ClaudeCode, OpenCode, Codex, Gemini, Antigravity }`, con `pub const ALL: [AgentId; 5]` en ese orden. Además:
    - `pub fn key(self) -> &'static str`: `"claude-code"`, `"opencode"`, `"codex"`, `"gemini"`, `"antigravity"`;
    - `pub fn label(self) -> &'static str`: `"Claude Code"`, `"OpenCode"`, `"Codex"`, `"Gemini CLI"`, `"Antigravity"`;
    - `pub fn from_key(s: &str) -> Option<AgentId>`.
  - `pub struct Env<'a>` con estos campos públicos, y `Env::program(&self, name: &str) -> Option<PathBuf>`, que busca un archivo ejecutable con ese nombre en `path`:
    - `home: PathBuf`;
    - `exe: PathBuf`;
    - `path: Vec<PathBuf>` (los directorios del `PATH`);
    - `codex_home: Option<PathBuf>` (`CODEX_HOME`);
    - `claude: Option<PathBuf>` (`CLIO_MEMORY_CLAUDE`);
    - `service: &'a dyn Fn(&[&str]) -> Result<(), String>`.
  - `pub struct Found { pub agent: AgentId, pub at: PathBuf }`.
  - `pub fn detect(env: &Env) -> Vec<Found>`, en el orden de `ALL`:
    - Claude Code: `env.claude` o el programa `claude`;
    - OpenCode: el programa `opencode`, o si no, la carpeta `~/.config/opencode/`;
    - Codex: el programa `codex`;
    - Gemini: el programa `gemini`;
    - Antigravity: el programa `agy`, o si no `antigravity`, o si no la carpeta `~/.gemini/config/`.
- Produces (en `clio_memory::setup::skill`):
  - `pub enum SkillDir { Claude, OpenCode, Agents, Antigravity }`;
  - `SkillDir::path(self, home: &Path) -> PathBuf`, que da `~/.claude/skills`, `~/.config/opencode/skills`, `~/.agents/skills` o `~/.gemini/config/skills`, más `/clio-memory/SKILL.md`;
  - `SkillDir::readers(self) -> &'static [AgentId]`:

    | Carpeta | La leen |
    |---|---|
    | Claude | ClaudeCode, OpenCode |
    | OpenCode | OpenCode |
    | Agents | OpenCode, Codex, Gemini |
    | Antigravity | Antigravity |

  - `pub fn placement(agents: &[AgentId]) -> Vec<SkillDir>`;
  - `pub const KNOWN_SKILLS: &[&str]`: los sha256 de las versiones publicadas, que son:
    - `e748bfac08f18e3288eaea2808a7609112775bd6586512732c931b97c135404a` (v0.2.0);
    - `c2482bda5826a43537eeb0773e88a7b96ba17dcafd062d47d63e2a8d01bce1c4` (v0.4.0);
    - `73eb801b86507af87bd643f0dd70478abddcb8418595b06f2ac70c1fb707524b` (v0.5.0);
    - y el de `SKILL` actual, calculado en tiempo de ejecución.
  - `pub enum SkillFile { Absent, Current, Old, Foreign }` y `pub fn skill_state(path: &Path) -> SkillFile`;
  - `pub fn place_skill(home: &Path, agents: &[AgentId], replace: bool) -> Result<Vec<String>, String>`: escribe la skill actual en cada carpeta de `placement(agents)` y borra la nuestra (`Current` u `Old`) de las demás. Una `Foreign` sin `replace` es un `Err` que nombra la ruta y no escribe nada.

El algoritmo de `placement`: entre los 16 subconjuntos de las 4 carpetas, tomar los que cubren a todos los agentes de la lista. De ellos, escoger el que tenga (1) menos agentes que ven dos copias, (2) menos carpetas y (3) el primero en el orden `Claude, OpenCode, Agents, Antigravity`. Con la lista vacía, devuelve vacío.

- [ ] **Step 1: `git mv src/setup.rs src/setup/mod.rs`**, declarar `pub mod agents; pub mod skill;` y correr `cargo test --all-features` → todo pasa igual.
- [ ] **Step 2: Escribir las pruebas que fallan** en `tests/setup_agents.rs`:
  - `placement_cases`, una tabla con estos casos:

    | Agentes | Carpetas |
    |---|---|
    | `[ClaudeCode]` | `[Claude]` |
    | `[OpenCode]` | `[OpenCode]` |
    | `[ClaudeCode, OpenCode]` | `[Claude]` |
    | `[Codex, Gemini]` | `[Agents]` |
    | `[ClaudeCode, OpenCode, Codex]` | `[Claude, Agents]` |
    | `[Gemini, Antigravity]` | `[Agents, Antigravity]` |
    | todos | `[Claude, Agents, Antigravity]` |
    | `[]` | `[]` |

  - `every_reader_sees_the_skill`: para los 31 subconjuntos no vacíos de `ALL`, cada agente aparece en el `readers()` de al menos una carpeta elegida;
  - `old_published_skill_is_ours`: un archivo con el contenido de v0.2.0, obtenido con `git show e7915d4:skill/SKILL.md` y guardado en `tests/fixtures/skill-v0.2.0.md`, da `Old`; un texto cualquiera da `Foreign`;
  - `place_skill_moves_between_dirs`: primero `[ClaudeCode]` y luego `[Codex]` → `~/.claude/skills/clio-memory` desaparece y aparece `~/.agents/skills/clio-memory/SKILL.md`;
  - `foreign_skill_blocks_without_replace`: `Err` que contiene la ruta, sin nada escrito; con `replace`, se escribe;
  - `detect_finds_by_program_and_by_folder`: un `PATH` falso con `codex` y `gemini`, y la carpeta `~/.gemini/config/` → `[Codex (at = ruta del programa), Gemini, Antigravity (at = la carpeta)]`.
- [ ] **Step 3: Ejecutarlas** → FALLAN.
- [ ] **Step 4: Implementar `agents.rs` y `skill.rs`** con las firmas de arriba.
- [ ] **Step 5: Hacer la skill neutral respecto del agente.** En `skill/SKILL.md`, cambiar «It takes precedence over Claude Code's file-based memory» por «It takes precedence over any other memory you have (memory files, notes in AGENTS.md or similar)». Revisar el resto del texto por menciones a Claude Code y dejar todo lo demás igual.
- [ ] **Step 6: Ejecutar** todas las pruebas → PASAN. Verificar por mutación (por ejemplo, invertir el criterio de desempate 1).
- [ ] **Step 7: Commit** `feat(setup): five agents, where each reads skills, one skill placement`.

### Task 5: Editar configuraciones JSON sin tocar lo ajeno

**Files:**
- Create: `src/setup/config_file.rs`
- Modify: `Cargo.toml` (`serde_json = { version = "1.0.154", features = ["preserve_order"] }`)
- Test: `tests/setup_config_file.rs`

**Interfaces:**
- Produces:
  - `pub fn read_json(path: &Path) -> Result<serde_json::Value, String>`. Si el archivo no existe o está vacío (incluidos 0 bytes y solo espacios), devuelve `{}`. Si no es JSON válido, un `Err` que nombra el archivo y dice «tiene comentarios o no es JSON: edítalo a mano o quítale los comentarios».
  - `pub fn write_json(path: &Path, value: &Value) -> Result<(), String>`. Escribe con `serde_json::to_string_pretty` más un salto de línea final, a un temporal y luego renombra. Conserva los permisos del archivo si ya existía; si es nuevo, usa 0600 y crea las carpetas que falten.
  - `pub fn opencode_config(home: &Path) -> PathBuf`: `~/.config/opencode/opencode.json`, o `opencode.jsonc` si solo existe ese.

- [ ] **Step 1: Pruebas que fallan:**
  - `untouched_keys_keep_their_order`: leer `{"$schema":"x","theme":"system","mcp":{"other":{"type":"local","command":["a"]}},"autoupdate":false}`, insertar `mcp.clio-memory`, escribir, releer → el orden de las claves de primer nivel y de `mcp` es el original, con `clio-memory` al final, y `other` intacto;
  - `empty_file_is_an_empty_object`: un archivo de 0 bytes → `{}`;
  - `comments_are_refused_and_nothing_is_written`: un `.jsonc` con `// x` → `Err` con «comentarios», y el archivo queda byte a byte igual;
  - `mode_is_kept`: un archivo con 0644 sigue con 0644.
- [ ] **Step 2: Ejecutarlas** → FALLAN.
- [ ] **Step 3: Implementar.** Activar `preserve_order` y correr **toda** la batería: si alguna prueba existente dependía del orden alfabético de un `Map`, ajustar esa prueba (no el código del motor) y explicarlo en el commit.
- [ ] **Step 4: Ejecutarlas** → PASAN. `cargo deny check` limpio (`indexmap` es MIT/Apache). Verificar por mutación.
- [ ] **Step 5: Commit** `feat(setup): edit an agent's JSON config touching only our key`.

### Task 6: El servidor MCP en cada agente

**Files:**
- Create: `src/setup/mcp.rs`
- Test: `tests/setup_mcp.rs` (con programas falsos `claude`, `codex` y `gemini`, en el estilo de `fake_claude` de `tests/setup.rs`; moverlos a `tests/common/fakes.rs` para reusarlos)

**Interfaces:**
- Consumes: `Env` y `AgentId` (Tarea 4); `read_json`, `write_json` y `opencode_config` (Tarea 5).
- Produces:
  - `pub enum McpState { Absent, Ours, OtherPath(PathBuf), Foreign(String) }`. `OtherPath` es una entrada cuyo comando se llama `clio-memory`, con args `["mcp"]` y otra ruta. `Foreign` es cualquier otra cosa, y lleva su descripción.
  - `pub fn mcp_state(env: &Env, agent: AgentId) -> Result<McpState, String>`.
  - `pub fn mcp_connect(env: &Env, agent: AgentId) -> Result<(), String>`: si hay algo, lo quita primero.
  - `pub fn mcp_disconnect(env: &Env, agent: AgentId) -> Result<(), String>`: no hace nada si no hay entrada.

Cómo se hace en cada agente:

| Agente | Estado | Conectar | Quitar |
|---|---|---|---|
| Claude Code | `claude mcp get clio-memory` (código ≠0 = Absent); la salida contiene `<exe> mcp` = Ours | `claude mcp add --scope user clio-memory -- <exe> mcp` | `claude mcp remove --scope user clio-memory` |
| Codex | `codex mcp get clio-memory --json`, con `CODEX_HOME` pasado tal cual (código ≠0 = Absent) | `codex mcp add clio-memory -- <exe> mcp` | `codex mcp remove clio-memory` |
| Gemini | leer `~/.gemini/settings.json` → `mcpServers.clio-memory.{command,args}` | `gemini mcp add -s user clio-memory <exe> mcp` | `gemini mcp remove -s user clio-memory` |
| OpenCode | `opencode_config` → `mcp.clio-memory.command` (arreglo) | escribir `mcp.clio-memory = {"type":"local","command":[<exe>,"mcp"],"enabled":true}` | quitar la clave; si `mcp` queda vacío, quitar `mcp` |
| Antigravity | `~/.gemini/config/mcp_config.json` → `mcpServers.clio-memory` | escribir `mcpServers.clio-memory = {"command":<exe>,"args":["mcp"]}` | quitar la clave (y `mcpServers` si queda vacío) |

Para Codex, antes de fijar el parser hay que mirar la forma real de `codex mcp get --json`. Correr en la máquina `codex mcp add clio-memory-probe -- /bin/true mcp`, luego `codex mcp get clio-memory-probe --json` y `codex mcp remove clio-memory-probe`, y anotar esa forma en un comentario. El falso de las pruebas imita esa forma.

- [ ] **Step 1: Pruebas que fallan:**
  - `connect_each_agent`: con `exe = "/opt/my bin/clio-memory"` (con espacio), por cada agente → `mcp_state` da `Ours`. Los falsos registran sus argumentos como un arreglo, uno por línea: `/opt/my bin/clio-memory` llega como un solo argumento. En OpenCode y Antigravity, el JSON tiene la ruta como un solo elemento.
  - `old_cargo_install_is_other_path`: una entrada OpenCode con `["/home/x/.cargo/bin/clio-memory","mcp"]` → `OtherPath("/home/x/.cargo/bin/clio-memory")`; con `["clio","memory","mcp"]` → `Foreign`.
  - `disconnect_leaves_the_rest`: una configuración de Antigravity con otro servidor `x` → tras conectar y desconectar, el archivo es igual al original.
  - `failing_program_is_an_error`: `codex` falso que sale con 1 en `mcp add` → `Err` que contiene «codex».
- [ ] **Step 2: Ejecutarlas** → FALLAN.
- [ ] **Step 3: Implementar.** Reusar la lógica de reintento por ETXTBSY de `ClaudeCode::claude`, sacándola a `fn run_program(program: &Path, args: &[&str]) -> Result<String, String>` en `src/setup/mod.rs`.
- [ ] **Step 4: Ejecutarlas** → PASAN. Verificar por mutación.
- [ ] **Step 5: Commit** `feat(setup): register the MCP server in each agent`.

### Task 7: La línea de instrucciones en cada agente

**Files:**
- Create: `src/setup/instructions.rs`
- Modify: `src/setup/mod.rs` (`pub const INSTRUCTIONS_LINE`; `CLAUDE_MD_LINE` queda como alias)
- Test: `tests/setup_instructions.rs`

**Interfaces:**
- Produces:
  - `pub fn instructions_file(env: &Env, agent: AgentId) -> PathBuf`:
    - Claude Code: `~/.claude/CLAUDE.md`;
    - Codex: `$CODEX_HOME` o `~/.codex`, y dentro `AGENTS.override.md` si existe y no está vacío; si no, `AGENTS.md`;
    - Gemini y Antigravity: `~/.gemini/GEMINI.md`;
    - OpenCode: `~/.config/opencode/clio-memory.md`.
  - `pub fn has_instructions(env: &Env, agent: AgentId) -> bool`.
  - `pub fn add_instructions(env: &Env, agent: AgentId) -> Result<Option<PathBuf>, String>`: devuelve `None` si ya estaba. Agrega la línea al final, con el mismo cuidado del salto de línea que hoy tiene `ClaudeCode::run`. En OpenCode, además, escribe `clio-memory.md` con la línea y agrega la ruta absoluta de ese archivo al arreglo `instructions` de `opencode.json`, una sola vez.
  - `pub fn remove_instructions(env: &Env, agent: AgentId, still: &[AgentId]) -> Result<Option<PathBuf>, String>`: quita solo líneas iguales a `INSTRUCTIONS_LINE` (con `trim`). En `GEMINI.md` no quita nada si el otro agente de Gemini sigue en `still`. En OpenCode, borra `clio-memory.md` y su entrada en `instructions`.

- [ ] **Step 1: Pruebas que fallan:**
  - `add_is_idempotent`: dos veces por agente → la línea aparece una sola vez; en OpenCode, `instructions` tiene la ruta una sola vez;
  - `opencode_never_creates_agents_md`: después de `add_instructions(OpenCode)`, `~/.config/opencode/AGENTS.md` no existe;
  - `gemini_md_is_shared`: con Gemini y Antigravity conectados, quitar Gemini (`still = [Antigravity]`) → la línea sigue; quitar Antigravity con `still = []` → la línea sale;
  - `remove_keeps_other_lines`: un `CLAUDE.md` con `# Mis reglas\n- corto\n<línea>\n` → queda `# Mis reglas\n- corto\n`;
  - `codex_override_wins`: con un `AGENTS.override.md` no vacío → la línea va ahí.
- [ ] **Step 2: Ejecutarlas** → FALLAN.
- [ ] **Step 3: Implementar.**
- [ ] **Step 4: Ejecutarlas** → PASAN. Verificar por mutación.
- [ ] **Step 5: Commit** `feat(setup): the instructions line for each agent`.

### Task 8: Orquestación: `apply`, `uninstall`, `doctor`, y `setup claude-code` encima

**Files:**
- Create: `src/setup/plan.rs`
- Modify: `src/setup/mod.rs` (`ClaudeCode::run` delega en `apply`; la parte del sueño se mueve a `pub fn ensure_sleep(env: &Env) -> Result<String, String>` sin cambiar sus textos)
- Modify: `src/bin/clio-memory/args.rs`, `main.rs`, `commands.rs` (`uninstall`, y las líneas de agentes en `doctor`)
- Test: `tests/setup_plan.rs`; `tests/setup.rs` sigue pasando sin cambios en sus asserts

**Interfaces:**
- Consumes: Tareas 4 a 7.
- Produces:
  - `pub struct Selection { pub agents: Vec<AgentId>, pub instructions: bool, pub replace: bool, pub exclusive: bool }`. `exclusive` lo pone solo la TUI: los agentes conectados que no están en `agents` se desconectan.
  - `pub struct Step { pub agent: Option<AgentId>, pub what: String, pub outcome: Result<String, String> }`.
  - `pub fn connected(env: &Env) -> Vec<AgentId>`: los que tienen `McpState::Ours` u `OtherPath`.
  - `pub fn apply(env: &Env, sel: &Selection, ask: &dyn Fn(&str) -> bool) -> Vec<Step>`. Por cada agente de `sel.agents`:
    1. si el programa o la carpeta no existe, un `Step` con `Err("no se encontró <label>")`;
    2. si hay algo ajeno (`Foreign` en el MCP o en la skill) y no hay `replace`, `ask("<label> ya tiene … ¿Reemplazarlo?")`; con un no, `Err` y **nada** escrito para ese agente;
    3. si es `OtherPath`, se vuelve a apuntar sin preguntar y el `Ok` lo dice: `apuntaba a <ruta>; ahora a <exe>`;
    4. `mcp_connect`;
    5. `add_instructions` si `sel.instructions`.

    Después: si `exclusive`, se desconectan los sobrantes. Luego `place_skill(conectados finales)`, que es un `Step` con `agent: None`, y al final `ensure_sleep`. Un fallo en un agente no detiene a los demás.
  - `pub fn uninstall(env: &Env, agents: Option<&[AgentId]>) -> Vec<Step>`. Con `None`: todos los conectados, el sueño (`remove_sleep`) y, si `env.exe` está en `$CLIO_MEMORY_BIN_DIR` o en `~/.local/bin`, el propio binario al final. Con una lista: solo esos agentes, y la skill se recoloca para los que quedan.
  - `pub fn agent_report(env: &Env) -> Vec<(AgentId, bool, String)>`: el `bool` dice si está bien, y el texto lleva el detalle.
  - CLI:
    - `clio-memory uninstall [--agents LISTA] [--models] [--data] [--yes]`. `--data` borra la carpeta de datos (la de `memory.db`): pregunta en una terminal y, sin terminal, exige `--yes`. `--models` borra `~/.cache/clio-models`.
    - `doctor` agrega, después de los checks actuales, una línea por agente detectado:
      - `ok   agente <key>: conectado`;
      - `FAIL agente <key>: <motivo>` (por ejemplo, `el MCP apunta a <ruta>` o `skill ajena en <ruta>`);
      - `--   agente <key>: no conectado`.

      Solo `FAIL` cambia el código de salida.

- [ ] **Step 1: Pruebas que fallan** en `tests/setup_plan.rs`, con fakes de los 3 programas y las carpetas de OpenCode y Antigravity:
  - `apply_all_connects_all_and_places_one_skill_each`: los 5 agentes → `connected` da los 5, y existen `~/.claude/skills/…`, `~/.agents/skills/…` y `~/.gemini/config/skills/…`, pero no `~/.config/opencode/skills/…` ni `~/.gemini/skills/…`;
  - `one_failing_agent_does_not_stop_the_rest` (Review Focus 5): `codex` falla en `mcp add` → su `Step` es `Err`, los otros 4 quedan conectados;
  - `declined_conflict_writes_nothing_for_that_agent`: un MCP ajeno en Gemini, con `ask` → false → nada cambia en `settings.json`, y la skill no aparece en `~/.agents/skills` por Gemini;
  - `old_install_is_repointed` (Review Focus 2): una entrada OpenCode a `~/.cargo/bin/clio-memory` → el `Ok` contiene `apuntaba a`, y la entrada nueva apunta a `exe`;
  - `exclusive_disconnects_the_unchecked`: conectar `[ClaudeCode, Codex]` y luego `apply` con `[ClaudeCode]` y `exclusive` → Codex queda sin MCP, sin línea y sin skill en `~/.agents`;
  - `non_exclusive_only_adds`: la misma secuencia sin `exclusive` → Codex sigue conectado;
  - `second_apply_changes_nothing` (Review Focus 3): mismos `mtime` en todos los archivos tocados;
  - `uninstall_all_leaves_memory_db`: crear `memory.db` en `CLIO_MEMORY_DATA`, correr `uninstall(None)` → no queda ningún agente conectado, los archivos del sueño no están, y `memory.db` sigue.
- [ ] **Step 2: Ejecutarlas** → FALLAN.
- [ ] **Step 3: Implementar `plan.rs`.** Reescribir `ClaudeCode::run` como `apply` con `Selection { agents: vec![ClaudeCode], instructions, replace, exclusive: false }`, donde `instructions` es `claude_md` o, si no viene, lo que responda `ask` con el texto actual. Convertir los `Step` a su `Result<Vec<String>, String>` de siempre: el primer `Err` es el error. Los asserts de `tests/setup.rs` no se cambian. Si alguno no pasa, el comportamiento nuevo está mal, no la prueba.
- [ ] **Step 4: CLI.** `uninstall` y `doctor` en `args.rs` y `commands.rs`, con pruebas en `tests/cli.rs`:
  - `uninstall_data_needs_yes_without_tty`: código 2 o 1 con el mensaje, y nada borrado;
  - `doctor_lists_agents`.
- [ ] **Step 5: Ejecutar todo** → PASA. Verificar por mutación.
- [ ] **Step 6: Commit** `feat(setup): apply a selection of agents, uninstall, doctor per agent`.

### Task 9: `docs/instalador.md` y abrir el PR 2

**Files:**
- Create: `docs/instalador.md`
- Modify: `README.md` (solo la tabla de agentes y `uninstall`; la sección de instalación cambia en la Tarea 13)

- [ ] **Step 1: Escribir `docs/instalador.md`**, verificando cada afirmación contra el código:
  - por agente: los archivos exactos y los comandos que corre, tal como quedaron en las Tareas 6 y 7;
  - dónde va la skill para cada combinación (la tabla de `placement_cases`) y el caso de las dos copias en OpenCode;
  - las banderas (que llegan en la Tarea 11; documentarlas ahí si este PR sale antes);
  - qué hace y qué no hace `uninstall`;
  - el `PATH`;
  - el modelo;
  - el `clientInfo.name` que manda de verdad cada agente. Se ve en la firma `mcp:<nombre>` de un recuerdo guardado desde cada uno, en la máquina: `clio-memory history <id>`. El agente que no se pueda probar queda como «sin verificar».
- [ ] **Step 2: Commit** `docs: what the installer touches in each agent`. Abrir el PR 2.

---

## PR 3 — La TUI

### Task 10: Elegir el modelo y bajar con progreso

**Files:**
- Create: `src/setup/model_choice.rs`
- Modify: `src/setup/mod.rs` (`ModelFile` gana `pub size: u64`; `curl_get_watching`)
- Test: `tests/setup_model_choice.rs`

**Interfaces:**
- Produces:
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum ModelChoice { Granite, Potion, None }`.
  - `pub fn granite_available(os: &str, arch: &str) -> bool`: es `!runtime_for(os, arch, None, "").is_empty()`, así que es falso en macOS x86_64.
  - `pub fn default_choice(os: &str, arch: &str) -> ModelChoice`: `Granite` si está disponible; si no, `Potion`.
  - `pub fn installed_best(home: &Path) -> ModelChoice`: comprueba los archivos de `GRANITE` y luego los de `POTION`, con el mismo criterio que usa `EmbedModel::of_home`, y si no hay ninguno da `None`.
  - `pub fn to_remove(home: &Path, choice: ModelChoice) -> Vec<(PathBuf, u64)>`: las carpetas que hay que borrar para que `choice` sea el que se use, con su tamaño. Con `Potion` sobre un Granite instalado: la carpeta de Granite y `RUNTIME_DIR`. Con `None`: además, la de potion.
  - `pub fn curl_get_watching(curl: &Path, url: &str, to: &Path, on_bytes: &dyn Fn(u64)) -> Result<(), String>`: corre `curl` como `curl_get`, y mientras corre lee cada 200 ms el tamaño de `to` y llama a `on_bytes`.
  - El `size` exacto de cada `ModelFile` de `GRANITE` y `POTION` se obtiene una vez con `curl -sIL <url> | grep -i content-length | tail -1` y se fija en el código. Una prueba comprueba que la suma de Granite está entre 1,2 y 1,3 GB, que es lo que dice el README.

- [ ] **Step 1: Pruebas que fallan:**
  - `no_granite_on_intel_mac`: `default_choice("macos","x86_64") == Potion` y `granite_available("linux","x86_64")`;
  - `installed_best_prefers_granite`: archivos falsos de los dos modelos → `Granite`; solo potion → `Potion`; nada → `None`;
  - `choosing_potion_over_granite_lists_granite_and_runtime`: lo que devuelve `to_remove` contiene las dos rutas, con tamaños mayores que 0;
  - `granite_size_is_pinned`: la suma de los tamaños está entre 1,2e9 y 1,3e9.
- [ ] **Step 2: Ejecutarlas** → FALLAN.
- [ ] **Step 3: Implementar** y fijar los tamaños.
- [ ] **Step 4: Ejecutarlas** → PASAN. Verificar por mutación.
- [ ] **Step 5: Commit** `feat(setup): choose the model; downloads report their bytes`.

### Task 11: `clio-memory setup`: argumentos y modo no interactivo

**Files:**
- Modify: `src/bin/clio-memory/args.rs` (`Command::Setup { … }`, `USAGE`)
- Create: `src/bin/clio-memory/setup_flow.rs`: el flujo completo, que llama a un trait de presentación
- Test: `tests/cli.rs` (`setup_*`)

**Interfaces:**
- Consumes: `apply`, `Selection`, `detect` y `ModelChoice` (Tareas 8 y 10).
- Produces:
  - `Command::Setup { yes: bool, from_installer: bool, agents: Option<Vec<AgentId>>, all: bool, model: Option<ModelChoice>, instructions: Option<bool>, replace: bool }`.
    - `setup` sin subcomando es esto.
    - `--agents all` pone `all`.
    - Un `key` desconocido en `--agents` es un error de uso que lista las claves válidas.
    - `--model granite|potion|none`.
    - `--instructions` y `--no-instructions` (las dos juntas es un error).
    - Los subcomandos actuales (`setup claude-code`, `model`, `sleep`) siguen igual.
  - `trait Screen` en `setup_flow.rs`. La implementan `Plain`, aquí, y `Tui`, en la Tarea 12:
    - `fn header(&mut self, version: &str)`;
    - `fn machine(&mut self, os: &str, arch: &str, gpu: Option<&str>)`;
    - `fn installed(&mut self, at: &Path)`;
    - `fn choose_agents(&mut self, found: &[Found], connected: &[AgentId]) -> Option<Vec<AgentId>>` (`None` = cancelado);
    - `fn choose_model(&mut self, options: &[ModelChoice], current: ModelChoice) -> Option<ModelChoice>`;
    - `fn confirm(&mut self, question: &str, default: bool) -> Option<bool>`;
    - `fn step(&mut self, step: &Step)`;
    - `fn download(&mut self, label: &str, total: Option<u64>) -> Box<dyn Fn(u64) + '_>`;
    - `fn note(&mut self, text: &str)`;
    - `fn done(&mut self, agents: &[AgentId], failed: bool)`.
  - `fn run(screen: &mut dyn Screen, env: &Env, args: SetupArgs) -> ExitCode`:
    - interactivo cuando no hay `--yes` **y** stderr es terminal; si no, `Plain` con los valores de la tabla del spec;
    - en no interactivo, `Selection.exclusive = false`, y `--model` nunca borra.
  - `fn path_hint(bin_dir: &Path, path_env: &str, shell: &str) -> Option<String>`:
    - `None` si `bin_dir` está en el `PATH`;
    - si no, según la shell: zsh → `echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc`; bash → lo mismo con `~/.bashrc`; fish → `fish_add_path ~/.local/bin`; cualquier otra → `~/.profile`.
  - Textos de `Plain` (una línea por evento):
    - `✓ <qué> · <detalle>` o `✖ <qué> · <motivo>`;
    - al final, `Listo. Abre <labels unidos por ", " y " y ">: la memoria ya está ahí.`;
    - la ayuda del `PATH`, si hace falta.

- [ ] **Step 1: Pruebas que fallan** en `tests/cli.rs`. Todas corren el binario con `HOME` temporal, `PATH` falso, sin terminal y con `CLIO_MEMORY_CLAUDE` apuntando al falso:
  - `setup_yes_connects_found_agents`: `setup --yes --model none` con `codex` y `gemini` falsos → código 0; stdout contiene `✓` para Codex y Gemini CLI, y `Listo. Abre Codex y Gemini CLI`;
  - `setup_agents_flag_only_adds`: `claude` ya conectado, luego `setup --yes --agents codex --model none` → Claude Code sigue conectado;
  - `setup_unknown_agent_is_usage_error`: `--agents cursor` → código 2, y stderr lista `claude-code, opencode, codex, gemini, antigravity`;
  - `setup_requested_but_missing_agent_fails_only_that_step`: `--agents opencode,codex` sin OpenCode → código 1, Codex conectado, y `✖ OpenCode` en la salida;
  - `path_hint_cases` (prueba de unidad en `setup_flow.rs`): los cuatro casos de shell, y `None` cuando `bin_dir` está en el `PATH`.
- [ ] **Step 2: Ejecutarlas** → FALLAN.
- [ ] **Step 3: Implementar** `args` y `setup_flow` con `Plain`. El modelo se instala por las funciones que ya existen (`install_model`, `install_runtime`), cambiando el `fetch` por uno que usa `curl_get_watching` y `Screen::download`. Antes, `to_remove`, previa confirmación (`confirm`, por defecto `false`), y solo en modo interactivo. Si en la TUI se desmarca un agente que estaba conectado, se pide `confirm("¿Desconecto <labels>?", false)` antes de usar `exclusive: true`; con un no, `exclusive` queda en `false`.
- [ ] **Step 4: Ejecutarlas** → PASAN. Verificar por mutación.
- [ ] **Step 5: Commit** `feat(setup): setup with no subcommand — the whole flow, plain without a terminal`.

### Task 12: La TUI con `cliclack` y la paleta

**Files:**
- Modify: `Cargo.toml` (`cliclack = "0.5.7"`, solo en el binario; sigue siendo una sola crate, así que va en `[dependencies]`)
- Create: `src/bin/clio-memory/tui.rs` (`Tui: Screen`), `src/bin/clio-memory/palette.rs`
- Test: pruebas de unidad en `palette.rs`; prueba manual guiada

**Interfaces:**
- Consumes: `trait Screen` (Tarea 11).
- Produces:
  - `pub enum ColorMode { TrueColor, Ansi256, None }`;
  - `pub fn color_mode(colorterm: Option<&str>, no_color: bool, is_term: bool) -> ColorMode`: `None` si `no_color` o si no es terminal; `TrueColor` si `colorterm` es `truecolor` o `24bit`; si no, `Ansi256`;
  - `pub struct Palette`, con un `console::Style` por nombre de la paleta;
  - `pub fn palette(mode: ColorMode) -> Palette`. En `Ansi256`, el índice más cercano de la paleta xterm-256: `fn nearest_256(r: u8, g: u8, b: u8) -> u8`, la distancia euclídea sobre el cubo 6×6×6 y la rampa de grises.
  - `struct ClioTheme` implementa `cliclack::Theme`:
    - la barra en ceniza;
    - el símbolo activo `◆` y la selección en ámbar;
    - los pasos hechos `◇` en verdeSuave;
    - el error `✖` en rojoApagado;
    - los spinners y las barras en hielo, con la parte vacía en borde;
    - los checkbox `◼` (ámbar) y `◻` (ceniza).

    Se activa con `cliclack::set_theme`.
  - Encabezado, con `cliclack::intro`: « Clio » invertido (fondo ámbar, letra grafito `#0D0F12`), luego ` memory` en negrita y ` · la memoria de tu agente · v<versión>` en ceniza.
  - Agentes: un `multiselect`. Cada ítem es el `label`, con hint `encontrado en <at>` en ceniza. Primero va el ítem «Todos los que encuentre»: si se marca, cuenta como todos. Al volver a correr, vienen marcados los que están en `connected`.
  - Modelo: un `select` con hints de tamaño:
    - `Granite 311M · 1,25 GB · recomendado (GPU)` o `(procesador)`;
    - `potion · <tamaño> · liviano`;
    - `ninguno · búsqueda por texto`.

    En Mac Intel, Granite no aparece y se agrega la nota `Granite no tiene runtime para Mac Intel: se recomienda potion`.
  - Cierre con `cliclack::outro`: `Listo. Abre …` y, en ceniza debajo, `clio-memory doctor revisa todo · clio-memory setup cambia lo que elegiste`.

- [ ] **Step 1: Pruebas que fallan** en `palette.rs`:
  - `color_mode_cases`: `NO_COLOR` gana sobre todo; sin terminal → `None`; `truecolor` y `24bit` → `TrueColor`; otro valor → `Ansi256`;
  - `nearest_256_of_palette`: `#FFB13D` → 215; `#7FD6FF` → 117; `#8A8F98` → 246. Calcular los esperados a mano con la fórmula del cubo y la rampa y anotarlos en la prueba; si el cálculo da otro índice, corregir el valor esperado según la fórmula, no la fórmula.
- [ ] **Step 2: Ejecutarlas** → FALLAN.
- [ ] **Step 3: Implementar `palette.rs`, `ClioTheme` y `Tui`.** `cargo deny check` limpio: si `cliclack` trae licencias nuevas, agregarlas a `deny.toml` con su razón, como las que ya hay. Si `cliclack` no deja aplicar algún color exacto, dibujar esa parte a mano con `console`; manda el aspecto de la maqueta (spec, «Lo que decidió Iván»).
- [ ] **Step 4: Ejecutar las pruebas** → PASAN. Verificar por mutación.
- [ ] **Step 5: Prueba manual.** En una terminal real, correr `HOME=$(mktemp -d) PATH=<fakes>:$PATH cargo run --features model2vec,granite -- setup` y recorrer: elegir agentes, el modelo `ninguno`, sí a las instrucciones. Comparar con la maqueta del spec. Repetir con `COLORTERM=` (256 colores) y con `NO_COLOR=1`. Hacer una captura con `script -q /dev/null` o describirlo en el PR.
- [ ] **Step 6: Commit** `feat(setup): the Clio TUI`.

### Task 13: README, versión 0.6.0 y abrir el PR 3

**Files:**
- Modify: `README.md` (la sección «Instalar y usar con Claude Code» pasa a «Instalar»), `Cargo.toml` (`version = "0.6.0"`), `Cargo.lock`, `docs/instalador.md` (banderas)

- [ ] **Step 1: Reescribir la sección del README:**
  - la línea `curl -fsSL https://raw.githubusercontent.com/i-montes/clio-memory/main/install.sh | sh`;
  - qué hace, en 4 viñetas;
  - el modo no interactivo (`sh -s -- --yes --agents codex,gemini --model potion`);
  - la tabla de agentes (enlazando a `docs/instalador.md`);
  - `clio-memory uninstall`;
  - «Compilar tú mismo» con el `cargo install` de hoy, sin `--tag` fijo;
  - un enlace a `docs/releases.md` en «Versiones y Clio».

  Verificar cada afirmación contra el código (m3).
- [ ] **Step 2: Subir la versión a `0.6.0`** (`cargo update -p clio-memory --offline` para el lock). `cargo test --all-features` → pasa.
- [ ] **Step 3: Commit** `docs: install with one line; v0.6.0`. Abrir el PR 3.
- [ ] **Step 4: Después de que Iván fusione los tres PR y lo pida, etiquetar `v0.6.0`.** Mirar el workflow de release hasta el final. Correr en esta máquina la línea real de `curl` y `clio-memory doctor`. Si algo falla, se arregla en `v0.6.1`, porque el release es inmutable.
