# Estable 3 de 5 — v0.11.0: la memoria por skill y CLI — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Un agente usa su memoria corriendo `clio-memory m <verbo>` desde su shell, guiado por la skill, sin que exista un proceso MCP por sesión; `setup` conecta cada agente por skill o por MCP delgado, y recuerda cuál.

**Architecture:** Los verbos de `clio-memory m` son otra cara de las mismas herramientas: cada uno arma el nombre y los argumentos de una herramienta `memory_*` y los manda al demonio (plan 2) o, si no hay, los atiende en el propio proceso. Quién llama se deduce del entorno y de los procesos antecesores. Una sola skill sirve a los dos modos. El modo de cada agente vive en `~/.config/clio-memory/agents.json`.

**Tech Stack:** Rust 2024 (1.95.0), `serde_json`, el demonio de v0.10 (`clio_memory::daemon`), `/proc` en Linux y `ps` en macOS para los antecesores.

**Spec:** `docs/superpowers/specs/2026-11-15-estable-design.md` (sección 5 del diseño). Léelo antes de cada tarea: manda sobre este plan si chocan.

## Global Constraints

- Requiere v0.10.0 fusionada (plan 2). Una rama, `feat/skill-cli`, desde `main`; un commit por tarea; un PR al final. Iván fusiona y etiqueta.
- TDD con verificación por mutación anotada en el commit. Antes de cada commit, las cuatro órdenes de comprobación de los planes anteriores.
- `unsafe_code = "forbid"`, `unwrap_used = "deny"`. Ninguna dependencia nueva.
- Pruebas sin red y sin el `$HOME` real; agentes falsos de `tests/common/fakes.rs`.
- El actor de un agente sigue siendo `mcp:<agente>` (por `mcp::actor_of`), llegue por MCP o por la CLI: el historial y las cuotas no se parten.
- Nombres de agente, exactos: `claude-code`, `opencode`, `codex`, `gemini`, `antigravity` (`AgentId::key`), más `clio` y `cli` para quien no es ninguno.
- Modo por defecto de cada agente:

  | Agente | Modo |
  |---|---|
  | Claude Code | skill |
  | OpenCode | skill |
  | Codex | MCP |
  | Gemini CLI | MCP |
  | Antigravity | MCP |

- La línea de instrucciones nueva, igual para todos: `- Mi memoria principal es clio-memory: úsala según la skill clio-memory.` La anterior (`… clio-memory (MCP): …`) se reconoce como nuestra y se reemplaza.
- La regla de permiso de Claude Code, exacta: `Bash(clio-memory m:*)`, en `permissions.allow` de `~/.claude/settings.json`.
- Códigos de salida de `clio-memory m`: 0 si la herramienta respondió, 1 si dijo que no (su texto va igual a la salida estándar), 2 si la orden está mal escrita.
- Textos para la persona en español con tildes; código y comentarios en inglés.
- Versión objetivo `0.11.0`.

## Review Focus

1. **Texto con comillas, `$`, acentos graves o saltos de línea.** Entra por la entrada estándar (`-`) y se guarda tal cual; la skill enseña el heredoc con comillas simples. Prueba en la Tarea 2.
2. **Un agente desconocido o una shell suelta.** Sin bandera, sin variables y sin antecesor conocido, el llamador es `cli` y la orden funciona. Prueba en la Tarea 1.
3. **Cambiar de modo.** Pasar un agente de MCP a skill quita su servidor MCP y no deja la skill duplicada; volver lo repone. Nada ajeno llamado `clio-memory` se toca sin `--replace`. Prueba en la Tarea 4.
4. **`~/.claude/settings.json` con más cosas** (otras reglas, `deny`, comentarios no: es JSON). Solo se agrega o se quita nuestra regla; el resto queda igual y en el mismo orden. Prueba en la Tarea 4.
5. **`clio-memory` no está en el `PATH` del agente.** En modo skill, `doctor` falla y dice cómo arreglarlo. Prueba en la Tarea 5.

---

### Task 1: Quién llama

**Files:**
- Create: `src/agent/caller.rs`
- Modify: `src/agent/mod.rs`
- Test: `src/agent/caller.rs` (unitarias)

