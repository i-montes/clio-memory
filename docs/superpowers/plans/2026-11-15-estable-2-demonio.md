# Estable 2 de 5 — v0.10.0: un solo proceso — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Con cualquier número de sesiones de agente abiertas hay un solo proceso con el modelo de vectores y un solo escritor; cada `clio-memory mcp` queda en unos MB, sin modelo ni base, y ningún guardado se rechaza por el candado.

**Architecture:** Un demonio por carpeta de datos (`clio-memory daemon`) atiende una petición por conexión en un socket Unix. Nadie lo instala: el primer cliente que no lo encuentra lo lanza, y sale solo tras 10 minutos sin peticiones. `clio-memory mcp` contesta el protocolo MCP él mismo y manda cada llamada de herramienta al demonio; si no puede, atiende dentro de su proceso como hoy. El demonio abre la base para cada llamada y la suelta, así que Clio, la CLI y el sueño siguen escribiendo como siempre.

**Tech Stack:** Rust 2024 (1.95.0), `std::os::unix::net` (sin dependencias nuevas), `serde_json`, `flock` por `std::fs::File::try_lock`, POSIX `sh` para lanzar el demonio.

**Spec:** `docs/superpowers/specs/2026-11-15-estable-design.md` (sección 4 del diseño). Léelo antes de cada tarea: manda sobre este plan si chocan.

## Global Constraints

- Requiere v0.9.1 fusionada (plan 1). Una rama, `feat/daemon`, desde `main`; un commit por tarea; un PR al final. Iván fusiona y etiqueta.
- TDD con verificación por mutación anotada en el commit (`mutation-checked: …`), como en el plan 1.
- Antes de cada commit: `cargo fmt --all --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, `cargo test --locked --all-features --no-fail-fast`, `cargo clippy --locked --lib --no-default-features --features model2vec,granite,telemetry -- -D warnings`.
- `unsafe_code = "forbid"`, `unwrap_used = "deny"`. Ninguna dependencia nueva. El esquema sigue en 2.
- Pruebas sin red y sin el `$HOME` real. Las que lanzan el demonio le pasan `HOME` y la carpeta de datos temporales por `Launch.env`, nunca por el entorno del proceso de pruebas.
- Valores exactos:

  | Qué | Valor |
  |---|---|
  | Socket | `$CLIO_MEMORY_SOCK` si está (ruta absoluta); si no, `<datos>/daemon.sock` |
  | Largo máximo de la ruta del socket | 100 bytes |
  | Candado del demonio | `<datos>/daemon.lock` (`flock` exclusivo) |
  | Registro | `<datos>/daemon.log` (0600; se vacía al arrancar si pasa de 1 MiB) |
  | Inactividad para salir | 600 s; `CLIO_MEMORY_DAEMON_IDLE` (segundos) lo cambia |
  | Espera del cliente al lanzarlo | hasta 3 s, reintentando cada 25 ms |
  | Versión del protocolo | `1` |
  | Línea más larga | 1 MiB |
  | Ventana de los presupuestos | 1800 s |
  | Sin demonio | `CLIO_MEMORY_NO_DAEMON=1` |

- El demonio nunca deja la base abierta entre peticiones.
- Textos para la persona en español con tildes; código y comentarios en inglés.
- Versión objetivo `0.10.0`.

## Review Focus

1. **El demonio muere a mitad de una petición** (`kill -9`). El cliente reintenta una vez lanzándolo de nuevo y, si falla, atiende en su proceso; el agente nunca se queda sin respuesta. Prueba en la Tarea 5.
2. **Dos clientes lo lanzan a la vez.** Queda exactamente uno y los dos reciben respuesta. Prueba en la Tarea 4.
3. **Una ruta de socket de más de 100 bytes** (carpeta de datos larga). Error claro, `doctor` lo dice y el cliente atiende en su proceso. Pruebas en las Tareas 4 y 6.
4. **Versiones mezcladas tras un `update`.** Un cliente más viejo es atendido; uno más nuevo reemplaza al demonio sin cortar la petición que otro tenía en curso. Prueba en la Tarea 3.
5. **Una línea que no es JSON o pasa de 1 MiB.** Respuesta de error y el demonio sigue atendiendo. Prueba en la Tarea 3.

---

### Task 1: Guardar no retiene el candado mientras vectoriza

**Files:**
- Modify: `src/agent/memory.rs` (función nueva junto a `embed_queue`, `:956-973`), `src/tools.rs:299-307` (`finish`), `:432-514` (`save`, `consolidate`), `:677-687` (`with_vectors`)
- Test: `tests/tools.rs`

**Interfaces:**
- Produces: `pub fn clio_memory::agent::memory::embed_pending(data: &Path, model: &EmbedModel) -> Res<usize>` — calcula con una lectura (`Engine::compute_vectors`, de 64 en 64) y guarda en una sección corta de escritura (`Engine::store_vectors`); repite hasta que no quede nada; al final una llamada a `Memory::embed_queue` cierra los trabajos en cola (ya no encuentran nada que vectorizar). Devuelve cuántos vectores guardó.

- [ ] **Step 1: Escribir la prueba que falla**

```rust
/// Inside `embed`, tries to take the writer: it can only if the session let it go.
struct ProbeEmbedder {
    inner: HashEmbedder,
    db: PathBuf,
    free: Arc<AtomicBool>,
}
impl Embedder for ProbeEmbedder {
    fn model_id(&self) -> &str { self.inner.model_id() }
    fn dim(&self) -> usize { self.inner.dim() }
    fn embed(&self, texts: &[&str]) -> clio_memory::Result<Vec<Vec<f32>>> {
        if Engine::open(&self.db, OpenMode::ReadWrite, Arc::new(SystemClock)).is_ok() {
            self.free.store(true, Ordering::SeqCst);
        }
        self.inner.embed(texts)
    }
}

#[test]
fn a_save_does_not_hold_the_writer_while_it_embeds() {
    let t = tempfile::tempdir().unwrap();
    let free = Arc::new(AtomicBool::new(false));
    let model = EmbedModel::from_embedder(Arc::new(ProbeEmbedder {
        inner: HashEmbedder::new("hash", 16),
        db: t.path().join("memory.db"),
        free: free.clone(),
    }));
    let mut s = Session::new(t.path().to_path_buf(), None, "mcp:prueba".into()).with_model(model);
    let said = s.call("memory_save", &json!({"text": "Qué: una prueba", "kind": "semantic"})).unwrap();
    assert!(said.starts_with("saved"), "{said}");
    assert!(free.load(Ordering::SeqCst), "the writer was held while the model ran");
    let m = Memory::open_read(t.path(), &EmbedModel::none(), None, "mcp:prueba").unwrap().unwrap();
    assert_eq!(m.engine().stats().unwrap().vectors, 1, "the vector is stored all the same");
}
```

- [ ] **Step 2: Correr y ver que falla**

Run: `cargo test --locked --all-features --test tools a_save_does_not_hold`
Expected: FAIL con «the writer was held while the model ran».

- [ ] **Step 3: Implementar**

`Session::save` y `Session::consolidate`: el `Memory` de escritura vive solo lo que dura el guardado (y, en `consolidate`, los olvidos); se suelta antes de vectorizar. Después: `embed_pending(&self.data, &self.model)` (un error va a stderr, como hoy) y, para el aviso de «muy parecida», un `Memory::open_read` nuevo. `with_vectors` pasa a ser un método que recibe solo la respuesta y decide con `self.model.is_present()` si agrega «sin modelo de vectores…». `Session::finish` llama a `embed_pending`.

- [ ] **Step 4: Correr y ver que pasa**

Run: `cargo test --locked --all-features --test tools --test relations_tools --test mcp --test vectors`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/agent/memory.rs src/tools.rs tests/tools.rs
git commit -m "fix(tools): a save lets the writer go before it computes vectors"
```

---

### Task 2: Los presupuestos de una sesión se reponen

**Files:**
- Modify: `src/tools.rs:1-16` (doc), `:104-109`, `:208-239` (`Session`), `:309-342`
- Test: `tests/tools.rs`

**Interfaces:**
- Produces: `pub fn Session::with_clock(self, clock: Arc<dyn Clock>) -> Self` (por defecto `SystemClock`); `const WINDOW_SECS: i64 = 1800`. Los topes no cambian: 20 lecturas o 24 KiB, 30 guardados, 10 cambios, ahora por ventana.

- [ ] **Step 1: Escribir las pruebas que fallan**