**Interfaces:**
- Produces:
  - `pub struct Caller { pub agent: String, pub session: String }`
  - `pub fn caller_from(flag: Option<&str>, env: &dyn Fn(&str) -> Option<String>, ancestors: &[(u32, String)], parent: u32) -> Caller`
  - `pub fn ancestors() -> Vec<(u32, String)>` — del padre hacia arriba, hasta 8 niveles: (pid, nombre del programa)
  - `pub fn caller(flag: Option<&str>) -> Caller` — con el entorno y los antecesores reales
  - `fn parse_stat(stat: &str) -> Option<(u32, String)>` — de una línea de `/proc/<pid>/stat`: (pid del padre, nombre entre el primer `(` y el último `)`)

- [ ] **Step 1: Escribir las pruebas que fallan**

```rust
fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
    move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| (*v).to_owned())
}

#[test]
fn the_flag_wins_then_the_variable_then_the_environment_then_an_ancestor() {
    let anc = [(700, "bash".to_owned()), (650, "codex".to_owned())];
    let c = caller_from(Some("gemini"), &env_of(&[("CLIO_MEMORY_AGENT", "opencode")]), &anc, 700);
    assert_eq!(c.agent, "gemini");
    let c = caller_from(None, &env_of(&[("CLIO_MEMORY_AGENT", "opencode"), ("CLAUDECODE", "1")]), &anc, 700);
    assert_eq!(c.agent, "opencode");
    let c = caller_from(None, &env_of(&[("CLAUDECODE", "1"), ("CLAUDE_CODE_SESSION_ID", "abc-123")]), &anc, 700);
    assert_eq!((c.agent.as_str(), c.session.as_str()), ("claude-code", "abc-123"));
    let c = caller_from(None, &env_of(&[]), &anc, 700);
    assert_eq!((c.agent.as_str(), c.session.as_str()), ("codex", "650"));
}

#[test]
fn nobody_known_is_the_command_line() {
    let c = caller_from(None, &env_of(&[]), &[(700, "bash".into()), (1, "systemd".into())], 700);
    assert_eq!((c.agent.as_str(), c.session.as_str()), ("cli", "700"));
}

#[test]
fn a_program_name_with_spaces_and_parentheses_is_read_whole() {
    assert_eq!(parse_stat("4242 (my (odd) prog) S 41 4242 4242 0 -1"), Some((41, "my (odd) prog".into())));
    assert_eq!(parse_stat("basura"), None);
}
```

Tabla de nombres de programa a agente, exacta: `claude` → `claude-code`, `codex` → `codex`, `opencode` → `opencode`, `gemini` → `gemini`, `agy` y `antigravity` → `antigravity`, `clio` → `clio`.

- [ ] **Step 2: Correr y ver que fallan**

Run: `cargo test --locked --all-features agent::caller`
Expected: FAIL (no existe el módulo).

- [ ] **Step 3: Implementar**

Sesión: con `claude-code` y `CLAUDE_CODE_SESSION_ID`, ese valor; si el agente salió de un antecesor, el pid de ese antecesor; si no, `parent`. `ancestors()` en Linux lee `/proc/<pid>/stat` empezando por `std::os::unix::process::parent_id()`; en macOS corre `ps -o ppid=,comm= -p <pid>` por nivel y toma el último componente de la ruta; en cualquier fallo, corta ahí.

- [ ] **Step 4: Correr y ver que pasan**