```rust
#[test]
fn the_read_budget_refills_after_half_an_hour() {
    let t = tempfile::tempdir().unwrap();
    let clock = Arc::new(FixedClock::new(10_000));
    let mut s = Session::new(t.path().to_path_buf(), None, "mcp:prueba".into()).with_clock(clock.clone());
    for _ in 0..20 {
        s.call("memory_context", &json!({})).unwrap();
    }
    let why = s.call("memory_context", &json!({})).unwrap_err();
    assert!(why.contains("refills in 30 minutes"), "{why}");
    clock.set(10_000 + 1799);
    assert!(s.call("memory_context", &json!({})).is_err());
    clock.set(10_000 + 1800);
    assert!(s.call("memory_context", &json!({})).is_ok());
}
```

Y `the_save_budget_refills_too`: 30 guardados de textos distintos, el 31.º falla con `refills in`, y con el reloj en +1800 se guarda.

- [ ] **Step 2: Correr y ver que fallan**

Run: `cargo test --locked --all-features --test tools refills`
Expected: FAIL (no existe `with_clock`).

- [ ] **Step 3: Implementar**

`Session` gana `clock: Arc<dyn Clock>` y `window: i64` (cuándo empezó la ventana; 0 al nacer). Al entrar en `call`: si `now - window >= WINDOW_SECS`, los cuatro contadores vuelven a 0 y `window = now`. Los tres mensajes de tope terminan con `; it refills in N minutes`, con N = `(window + WINDOW_SECS - now + 59) / 60`. `now_secs()` deja de usarse dentro de `Session` (usa el reloj).

- [ ] **Step 4: Correr y ver que pasan**

Run: `cargo test --locked --all-features --test tools --test mcp`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/tools.rs tests/tools.rs
git commit -m "fix(tools): a session's budgets refill every half hour"
```

---

### Task 3: El protocolo y el núcleo del demonio (sin procesos)

**Files:**
- Create: `src/daemon/mod.rs`, `src/daemon/proto.rs`
- Modify: `src/lib.rs` (`pub mod daemon;`)
- Test: `tests/daemon_proto.rs`

**Interfaces:**
- Consumes: `Session` (`with_model`, `with_clock`, `call`), `mcp::actor_of`, `agent::project::settle`, `EmbedModel`.
- Produces:
  - `pub const PROTOCOL: u32 = 1;` y `pub const MAX_LINE: usize = 1 << 20;` en `proto`
  - `pub fn proto::newer(a: &str, b: &str) -> bool` — `a` es una versión `X.Y.Z` mayor que `b`; si alguna no se entiende, `false`
  - `pub struct Daemon`; `pub fn Daemon::new(data: PathBuf, home: PathBuf, clock: Arc<dyn Clock>, model: Arc<EmbedModel>) -> Self`
  - `pub enum After { Continue, Exit }`
  - `pub fn Daemon::handle_line(&self, line: &str) -> (String, After)` — la respuesta, en una línea sin salto
  - `pub fn Daemon::idle_secs(&self) -> i64` — segundos desde la última petición (0 con una en curso)
- Formas exactas (una línea JSON cada una):

  ```json
  {"v":1,"bin":"0.10.0","op":"call","client":"claude-code","session":"41385-1794751150","cwd":"/abs/dir","tool":"memory_search","args":{"query":"x"}}
  {"ok":true,"text":"…"}                         // la herramienta respondió
  {"ok":false,"text":"…"}                        // la herramienta dijo que no (isError)
  {"v":1,"bin":"0.10.0","op":"embed","texts":["a","b"]}
  {"ok":true,"model":"…","dim":384,"vectors":[[…],[…]]}
  {"v":1,"bin":"0.10.0","op":"model"}            // {"ok":true,"model":"…","dim":384} o {"ok":false,"error":"no-model","text":"…"}
  {"v":1,"bin":"0.10.0","op":"status"}           // {"ok":true,"bin":"0.10.0","pid":123,"loaded":false,"sessions":2,"idle_secs":4}
  {"v":1,"bin":"0.10.0","op":"shutdown"}         // {"ok":true}
  {"ok":false,"error":"stale|protocol|bad-request","text":"…"}
  ```

- [ ] **Step 1: Escribir las pruebas que fallan** (`tests/daemon_proto.rs`, con `HashEmbedder` por `EmbedModel::from_embedder` y `FixedClock`)

| Prueba | Petición | Afirma |
|---|---|---|
| `a_call_answers_with_the_tools_text` | `call` de `memory_save`, luego `call` de `memory_search` del mismo texto | `ok: true`; el segundo `text` contiene el marco `UNTRUSTED-DATA` y el texto guardado |
| `a_tool_refusal_is_ok_false_with_its_words` | `call` de `memory_get` con `id: 999` | `ok: false`, `text` = «ese recuerdo no existe o no se puede leer», sin `error` |
| `each_session_has_its_own_budget` | 20 `memory_context` de la sesión `a`, uno más, y uno de la sesión `b` | el 21.º de `a` es `ok: false`; el de `b` es `ok: true` |
| `the_project_comes_from_the_folder_sent` | `memory_current_project` con dos `cwd` (dos carpetas temporales con `.clio-project` distintos) | cada respuesta nombra su proyecto |
| `the_actor_is_the_clients` | `call` de `memory_save` con `client: "Claude Code"` | la base tiene el actor `mcp:claude-code` |
| `a_newer_client_makes_it_leave` | `status` con `bin: "99.0.0"` | `error: "stale"` y `After::Exit` |
| `an_older_client_is_served` | `status` con `bin: "0.0.1"` | `ok: true`, `After::Continue` |
| `a_newer_protocol_is_refused` | `v: 2` | `error: "protocol"`, `After::Continue` |
| `garbage_gets_an_answer_and_it_goes_on` | `no es json`; una línea de `MAX_LINE + 1` bytes; `{"v":1,"bin":"0.10.0","op":"baila"}` | las tres: `error: "bad-request"`, `After::Continue` |
| `embed_returns_one_vector_per_text` | `embed` de dos textos | `dim: 16`, dos vectores de 16 |
| `shutdown_says_ok_and_leaves` | `shutdown` | `ok: true`, `After::Exit` |

Además, unitaria en `proto`: `newer("0.10.1","0.10.0")`, `newer("1.0.0","0.99.9")`, `!newer("0.10.0","0.10.0")`, `!newer("x","0.1.0")`, `!newer("0.1.0","x")`.

- [ ] **Step 2: Correr y ver que fallan**

Run: `cargo test --locked --all-features --test daemon_proto`
Expected: FAIL (no existe `clio_memory::daemon`).

- [ ] **Step 3: Implementar**

`Daemon` guarda `Mutex<HashMap<(String, String), Session>>` (clave: cliente tal como llegó y sesión) y la hora de la última petición. `handle_line` toma el candado de las sesiones durante toda la llamada: las herramientas se atienden de una en una. Una sesión nueva se crea con `Session::new(data, proyecto, actor_of(client)).with_model(model.clone()).with_clock(clock.clone())`, donde el proyecto sale de `project::settle(&data, &cwd)` con la misma regla de `mcp::serve` para la carpeta personal (`src/mcp.rs:35-45`). Las sesiones sin uso en 24 horas se borran al atender. `stale` se decide con `proto::newer(&req.bin, env!("CARGO_PKG_VERSION"))`, antes de mirar `op`.

- [ ] **Step 4: Correr y ver que pasan**

Run: `cargo test --locked --all-features --test daemon_proto && cargo test --locked --all-features daemon::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/daemon src/lib.rs tests/daemon_proto.rs
git commit -m "feat(daemon): the protocol and the request handler, one session per client"
```

---

### Task 4: El proceso: socket, candado, arranque bajo demanda y salida por inactividad

**Files:**
- Create: `src/daemon/server.rs`, `src/daemon/client.rs`
- Modify: `src/platform/unix.rs` (una función), `src/bin/clio-memory/args.rs` (orden `daemon`), `src/bin/clio-memory/main.rs`
- Test: `tests/daemon.rs`, `tests/common/mod.rs` (un ayudante)

**Interfaces:**
- Consumes: `Daemon`, `After`, `proto::{PROTOCOL, MAX_LINE}` (Tarea 3).
- Produces:
  - `pub fn platform::try_lock(path: &Path) -> Result<Option<File>>` — `flock` exclusivo sin espera sobre un archivo 0600 que crea si falta; `None` si otro lo tiene
  - `pub fn daemon::socket_path(data: &Path, sock_env: Option<&OsStr>) -> Result<PathBuf, String>` — error `la ruta del socket es demasiado larga (N bytes): pon CLIO_MEMORY_SOCK` si pasa de 100
  - `pub fn daemon::serve(data: &Path, home: &Path, idle: Duration) -> Result<(), String>` — devuelve `Ok(())` al salir por inactividad, por `shutdown`, por `stale`, o si otro demonio ya tiene el candado
  - `pub struct daemon::Launch { pub exe: PathBuf, pub env: Vec<(String, String)> }`
  - `pub fn daemon::request(data: &Path, launch: Option<&Launch>, body: &serde_json::Value) -> Result<serde_json::Value, String>` — agrega `v` y `bin`; conecta; si no hay nadie y hay `launch`, lanza el demonio y reintenta hasta 3 s; ante `stale`, espera a que el socket desaparezca (hasta 2 s), lanza y reintenta una vez
  - Orden de la CLI: `clio-memory daemon [--status|--stop]` (`Command::Daemon { status: bool, stop: bool }`); sin banderas corre en primer plano
  - Ayudante de pruebas `common::short_dir() -> TempDir`: `tempfile::tempdir()`, y si su ruta pasa de 60 bytes, `tempfile::Builder::new().prefix("cm").tempdir_in("/tmp")`

- [ ] **Step 1: Escribir las pruebas que fallan** (`tests/daemon.rs`; todas lanzan el binario real con `Launch { exe: env!("CARGO_BIN_EXE_clio-memory").into(), env: vec![("HOME".into(), …), ("CLIO_MEMORY_DAEMON_IDLE".into(), …)] }` sobre `short_dir()`; al terminar cada una manda `shutdown`)

| Prueba | Afirma |
|---|---|
| `a_client_starts_the_daemon_and_the_next_one_finds_it` | dos `status` seguidos devuelven el mismo `pid`; existe `<datos>/daemon.sock` |
| `two_clients_starting_it_at_once_get_one_daemon` | 8 hilos piden `status` a la vez sobre una carpeta sin demonio: todos `ok: true` y un solo `pid` distinto |
| `it_leaves_when_idle_and_comes_back_when_asked` | con `CLIO_MEMORY_DAEMON_IDLE=1`: `pid` A; tras 2,5 s el socket no existe; otro `status` da un `pid` B ≠ A |
| `a_second_daemon_for_the_same_folder_steps_aside` | con uno corriendo, `clio-memory --db <datos>/memory.db daemon` termina con código 0 en menos de 2 s y `status` sigue dando el primer `pid` |
| `a_dead_socket_file_is_replaced` | un archivo normal en `<datos>/daemon.sock`: `request` responde `ok: true` |
| `a_socket_path_that_is_too_long_is_said` | `socket_path` con una carpeta de 120 bytes → `Err` que contiene `demasiado larga`; con `CLIO_MEMORY_SOCK` corta → `Ok` |
| `stop_and_status_from_the_command_line` | `daemon --status` imprime `activo` y el pid; `daemon --stop` lo detiene; `daemon --status` imprime `inactivo` |

- [ ] **Step 2: Correr y ver que fallan**

Run: `cargo test --locked --all-features --test daemon`
Expected: FAIL (no existen `request`, `Launch`, ni la orden `daemon`).

- [ ] **Step 3: Implementar**

`serve`, en este orden: `try_lock(<datos>/daemon.lock)` (si es `None`, `Ok(())`); borrar lo que haya en la ruta del socket; `UnixListener::bind`; permisos 0600 al socket; un hilo vigía que duerme de a 1 s y, cuando `daemon.idle_secs() >= idle` o se pidió salir, marca la salida y se conecta al socket para despertar al `accept`; el hilo principal acepta en bucle y atiende cada conexión en un hilo: lee una línea (como mucho `MAX_LINE + 1` bytes), `handle_line`, escribe la respuesta y un salto, cierra. Con `After::Exit` deja de aceptar, espera a los hilos en curso y sale. Al salir: `embed_pending(data, model)` una vez (lo que haya quedado sin vector, de quien sea), borrar el socket y soltar el candado.

Lanzamiento (`client.rs`). Se da el cuerpo porque de él depende que el demonio no quede zombi ni muera con el agente:

```rust
/// Starts the daemon detached: `sh` backgrounds it and leaves, so init adopts it (no zombie under
/// a long-lived client), and its own process group keeps an agent that kills its children's
/// group from taking it down.
fn spawn(launch: &Launch, data: &Path) -> Result<(), String> {
    use std::os::unix::process::CommandExt;
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(r#""$0" --db "$1" daemon </dev/null >/dev/null 2>>"$2" &"#)
        .arg(&launch.exe)
        .arg(data.join(crate::agent::memory::DB_FILE))
        .arg(data.join("daemon.log"))
        .envs(launch.env.iter().map(|(k, v)| (k, v)))
        .process_group(0)
        .status()
        .map_err(|e| format!("no se pudo lanzar el demonio: {e}"))?;
    status.success().then_some(()).ok_or_else(|| "no se pudo lanzar el demonio".to_owned())
}
```

`main.rs`: `daemon` sin banderas llama a `serve(data, &home()?, idle)` con `idle` de `CLIO_MEMORY_DAEMON_IDLE` o 600 s; `--status` imprime `demonio: activo (pid N, vX.Y.Z, modelo cargado|sin cargar, S sesiones)` o `demonio: inactivo (se inicia solo con la primera llamada)`; `--stop` manda `shutdown` sin `launch` e imprime `demonio detenido` o `no había demonio`.

- [ ] **Step 4: Correr y ver que pasan**

Run: `cargo test --locked --all-features --test daemon -- --test-threads=4`
Expected: PASS, y `pgrep -f "clio-memory.*daemon"` no deja procesos de prueba vivos al terminar.

- [ ] **Step 5: Commit**

```bash
git add src/daemon src/platform/unix.rs src/bin/clio-memory tests/daemon.rs tests/common/mod.rs
git commit -m "feat(daemon): one process per data folder, started on demand, gone when idle"
```

---

### Task 5: `clio-memory mcp` es un cliente delgado

**Files:**
- Modify: `src/mcp.rs:25-192`, `src/bin/clio-memory/main.rs:144-164`, `src/bin/clio-memory/args.rs` (texto de uso de `mcp`)
- Test: `tests/mcp.rs` (las existentes no cambian), `tests/mcp_daemon.rs` (nuevo)

**Interfaces:**
- Consumes: `daemon::{request, Launch}` (Tarea 4).
- Produces:
  - `pub enum mcp::Backend { Local, Daemon(Launch) }`
  - `pub fn mcp::serve_with(input, output, data, cwd, home, backend: Backend) -> Result<(), String>`; `mcp::serve` queda igual y llama a `serve_with(…, Backend::Local)`
  - La sesión que el proceso manda al demonio: `format!("{}-{}", std::process::id(), <segundos al arrancar>)`

- [ ] **Step 1: Escribir las pruebas que fallan** (`tests/mcp_daemon.rs`; un ayudante `mcp(data, home, extra_env)` lanza `clio-memory mcp` con tuberías y devuelve una función que manda una línea y lee la respuesta)

| Prueba | Afirma |
|---|---|
| `the_mcp_process_never_opens_the_database` (Linux) | tras `initialize` y un `memory_save` que responde `saved`, ningún `read_link` de `/proc/<pid>/fd/*` del proceso MCP contiene `memory.db`, y existe `<datos>/daemon.sock` |
| `five_sessions_saving_at_once_are_all_saved` | 5 procesos MCP, 10 guardados cada uno en hilos a la vez: las 50 respuestas empiezan por `saved`; ninguna contiene `otro proceso` |
| `the_budget_lives_in_the_daemon` | 20 `memory_context` responden; el 21.º responde con `isError: true` y `refills in` |
| `without_a_daemon_it_serves_in_its_own_process` | con `CLIO_MEMORY_NO_DAEMON=1`: `memory_save` responde `saved` y no existe `daemon.sock` |
| `when_the_daemon_dies_the_call_is_still_answered` (Linux) | tras un guardado, `kill -9` al pid del demonio (de `status`); el siguiente `memory_search` responde con el recuerdo |
| `a_socket_path_too_long_falls_back` | con `CLIO_MEMORY_SOCK` de 120 bytes: `memory_save` responde `saved` y stderr contiene `sin demonio` |

- [ ] **Step 2: Correr y ver que fallan**

Run: `cargo test --locked --all-features --test mcp_daemon`
Expected: FAIL.

- [ ] **Step 3: Implementar**

`State` guarda el `Backend`. Con `Local`, todo como hoy. Con `Daemon(launch)`: `initialize` solo recuerda el nombre del cliente (no crea `Session` ni toca el modelo); `tools/call` arma la petición `call` con el cliente, la sesión del proceso, `cwd` y los argumentos tal como llegaron, y traduce `ok` a `isError`. Si `request` falla (también tras su reintento), el proceso escribe una vez en stderr `clio-memory mcp: sin demonio ({motivo}): atiendo en este proceso`, cambia a `Local` para el resto de su vida y atiende esa misma llamada. Al terminar la entrada, en modo demonio no hay nada que cerrar.

`main.rs`: `Backend::Daemon(Launch { exe: current_exe()?, env: vec![] })`, salvo que `CLIO_MEMORY_NO_DAEMON` valga `1`.

- [ ] **Step 4: Correr y ver que pasan**

Run: `cargo test --locked --all-features --test mcp --test mcp_daemon --test cli`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/mcp.rs src/bin/clio-memory tests/mcp_daemon.rs
git commit -m "feat(mcp): the server forwards each tool call to the daemon, and serves alone without it"
```

---

### Task 6: `doctor`, `setup` y `uninstall` conocen al demonio

**Files:**
- Modify: `src/bin/clio-memory/commands.rs:273-336` (`doctor`), `src/bin/clio-memory/main.rs:226-363` (`uninstall`, `MemoryFiles`), `src/bin/clio-memory/setup_flow.rs` (al terminar), `docs/instalador.md`
- Test: `tests/cli.rs`

- [ ] **Step 1: Escribir las pruebas que fallan**

| Prueba | Afirma |
|---|---|
| `doctor_says_whether_the_daemon_runs` | sin demonio: una línea `info demonio: inactivo (se inicia solo con la primera llamada)`; con uno lanzado: `info demonio: activo (pid ` |
| `doctor_fails_on_a_socket_path_too_long` | con `CLIO_MEMORY_SOCK` de 120 bytes: `FAIL demonio: la ruta del socket es demasiado larga` y código 1 |
| `uninstall_data_stops_the_daemon_and_takes_its_files` | con un demonio corriendo: tras `uninstall --data --yes` no existen `daemon.sock`, `daemon.lock` ni `daemon.log`, y el pid ya no está |
| `setup_ends_by_stopping_an_older_daemon` | con un demonio corriendo: tras `setup --yes --agents claude-code` (agente falso), `daemon --status` dice `inactivo` |

- [ ] **Step 2: Correr y ver que fallan**

Run: `cargo test --locked --all-features --test cli daemon`
Expected: FAIL.

- [ ] **Step 3: Implementar**

`doctor` pide `status` sin lanzar (`launch: None`). `uninstall`: antes de borrar nada, `shutdown` sin lanzar y espera de hasta 2 s a que el socket desaparezca; `MemoryFiles.files` suma `<datos>/daemon.sock`, `daemon.lock` y `daemon.log`. `setup_flow::run`: último paso, sin línea propia si no había demonio, `demonio anterior detenido: el próximo uso lanza el de esta versión`.

- [ ] **Step 4: Correr y ver que pasan**

Run: `cargo test --locked --all-features --test cli --test setup --test setup_plan`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/bin/clio-memory tests/cli.rs docs/instalador.md
git commit -m "feat(setup): doctor, setup and uninstall know the daemon"
```

---

### Task 7: Los vectores, también para un programa anfitrión

**Files:**
- Modify: `src/daemon/client.rs`, `src/daemon/mod.rs` (reexportar), `README.md` (sección de la biblioteca)
- Test: `tests/daemon.rs`, `tests/granite.rs`

**Interfaces:**
- Produces: `pub struct daemon::RemoteEmbedder`; `pub fn RemoteEmbedder::connect(data: &Path, launch: Launch) -> Result<Self, String>` (pide `model`; error `el demonio no tiene modelo de vectores` con `no-model`); `impl Embedder for RemoteEmbedder` (`model_id` y `dim` los que dijo el demonio; `embed` manda `embed` y devuelve `Error::Embedding` si falla).

- [ ] **Step 1: Escribir las pruebas que fallan**

`tests/daemon.rs`: `a_host_without_a_model_in_the_daemon_is_told` — con `HOME` temporal vacío, `RemoteEmbedder::connect` es `Err` y contiene `no tiene modelo`.

`tests/granite.rs` (se salta sin `CLIO_MEMORY_GRANITE_DIR`): `the_daemons_vectors_are_granites` — con `HOME` temporal enlazado a los modelos, `RemoteEmbedder::connect` da `dim() == 384` y `model_id() == GRANITE_Q8_ID`; el vector de «el contenedor arrancaba antes que la base» tiene coseno ≥ 0,9999 con el de `Granite::load(dir)` en el propio proceso.

- [ ] **Step 2: Correr y ver que fallan**

Run: `cargo test --locked --all-features --test daemon a_host_without`
Expected: FAIL.

- [ ] **Step 3: Implementar** según las firmas. `README.md`: un programa que usa la biblioteca puede pasar `EmbedModel::from_embedder(Arc::new(RemoteEmbedder::connect(data, launch)?))` para no cargar su propia copia del modelo, y debe quedarse con el modelo local si `connect` falla.

- [ ] **Step 4: Correr y ver que pasan**

Run: `cargo test --locked --all-features --test daemon` y, con el modelo, `CLIO_MEMORY_GRANITE_DIR=$HOME/.cache/clio-models/granite-embedding-311m-multilingual-r2 cargo test --locked --release --features granite --test granite`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/daemon README.md tests/daemon.rs tests/granite.rs
git commit -m "feat(daemon): a host program can take its vectors from the daemon"
```

---

### Task 8: El banco de sesiones, la versión 0.10.0 y la documentación

**Files:**
- Modify: `bench/sessions/run.py` (ya existe, con la línea base de v0.9.0 en `bench/sessions/results.md`), `bench/sessions/results.md`, `Cargo.toml:5`, `Cargo.lock`, `README.md`, `docs/motor.md`, `docs/arquitectura.svg`

- [ ] **Step 1: Medir con el modelo real**

```bash
cargo build --locked --release --features model2vec,granite
for n in 1 3 5 8; do CLIO_BENCH_BIN=target/release/clio-memory python3 bench/sessions/run.py conc $n 15; done
CLIO_BENCH_BIN=target/release/clio-memory python3 bench/sessions/run.py cold 5
CLIO_BENCH_BIN=target/release/clio-memory python3 bench/sessions/run.py idle
```

Expected (aceptación, con 5 sesiones): la suma de RSS de los procesos `mcp` es menor que 60 MB; hay un solo proceso `daemon` y no pasa de 700 MB; `cold 5` guarda 5 de 5; tras 10 minutos sin uso no queda ningún proceso `daemon`. Si `run.py` no mide el demonio aparte, agregarle esa lectura (pid por `clio-memory daemon --status`).

- [ ] **Step 2: Escribir los resultados**

Agregar a `bench/sessions/results.md` la tabla de v0.10.0 al lado de la de v0.9.0, con la fecha y el equipo.

- [ ] **Step 3: Documentar y subir la versión**

`README.md` y `docs/motor.md`: un demonio por carpeta de datos, cómo se lanza y cuándo sale, `CLIO_MEMORY_NO_DAEMON`, `CLIO_MEMORY_SOCK`, `clio-memory daemon --status|--stop`. `docs/arquitectura.svg`: el demonio entre los clientes y la base. `version = "0.10.0"`.

- [ ] **Step 4: Comprobar como el CI y abrir el PR**

Run: `cargo fmt --all --check && cargo clippy --locked --all-targets --all-features -- -D warnings && cargo test --locked --all-features --no-fail-fast && cargo clippy --locked --lib --no-default-features --features model2vec,granite,telemetry -- -D warnings`
Expected: todo en verde.

```bash
git add bench/sessions Cargo.toml Cargo.lock README.md docs
git commit -m "chore: release v0.10.0"
gh pr create --title "v0.10.0: un solo proceso con el modelo" --body "…"
```

El cuerpo del PR lleva la tabla de antes y después, y recuerda que tras actualizar hay que reiniciar las sesiones de agente abiertas (las viejas siguen con su proceso de v0.9.x hasta cerrarse).