Run: `cargo test --locked --all-features agent::caller`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/agent
git commit -m "feat(agent): who is calling, from a flag, the environment or an ancestor process"
```

---

### Task 2: Los verbos `clio-memory m`

**Files:**
- Create: `src/bin/clio-memory/agent_cli.rs`
- Modify: `src/bin/clio-memory/args.rs` (uso y `Command::Agent`), `src/bin/clio-memory/main.rs`
- Test: `src/bin/clio-memory/agent_cli.rs` (unitarias del análisis), `tests/cli_agent.rs` (nuevo)

**Interfaces:**
- Consumes: `agent::caller::caller` (Tarea 1), `daemon::{request, Launch}` y `tools::Session` (plan 2).
- Produces:
  - `Command::Agent { agent: Option<String>, tool: &'static str, args: serde_json::Value }`
  - `pub fn agent_cli::parse(rest: Vec<String>, stdin: &mut dyn std::io::Read) -> Result<(Option<String>, &'static str, serde_json::Value), UsageError>`
  - `pub fn agent_cli::run(db: &Path, agent: Option<&str>, tool: &str, args: &serde_json::Value) -> ExitCode`
- Gramática exacta (`ID` es `12` o `m12`; `TEXTO` son las palabras que quedan unidas por un espacio, o `-` para leerlo de la entrada estándar sin su salto final):

  | Orden | Herramienta y argumentos |
  |---|---|
  | `m [--agent N] context` | `memory_context` `{}` |
  | `m search TEXTO [--limit N]` | `memory_search` `{query, limit?}` |
  | `m get ID` | `memory_get` `{id}` |
  | `m timeline ID [--before N] [--after N]` | `memory_timeline` `{id, before?, after?}` |
  | `m save [--kind semantic\|procedural] [--title T] [--category C] TEXTO` | `memory_save` `{text, kind, title?, category?}`; `kind` por defecto `semantic` |
  | `m update ID [--title T] [--category C] TEXTO` | `memory_update` `{id, text, title?, category?}` |
  | `m consolidate IDS [--title T] [--category C] TEXTO` | `memory_consolidate` `{ids, text, title?, category?}`; `IDS` como `12,15` |
  | `m summary --goal G --accomplished A [--instructions I] [--discoveries D] [--next-steps N] [--files F]` | `memory_session_summary` con esas claves (`next_steps`) |
  | `m forget ID` / `m pin ID` | `memory_forget` / `memory_pin` `{id}` |
  | `m revert CAMBIO` | `memory_revert` `{change}` |
  | `m project` | `memory_current_project` `{}` |

- [ ] **Step 1: Escribir las pruebas que fallan**

Unitarias de `parse` (una por fila de la tabla, comparando el JSON), más: `m save` sin texto → error de uso `expected TEXT`; `m baila` → `unknown memory verb "baila"`; `m get doce` → error de uso.

`tests/cli_agent.rs` (binario real, datos y `HOME` temporales, `CLIO_MEMORY_NO_DAEMON=1` salvo donde se dice):

```rust
#[test]
fn text_from_stdin_is_saved_as_typed() {
    let t = Dirs::new();
    let text = "Qué: usar `cargo test -- --ignored` y $HOME sin \"comillas\" raras";
    let out = t.m_stdin(&["save", "--title", "Órdenes de prueba", "-"], text);
    assert!(out.stdout.starts_with("saved"), "{out:?}");
    let got = t.m(&["search", "cargo", "test"]);
    assert_eq!(got.code, 0);
    assert!(got.stdout.contains("cargo test -- --ignored") && got.stdout.contains("$HOME"), "{}", got.stdout);
}
```

| Prueba | Afirma |
|---|---|
| `a_refusal_is_exit_1_with_its_words_on_stdout` | `m get 999` → código 1, stdout «ese recuerdo no existe o no se puede leer» |
| `a_typo_is_exit_2_and_changes_nothing` | `m sav hola` → código 2; no existe `memory.db` |
| `the_agent_signs_as_before` | `m --agent codex save "Qué: x"` → la base tiene el actor `mcp:codex` |
| `the_project_is_the_folder_it_runs_in` | en dos carpetas con `.clio-project` distintos, `m project` nombra cada uno |
| `it_goes_through_the_daemon_when_there_is_one` | sin `CLIO_MEMORY_NO_DAEMON`: tras `m save`, existe `daemon.sock`, y `m search` de otra invocación encuentra lo guardado |
| `two_invocations_of_one_session_share_a_budget` | con el demonio y `CLIO_MEMORY_AGENT=codex`: 20 `m context` con código 0 y el 21.º con código 1 y `refills in` |

- [ ] **Step 2: Correr y ver que fallan**

Run: `cargo test --locked --all-features --test cli_agent`
Expected: FAIL (`m` no existe).

- [ ] **Step 3: Implementar**

`run`: `Caller` de la Tarea 1; petición `call` al demonio con `client = caller.agent`, `session = caller.session`, `cwd` actual; si `request` falla o `CLIO_MEMORY_NO_DAEMON=1`, una `Session::new(datos, proyecto de la carpeta, actor_of(&caller.agent)).with_model(EmbedModel::of_home(home))` de una sola llamada, con `finish()` al terminar. Imprime el texto en stdout y devuelve 0 o 1. En `args.rs`, el texto de uso gana un bloque `memory (what an agent runs; see the skill):` con las doce órdenes.

- [ ] **Step 4: Correr y ver que pasan**

Run: `cargo test --locked --all-features --test cli_agent --test cli`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/bin/clio-memory tests/cli_agent.rs
git commit -m "feat(cli): `clio-memory m`, the memory tools as commands an agent runs"
```

---

### Task 3: Una skill para los dos modos, y la línea de instrucciones

**Files:**
- Modify: `skill/SKILL.md`, `src/setup/skill.rs:98-107` (`KNOWN_SKILLS`), `src/setup/mod.rs:28-33`, `src/setup/instructions.rs:50-51`, `:77`, `:239-243`, `src/mcp.rs:23` (no cambia el texto; solo se comprueba), `docs/instalador.md:35`
- Test: `tests/setup.rs`, `tests/setup_instructions.rs`

**Interfaces:**
- Produces: `pub const OLD_INSTRUCTIONS_LINES: &[&str]` en `src/setup/mod.rs` con la línea anterior; `INSTRUCTIONS_LINE` con el texto nuevo de Global Constraints.

- [ ] **Step 1: Escribir las pruebas que fallan**

`tests/setup.rs`:

```rust
#[test]
fn the_skill_of_v0_7_to_v0_10_is_known_as_ours() {
    assert!(clio_memory::setup::skill::KNOWN_SKILLS
        .contains(&"2e3637e12fd31d53632ed8574e94241bae03ca95d84ca6c016f836da0317df01"));
}

#[test]
fn the_skill_teaches_both_ways() {
    let s = clio_memory::setup::SKILL;
    for needle in ["memory_search", "clio-memory m search", "clio-memory m save", "<<'EOF'", "UNTRUSTED-DATA"] {
        assert!(s.contains(needle), "the skill does not mention {needle}");
    }
    assert!(!s.lines().nth(2).unwrap_or_default().contains("MCP tools"), "the description no longer says MCP only");
}
```

`tests/setup_instructions.rs`: `the_older_line_is_replaced_not_doubled` — un `CLAUDE.md` con la línea anterior: tras `add_instructions` tiene la nueva una sola vez y no la anterior; `has_instructions` es verdadero con cualquiera de las dos; `remove_instructions` quita cualquiera de las dos.

- [ ] **Step 2: Correr y ver que fallan**

Run: `cargo test --locked --all-features --test setup --test setup_instructions`
Expected: FAIL.

- [ ] **Step 3: Escribir la skill y el resto**

`skill/SKILL.md` conserva sus secciones y sus medidas de seguridad, con estos cambios:
- `description` del encabezado: «Your primary long-term memory (clio-memory). Use it in every session…» (sin «the clio-memory MCP tools»).
- Sección nueva, la primera, «How to call it»: si existen las herramientas `memory_*`, úsalas; si no, corre las órdenes `clio-memory m …`. Lleva la tabla de equivalencias de la Tarea 2 (herramienta ↔ orden) y este ejemplo para textos largos o con símbolos:

  ```sh
  clio-memory m save --title "Chose Postgres 16 for orders" --category decision - <<'EOF'
  Qué: … · Por qué: … · Dónde: … · Aprendido: …
  EOF
  ```
- Donde nombra una herramienta suelta (`memory_context`, `memory_update`…), se queda el nombre de la herramienta: la tabla da la orden.
- En «When to read it», «If the memory slept last night» pasa a «If the memory slept since you last looked».
- Una línea en las medidas de seguridad: las órdenes que no empiezan por `clio-memory m` (`purge`, `promote`, `remember`, `uninstall`…) son de la persona: no las corras.

`KNOWN_SKILLS` gana la entrada `// v0.7.0 to v0.10.0` con el sha256 de arriba. `docs/instalador.md:35` lista las cinco versiones conocidas.

- [ ] **Step 4: Correr y ver que pasan**

Run: `cargo test --locked --all-features --test setup --test setup_instructions --test setup_plan --test cli`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add skill/SKILL.md src/setup src/mcp.rs docs/instalador.md tests/setup.rs tests/setup_instructions.rs
git commit -m "feat(skill): one skill for both ways, tools or commands"
```

---

### Task 4: `setup` conecta por skill o por MCP

**Files:**
- Create: `src/setup/modes.rs`, `src/setup/permission.rs`
- Modify: `src/setup/mod.rs` (módulos), `src/setup/plan.rs:17-28` (`Selection`), `:67-93`, `:120-330` (`apply`), `src/bin/clio-memory/args.rs` (bandera `--mode`), `src/bin/clio-memory/setup_flow.rs:27-39`, `:386-391`, `src/bin/clio-memory/tui.rs`
- Test: `tests/setup_modes.rs` (nuevo), `tests/setup_plan.rs`

**Interfaces:**
- Produces:
  - `pub enum Mode { Skill, Mcp }` con `key()` (`"skill"`, `"mcp"`) y `from_key`
  - `pub fn modes::default_mode(agent: AgentId) -> Mode` (tabla de Global Constraints)
  - `pub fn modes::file(home: &Path) -> PathBuf` → `<home>/.config/clio-memory/agents.json`
  - `pub fn modes::load(home: &Path) -> BTreeMap<AgentId, Mode>` (vacío si no existe o no se lee)
  - `pub fn modes::set(home: &Path, agent: AgentId, mode: Option<Mode>) -> Result<(), String>` (`None` borra la entrada; el archivo se escribe entero, 0600)
  - `pub fn permission::allow(home: &Path) -> Result<bool, String>` y `permission::revoke(home: &Path) -> Result<bool, String>` — agregan o quitan `Bash(clio-memory m:*)` de `permissions.allow` en `~/.claude/settings.json` con `config_file::{read_json, write_json}`; devuelven si cambió algo
  - `Selection.mode: Option<Mode>` (`None`: el anotado, y si no hay, el de por defecto)
  - `plan::connected(env)`: los agentes con nuestro MCP **o** con `Mode::Skill` anotado
  - Bandera `setup --mode skill|mcp|auto` (por defecto `auto`)

- [ ] **Step 1: Escribir las pruebas que fallan**

`tests/setup_modes.rs` (con los agentes falsos de `tests/common/fakes.rs`):

| Prueba | Preparación | Afirma |
|---|---|---|
| `each_agent_has_its_default` | — | la tabla de Global Constraints |
| `claude_code_by_skill_has_no_mcp_server` | `apply` con `agents: [ClaudeCode]`, `mode: None` | `mcp_state` es `Absent`; `modes::load` dice `Skill`; la skill está en `~/.claude/skills/clio-memory/SKILL.md`; `settings.json` tiene la regla |
| `codex_by_default_keeps_its_mcp_server` | `apply` con `agents: [Codex]` | `mcp_state` es `Ours`; `modes::load` dice `Mcp` |
| `switching_to_skill_takes_the_server_away` | Codex conectado por MCP; `apply` con `mode: Some(Skill)` | `mcp_state` es `Absent`; el modo anotado es `Skill`; la skill sigue una sola vez donde Codex la lee |
| `switching_back_puts_it_again` | lo anterior; `apply` con `mode: Some(Mcp)` | `mcp_state` es `Ours`; el modo anotado es `Mcp` |
| `a_foreign_server_is_left_alone_without_replace` | un MCP ajeno llamado `clio-memory` en Claude Code; `mode: Some(Skill)`, sin `replace` y con un `ask` que dice no | el paso es `Err`; el servidor ajeno sigue; no se anota modo |
| `the_permission_rule_touches_nothing_else` | `settings.json` con `{"permissions":{"allow":["Bash(ls:*)"],"deny":["Read(.env)"]},"model":"x"}` | tras `allow`: `allow` es `["Bash(ls:*)","Bash(clio-memory m:*)"]` y lo demás idéntico y en el mismo orden; `allow` otra vez devuelve `false`; tras `revoke`: el JSON es igual al original |
| `a_rerun_keeps_each_mode` | Claude Code por skill y Codex por MCP; `apply` con los dos y `mode: None` | nada cambia; los dos pasos dicen que ya estaba |

- [ ] **Step 2: Correr y ver que fallan**

Run: `cargo test --locked --all-features --test setup_modes`
Expected: FAIL.

- [ ] **Step 3: Implementar**

En `apply`, por cada agente elegido, `mode = sel.mode.or(anotado).unwrap_or(default_mode(agent))`:
- `Skill`: si el MCP es `Ours` u `OtherPath`, `mcp_disconnect`; si es `Foreign`, la misma pregunta de hoy (o `replace`); anota el modo; en Claude Code, `permission::allow`. El paso dice `conectado por skill: corre «clio-memory m …» (sin proceso por sesión)`.
- `Mcp`: lo de hoy; anota el modo; en Claude Code, `permission::revoke`.
- La colocación de la skill, la línea de instrucciones y el sueño no cambian, pero «conectado» pasa a ser la definición nueva (`skill_agents` usa `connected`).
- Con `exclusive`, desconectar un agente borra además su modo anotado y, en Claude Code, la regla.

`setup_flow`: `SetupArgs.mode: Option<Mode>` (de `--mode`; `auto` es `None`). La TUI, después de elegir agentes, muestra una confirmación con el reparto que va a aplicar («Claude Code y OpenCode por skill, sin proceso por sesión; Codex por MCP») y tres opciones: «Así», «Todos por skill», «Todos por MCP». Sin terminal no pregunta.

- [ ] **Step 4: Correr y ver que pasan**

Run: `cargo test --locked --all-features --test setup_modes --test setup_plan --test setup --test setup_mcp --test cli`
Expected: PASS (las pruebas existentes de `setup_plan` que esperaban un MCP en Claude Code u OpenCode pasan `mode: Some(Mode::Mcp)` o esperan el modo skill, según lo que prueben).

- [ ] **Step 5: Commit**

```bash
git add src/setup src/bin/clio-memory tests/setup_modes.rs tests/setup_plan.rs
git commit -m "feat(setup): each agent connects by skill or by MCP, and setup remembers which"
```

---

### Task 5: `doctor` y `uninstall` en modo skill

**Files:**
- Modify: `src/setup/plan.rs` (`agent_report`, `uninstall_with`), `src/bin/clio-memory/commands.rs:273-336`
- Test: `tests/setup_plan.rs`, `tests/cli.rs`

- [ ] **Step 1: Escribir las pruebas que fallan**

| Prueba | Afirma |
|---|---|
| `doctor_names_the_way_each_agent_is_connected` | `ok   agente claude-code: conectado (skill)` y `ok   agente codex: conectado (MCP)` |
| `doctor_fails_when_the_skill_way_cannot_find_the_binary` | Claude Code por skill y un `PATH` sin `clio-memory`: `FAIL agente claude-code: por skill, pero «clio-memory» no está en el PATH (agrega <carpeta del binario> al PATH)`; código 1 |
| `doctor_fails_when_the_skill_is_gone` | modo skill anotado y la skill borrada: `FAIL agente claude-code: por skill, pero no ve la skill (clio-memory setup la repone)` |
| `uninstall_takes_the_mode_and_the_rule_away` | tras `uninstall --agents claude-code`: sin entrada en `agents.json`, sin la regla en `settings.json`, sin la skill si nadie más la lee |
| `uninstall_of_everything_removes_the_modes_file` | tras `uninstall`: no existe `~/.config/clio-memory/agents.json` |

- [ ] **Step 2: Correr y ver que fallan**

Run: `cargo test --locked --all-features --test setup_plan --test cli doctor uninstall`
Expected: FAIL en las nuevas.

- [ ] **Step 3: Implementar** según la tabla. `doctor` mira el `PATH` con `Env::program("clio-memory")` y, si lo que encuentra no es este binario, usa la nota de `agents::shadowing_note`.

- [ ] **Step 4: Correr y ver que pasan**

Run: `cargo test --locked --all-features --no-fail-fast`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/setup/plan.rs src/bin/clio-memory/commands.rs tests/setup_plan.rs tests/cli.rs
git commit -m "feat(doctor): the skill way is checked and uninstalled like the MCP way"
```

---

### Task 6: Las órdenes de la persona no corren desde un agente

Con la skill, un agente aprende a correr `clio-memory`. Las órdenes que cambian la memoria con la autoridad de la persona no deben salirle por accidente.

**Files:**
- Modify: `src/bin/clio-memory/write.rs:29` (`run`), `src/bin/clio-memory/main.rs:226` (`uninstall` con `--data`), `src/bin/clio-memory/args.rs` (texto de uso)
- Test: `tests/cli.rs`

**Interfaces:**
- Consumes: `agent::caller::caller` (Tarea 1).
- Produces: `fn write::persons_only(command: &Command, actor: Option<&str>, caller: &Caller, as_person: bool) -> Result<(), String>` — `Err` con el texto exacto `esta orden es de la persona: córrela tú en una terminal (o pon CLIO_MEMORY_AS_PERSON=1 si eres tú quien la corre desde un agente)` cuando el llamador es un agente conocido (cualquiera menos `cli`), `as_person` es falso y la orden es una de: `purge`, `promote`, `archive`, `revert`, `feedback`, `import`, `import-memories` con origen `user`, `remember` sin `--actor`, `uninstall --data`.

- [ ] **Step 1: Escribir las pruebas que fallan** (`tests/cli.rs`; el agente se finge con `CLIO_MEMORY_AGENT=codex`)

| Prueba | Afirma |
|---|---|
| `an_agent_cannot_purge` | `purge 1 --yes` con `CLIO_MEMORY_AGENT=codex` → código 1, el texto de arriba, y el recuerdo sigue |
| `an_agent_cannot_write_as_the_person` | `remember clio "soy la persona"` → código 1; con `--actor mcp:codex` → código 0 |
| `the_person_can_say_it_is_them` | lo mismo con `CLIO_MEMORY_AS_PERSON=1` → código 0 |
| `reading_commands_are_not_stopped` | `list`, `get`, `stats`, `doctor`, `export` con `CLIO_MEMORY_AGENT=codex` → como siempre |
| `from_a_plain_shell_nothing_changes` | sin variables y sin antecesor conocido (las pruebas existentes de `purge` y `promote` siguen pasando) |

Si las pruebas corren dentro de un agente (un antecesor `claude` o `codex`), las existentes que purgan o promueven fallarían: el ayudante que lanza el binario en `tests/cli.rs` pone `CLIO_MEMORY_AGENT=cli` por defecto, y estas pruebas lo cambian.

- [ ] **Step 2: Correr y ver que fallan**

Run: `cargo test --locked --all-features --test cli agent_cannot the_person_can`
Expected: FAIL.

- [ ] **Step 3: Implementar** `persons_only` y llamarla al entrar en `write::run` y en `uninstall` cuando lleva `--data`. `as_person` es `CLIO_MEMORY_AS_PERSON == "1"`. El texto de uso lo dice en una línea bajo `change`.

- [ ] **Step 4: Correr y ver que pasan**

Run: `cargo test --locked --all-features --test cli --test transfer --test purge`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/bin/clio-memory tests/cli.rs
git commit -m "fix(cli): the person's commands do not run from an agent by accident"
```

---

### Task 7: Versión 0.11.0, documentación y la comprobación en cada agente

**Files:**
- Modify: `Cargo.toml:5`, `Cargo.lock`, `README.md`, `docs/instalador.md`

- [ ] **Step 1: Documentar**

`README.md`: la memoria se usa por herramientas MCP o por `clio-memory m`; tabla de órdenes; cómo cambiar un agente de modo (`clio-memory setup --agents codex --mode skill`). `docs/instalador.md`: lo que `setup` toca en cada agente según su modo (incluida la regla de Claude Code) y esta lista, que Iván llena a mano después de instalar, una fila por agente:

| Agente | En una sesión nueva: «¿qué recuerdas de este proyecto?» corre `clio-memory m context` sin pedir permiso | «guarda que …» corre `clio-memory m save` y responde `saved` | Pasa a skill por defecto |
|---|---|---|---|
| Claude Code | | | ya lo es |
| OpenCode | | | ya lo es |
| Codex | | | |
| Gemini CLI | | | |
| Antigravity | | | |

La nota bajo la tabla: un agente cuya shell corre en sandbox (Codex por defecto) puede no alcanzar el socket ni la carpeta de datos; si es así, se queda por MCP.

- [ ] **Step 2: Subir la versión y comprobar como el CI**

`version = "0.11.0"`. Run: las cuatro órdenes de comprobación. Expected: todo en verde.

- [ ] **Step 3: Commit y PR**

```bash
git add Cargo.toml Cargo.lock README.md docs/instalador.md
git commit -m "chore: release v0.11.0"
gh pr create --title "v0.11.0: la memoria por skill y CLI" --body "…"
```

El cuerpo del PR pide a Iván llenar la tabla en Codex, Gemini CLI y Antigravity antes de cambiar sus modos por defecto, y recuerda que el modo por defecto de un agente se cambia en `modes::default_mode` con su prueba.
