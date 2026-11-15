# Estable 1 de 5 — v0.9.1: lo urgente — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** clio-memory deja de enviar telemetría a Microsoft, lo que se exporta se puede importar, purgar no deja el título, el sueño corre cuando la memoria lleva 30 minutos sin uso (a cualquier hora), `update` baja el instalador del release y la búsqueda encuentra por título y vectoriza la consulta una sola vez.

**Architecture:** Siete cambios independientes sobre v0.9.0, sin dependencias nuevas y sin cambiar el esquema de la base (sigue en 2). La telemetría se apaga con una variable de entorno que el binario se pone a sí mismo volviéndose a ejecutar, y la biblioteca se niega a iniciar ONNX Runtime sin ella. El sueño conserva su timer, que pasa a correr cada media hora todo el día; el sueño decide solo si le toca.

**Tech Stack:** Rust 2024 (1.95.0), rusqlite 0.41 (SQLite incluido), ort 2.0.0-rc.15 (ONNX Runtime 1.31.1 cargado en ejecución), systemd de usuario y launchd, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-11-15-estable-design.md` (secciones 1, 2 y 3 del diseño). Léelo antes de cada tarea: manda sobre este plan si chocan.

## Global Constraints

- Una rama, `fix/v0.9.1`, desde `main`; un commit por tarea; un PR al final. Iván fusiona y etiqueta: nunca hacer push a `main` ni etiquetar.
- TDD: la prueba primero. **Cada prueba nueva se verifica por mutación:** romper a mano la línea que la hace pasar, ver que falla y restaurar. Se anota en el mensaje del commit (`mutation-checked: <qué se rompió>`).
- Antes de cada commit: `cargo fmt --all --check`, `cargo clippy --locked --all-targets --all-features -- -D warnings`, `cargo test --locked --all-features --no-fail-fast` y `cargo clippy --locked --lib --no-default-features --features model2vec,granite,telemetry -- -D warnings`.
- `unsafe_code = "forbid"`, `unwrap_used = "deny"` (las pruebas pueden usar `unwrap`). No usar `cargo +nightly`. `std::env::set_var` es `unsafe` en la edición 2024: no se usa.
- Ninguna dependencia nueva. El esquema de la base sigue en la versión 2 (`src/sqlite/schema.rs:5`).
- Pruebas sin red y sin el `$HOME` real: `HOME` y carpeta de datos temporales (`tempfile`), programas falsos en un `PATH` de prueba.
- Textos para la persona en español con tildes; comentarios y nombres de código en inglés, con la densidad de comentarios del código vecino.
- No se toca `skill/SKILL.md` en esta entrega (cambiarlo obliga a registrar su sha256 en `src/setup/skill.rs:98`).
- Versión objetivo `0.9.1` (se sube en la última tarea).

## Review Focus

1. **Un binario reemplazado mientras corre.** Tras un `update`, `current_exe()` de una sesión vieja puede no existir: `private_runtime` devuelve error y el programa sigue, sin Granite y sin telemetría, nunca en bucle. Prueba en la Tarea 1.
2. **La memoria real de Iván.** Tiene m1 y m2 purgados y vueltos a nacer, y dos purgados con título. Debe exportarse, importarse y quedar limpia sin intervención. Pruebas en las Tareas 2 y 3 con un fixture hecho con el código de v0.9.0.
3. **Dos sueños el mismo día.** El segundo no borra lo que informó el primero ni rehace la copia. Prueba en la Tarea 6.
4. **Una operación que sí cambia un recuerdo purgado** (no un renacimiento) sigue rechazada al importar. Prueba en la Tarea 2.
5. **Una consulta sin letras ni dígitos** no toca el modelo, tampoco tras el cambio de `recall_many`. Prueba en la Tarea 5.

---

### Task 1: La telemetría de ONNX Runtime, apagada

**Files:**
- Create: `src/privacy.rs`, `.cargo/config.toml`
- Modify: `src/lib.rs:14-62`, `src/granite.rs:58-68`, `src/bin/clio-memory/main.rs:18`, `README.md`, `docs/motor.md`
- Test: `src/privacy.rs` (unitarias), `src/granite.rs` (unitaria), `tests/cli.rs`, `tests/granite.rs`

**Interfaces:**
- Produces:
  - `pub const ORT_TELEMETRY_VAR: &str = "ORT_DISABLE_TELEMETRY";` (en `privacy`, reexportada en `lib.rs`)
  - `pub fn telemetry_is_off(value: Option<&std::ffi::OsStr>) -> bool` — `true` solo si el valor es exactamente `1`
  - `pub fn private_runtime() -> Result<(), String>` — reexportada como `clio_memory::private_runtime`
  - `fn run_again(exe: &std::path::Path) -> String` (privada) — reemplaza el proceso por `exe` con la variable puesta y los mismos argumentos; solo vuelve, con el motivo, si no pudo

- [ ] **Step 1: Escribir las pruebas que fallan**

En `src/privacy.rs`:

```rust
#[test]
fn only_the_value_1_turns_telemetry_off() {
    use std::ffi::OsStr;
    assert!(telemetry_is_off(Some(OsStr::new("1"))));
    for v in ["", "0", "true", "yes", " 1"] {
        assert!(!telemetry_is_off(Some(OsStr::new(v))), "{v:?}");
    }
    assert!(!telemetry_is_off(None));
}

/// A binary replaced by an update while it ran: the program goes on, it does not loop or die.
#[test]
fn a_program_that_cannot_be_run_again_says_why_and_returns() {
    let why = run_again(std::path::Path::new("/no/existe/clio-memory"));
    assert!(!why.is_empty());
}
```

En `src/granite.rs` (módulo `tests` nuevo; `private` es la función de la Step 3):

```rust
#[test]
fn the_runtime_is_refused_while_telemetry_is_on() {
    let why = private(None).unwrap_err();
    assert!(why.contains("ORT_DISABLE_TELEMETRY"), "{why}");
    assert!(private(Some(std::ffi::OsStr::new("1"))).is_ok());
}
```

En `tests/cli.rs` (solo Linux; no necesita el modelo):

```rust
#[cfg(target_os = "linux")]
#[test]
fn the_binary_runs_itself_again_with_telemetry_off() {
    use std::io::{BufRead, BufReader, Write};
    let t = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_clio-memory"))
        .arg("mcp")
        .env_remove("ORT_DISABLE_TELEMETRY")
        .env("HOME", t.path())
        .env("CLIO_MEMORY_DATA", t.path().join("data"))
        .current_dir(t.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"ping"}}"#).unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let environ = std::fs::read(format!("/proc/{}/environ", child.id())).unwrap();
    drop(stdin);
    child.wait().unwrap();
    assert!(
        environ.split(|b| *b == 0).any(|v| v == b"ORT_DISABLE_TELEMETRY=1"),
        "the process serving must carry the variable"
    );
}
```

En `tests/granite.rs` (solo Linux, se salta sin `CLIO_MEMORY_GRANITE_DIR`, como las demás pruebas del archivo): `a_session_that_embedded_holds_no_telemetry_database`. Arranca `clio-memory mcp` como arriba, con `HOME` temporal en el que `.cache/clio-models` es un enlace simbólico a la carpeta madre de `CLIO_MEMORY_GRANITE_DIR`; manda `initialize` y un `tools/call` de `memory_save` (`text`: «Qué: prueba de vectores», `kind`: `semantic`); lee las dos respuestas; afirma que la segunda no contiene `sin modelo de vectores` y que ningún `read_link` de `/proc/<pid>/fd/*` contiene `onnxruntime.db`.

- [ ] **Step 2: Correr y ver que fallan**

Run: `cargo test --locked --all-features privacy:: granite::tests && cargo test --locked --all-features --test cli the_binary_runs_itself`
Expected: FAIL (no existe `privacy`, ni `private`; la prueba de `cli` no encuentra la variable).

- [ ] **Step 3: Implementar**

`src/privacy.rs`. El cuerpo se da porque la firma no lo determina:

```rust
/// Runs this program again with ONNX Runtime's telemetry off, unless it already is. Returns only
/// when nothing had to change (`Ok`) or the program could not be run again (`Err`): the caller
/// goes on either way, and Granite refuses to start the runtime without the variable.
pub fn private_runtime() -> Result<(), String> {
    if telemetry_is_off(std::env::var_os(ORT_TELEMETRY_VAR).as_deref()) {
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Err(run_again(&exe))
}

fn run_again(exe: &std::path::Path) -> String {
    use std::os::unix::process::CommandExt;
    std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env(ORT_TELEMETRY_VAR, "1")
        .exec()
        .to_string()
}
```

`src/granite.rs`: `fn private(value: Option<&std::ffi::OsStr>) -> std::result::Result<(), String>` devuelve `Err` con este texto exacto cuando `telemetry_is_off` es falso: `ORT_DISABLE_TELEMETRY=1 no está puesta: ONNX Runtime no se inicia con la telemetría encendida (llama a clio_memory::private_runtime() al empezar el programa)`. `env()` la llama antes de `ort::init_from` (dentro del `get_or_init`, con `std::env::var_os`), y agrega `.with_telemetry(false)` antes de `.build()`.

`src/bin/clio-memory/main.rs`: primera línea de `main`: `let _ = clio_memory::private_runtime();`.

`.cargo/config.toml`:

```toml
# Tests, examples and benches load ONNX Runtime in their own process: never with telemetry on.
[env]
ORT_DISABLE_TELEMETRY = "1"
```

- [ ] **Step 4: Correr y ver que pasan**

Run: `cargo test --locked --all-features --no-fail-fast`
Expected: PASS. Con el modelo instalado, además: `CLIO_MEMORY_GRANITE_DIR=$HOME/.cache/clio-models/granite-embedding-311m-multilingual-r2 cargo test --locked --release --features granite --test granite` → PASS, incluida `a_session_that_embedded_holds_no_telemetry_database`.

- [ ] **Step 5: Documentar**

`README.md`: una sección «Qué sale de esta máquina» con tres líneas: nada sale salvo las descargas de `setup` y `update`; ONNX Runtime trae telemetría de Microsoft y clio-memory la apaga con `ORT_DISABLE_TELEMETRY=1` y no carga el runtime sin ella; un programa que use la biblioteca debe llamar a `clio_memory::private_runtime()` al empezar. `docs/motor.md`: la misma regla donde habla de Granite.

- [ ] **Step 6: Commit**

```bash
git add src/privacy.rs src/lib.rs src/granite.rs src/bin/clio-memory/main.rs .cargo/config.toml tests/cli.rs tests/granite.rs README.md docs/motor.md
git commit -m "fix(privacy): ONNX Runtime never starts with its telemetry on"
```

---

### Task 2: Un id purgado no se vuelve a dar, y los que ya se dieron se importan

**Files:**
- Create: `tests/fixtures/export-reused-id-v0.9.0.jsonl`
- Modify: `src/sqlite/mod.rs:1333-1339` (`next_memory_id`), `src/sqlite/mod.rs:475-484` (restore)
- Test: `tests/purge.rs`, `tests/transfer.rs`

**Interfaces:**
- Consumes: `Engine::purge(memory, actor, reason)`, `Engine::export(&mut out)`, `Engine::import(&mut input)`, `common::{fixture, prop}`.
- Produces: el fixture, que la Tarea 3 vuelve a usar (lleva un purgado con título).

- [ ] **Step 1: Hacer el fixture con el código que aún reutiliza ids**

Antes de cambiar nada, agregar a `tests/transfer.rs` esta prueba temporal y correrla una vez:

```rust
#[test]
#[ignore = "writes the fixture; deleted in the same task"]
fn write_the_reused_id_fixture() {
    let f = fixture();
    let one = f.engine.propose(prop(f.scope, f.person, "uno")).unwrap().memory;
    let two = f
        .engine
        .propose(Proposal {
            title: Some("Título de dos".into()),
            ..prop(f.scope, f.person, "dos")
        })
        .unwrap()
        .memory;
    f.engine.purge(two, f.person, "prueba").unwrap();
    let three = f.engine.propose(prop(f.scope, f.person, "tres")).unwrap().memory;
    assert_eq!((one.0, two.0, three.0), (1, 2, 2), "v0.9.0 gives the purged id again");
    let mut out = Vec::new();
    f.engine.export(&mut out).unwrap();
    std::fs::write("tests/fixtures/export-reused-id-v0.9.0.jsonl", out).unwrap();
}
```

Run: `cargo test --locked --test transfer write_the_reused_id_fixture -- --ignored`
Expected: PASS, y el archivo existe. Borrar la prueba temporal. El fixture se queda.

- [ ] **Step 2: Escribir las pruebas que fallan**

`tests/purge.rs`:

```rust
#[test]
fn a_purged_id_is_never_given_to_a_new_memory() {
    let f = fixture();
    f.engine.propose(prop(f.scope, f.person, "uno")).unwrap();
    let two = f.engine.propose(prop(f.scope, f.person, "dos")).unwrap().memory;
    f.engine.purge(two, f.person, "prueba").unwrap();
    let three = f.engine.propose(prop(f.scope, f.person, "tres")).unwrap().memory;
    assert_eq!(three.0, 3);
}
```

`tests/transfer.rs`:

```rust
const REUSED: &str = include_str!("fixtures/export-reused-id-v0.9.0.jsonl");

#[test]
fn an_export_with_a_reborn_id_is_restored() {
    let t = tempfile::tempdir().unwrap();
    let engine = Engine::open(&t.path().join("m.db"), OpenMode::ReadWrite, Arc::new(SystemClock)).unwrap();
    engine.import(&mut REUSED.as_bytes()).unwrap();
    assert_eq!(engine.get(MemoryId(2)).unwrap().content, "tres");
    let next = engine.propose(prop(engine.scopes().unwrap()[0].id, engine.actors().unwrap()[0].id, "cuatro"));
    assert_eq!(next.unwrap().memory.0, 3, "never 2 again");
}

#[test]
fn an_operation_that_changes_a_purged_memory_is_still_refused() {
    // The fixture's last Add, turned into a change of the purged memory: it gains a `before`.
    let forged = forge_before_on_last_change(REUSED); // helper below
    let t = tempfile::tempdir().unwrap();
    let engine = Engine::open(&t.path().join("m.db"), OpenMode::ReadWrite, Arc::new(SystemClock)).unwrap();
    assert!(bad_import(engine.import(&mut forged.as_bytes())).contains("was purged"));
}
```

`forge_before_on_last_change(text: &str) -> String`: en la última línea que sea una operación con `changes` no vacío, copia `changes[0].after` en `changes[0].before` (con `serde_json::Value`) y vuelve a serializar esa línea.

- [ ] **Step 3: Correr y ver que fallan**

Run: `cargo test --locked --test purge a_purged_id && cargo test --locked --test transfer reborn`
Expected: FAIL (`three.0` es 2; `import rejected: … memory 2 was purged`).

- [ ] **Step 4: Implementar**

`next_memory_id`: el mayor entre los recuerdos vivos y todo recuerdo que alguna operación tocó (`operation_memories` conserva los purgados):

```sql
SELECT MAX(m) FROM (
  SELECT COALESCE(MAX(id), 0) AS m FROM memories
  UNION ALL SELECT COALESCE(MAX(memory), 0) FROM operation_memories)
```

Restore (`src/sqlite/mod.rs:475-484`): cuando una operación toca un id que está en `purged`, se acepta solo si no es un `Purge` y todos sus cambios sobre ese id tienen `before == None`; entonces el id sale de `purged` (volvió a nacer) y la operación se importa como cualquier otra. En cualquier otro caso, el error de hoy, con el mismo texto.

- [ ] **Step 5: Correr y ver que pasan**

Run: `cargo test --locked --all-features --test purge --test transfer`
Expected: PASS (las pruebas existentes de `transfer` también).

- [ ] **Step 6: Commit**

```bash
git add src/sqlite/mod.rs tests/purge.rs tests/transfer.rs tests/fixtures/export-reused-id-v0.9.0.jsonl
git commit -m "fix(store): a purged id is never reused, and a file where one was is restored"
```

---

### Task 3: Purgar borra también el título y la categoría

**Files:**
- Modify: `src/sqlite/mod.rs:1116-1123` (`scrub`), `src/sqlite/mod.rs:376-393` (`check_purged_clean`), `src/store.rs` (dos métodos), `src/engine.rs` (dos métodos y el chequeo de `doctor`), `src/sleep/mod.rs:296-363` (`Night::sleep`)
- Test: `tests/purge.rs`, `tests/sleep_engine.rs`, `tests/transfer.rs`, `tests/cli.rs`

**Interfaces:**
- Consumes: el fixture de la Tarea 2 (su purgado lleva el título `Título de dos`).
- Produces:
  - `Store::unscrubbed_purged(&self) -> Result<usize>` y `Store::rescrub_purged(&self) -> Result<usize>` (cuántas instantáneas se limpiaron)
  - `Engine::unscrubbed_purged(&self) -> Result<usize>` (lectura)
  - `Engine::sleep_rescrub_purged(&self) -> Result<usize>` (escritura; no crea operación: reescribe instantáneas, como hace `purge`)
  - chequeo de `doctor` llamado `purged_scrubbed`

- [ ] **Step 1: Escribir las pruebas que fallan**

`tests/purge.rs`:

```rust
#[test]
fn purge_erases_the_title_and_the_category_from_every_snapshot() {
    let f = fixture();
    let id = f
        .engine
        .propose(Proposal {
            title: Some("Clave del banco".into()),
            category: Some(Category::Config),
            ..prop(f.scope, f.person, "el contenido")
        })
        .unwrap()
        .memory;
    f.engine.purge(id, f.person, "prueba").unwrap();
    let dump = common::dump_all(&f.path);
    assert!(!dump.contains("Clave del banco"), "{dump}");
    assert_eq!(f.engine.unscrubbed_purged().unwrap(), 0);
}
```

`tests/transfer.rs`:

```rust
#[test]
fn an_import_scrubs_what_an_older_purge_left() {
    let t = tempfile::tempdir().unwrap();
    let path = t.path().join("m.db");
    let engine = Engine::open(&path, OpenMode::ReadWrite, Arc::new(SystemClock)).unwrap();
    assert!(REUSED.contains("Título de dos"), "the fixture carries the purged title");
    engine.import(&mut REUSED.as_bytes()).unwrap();
    assert!(!common::dump_all(&path).contains("Título de dos"));
}
```

`tests/sleep_engine.rs`: `the_sleep_scrubs_what_older_purges_left`. Propone un recuerdo con título `Viejo título`, lo purga, y con `rusqlite` vuelve a poner el título en la instantánea de su `Add` (`UPDATE operations SET changes = replace(changes, '"title":null', '"title":"Viejo título"') WHERE id = 1`). Afirma: `unscrubbed_purged() == 1`; `sleep_rescrub_purged() == 1`; el volcado ya no contiene `Viejo título`; una segunda llamada devuelve 0.

`tests/cli.rs`: `doctor_fails_while_a_purged_memory_keeps_its_title` — el mismo archivo manipulado; `clio-memory --db … doctor` sale con 1 y una línea que empieza por `FAIL purged_scrubbed`.

- [ ] **Step 2: Correr y ver que fallan**

Run: `cargo test --locked --all-features --test purge --test transfer --test sleep_engine --test cli`
Expected: FAIL en las cuatro nuevas.

- [ ] **Step 3: Implementar**

- `scrub` pone además `row.title = None; row.category = None;`.
- `rescrub_purged`: por cada operación `Purge` P de un recuerdo M, toma las operaciones con id menor que P que tocaron M (`operation_memories`), y a cada cambio de M le aplica `scrub` en `before` y `after`; reescribe `changes` solo si cambió. Las operaciones posteriores a P no se tocan (pueden ser de un M renacido). Una transacción.
- `unscrubbed_purged`: el mismo recorrido, contando sin escribir.
- `check_purged_clean` no cambia (sigue mirando contenido y hash). Al final de la transacción de `import` se llama a la misma rutina de limpieza.
- `Night::sleep`: después de `self.vectors()?` y antes de la cuarentena, si no es `dry_run` y `self.read.unscrubbed_purged()? > 0`: `self.writer()?.sleep_rescrub_purged()`; un error se anota como alarma (`la limpieza de purgados falló: …`) y la noche sigue.
- `doctor`: `purged_scrubbed` es `ok` con 0 y `FAIL` con N: `N purged memories still carry a title or a category (the next sleep cleans them)`.

- [ ] **Step 4: Correr y ver que pasan**

Run: `cargo test --locked --all-features --no-fail-fast`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/sqlite/mod.rs src/store.rs src/engine.rs src/sleep/mod.rs tests/purge.rs tests/transfer.rs tests/sleep_engine.rs tests/cli.rs
git commit -m "fix(purge): a purge erases the title and the category too; the sleep cleans older ones"
```

---

### Task 4: `update` baja el instalador del release, no de `main`

**Files:**
- Modify: `src/setup/mod.rs:35-36`, `.github/workflows/release.yml:181-192`, `docs/instalador.md`, `docs/releases.md`
- Test: `tests/update.rs`

- [ ] **Step 1: Escribir la prueba que falla**

```rust
#[test]
fn the_installer_comes_from_the_latest_release() {
    assert_eq!(
        clio_memory::setup::INSTALLER_URL,
        "https://github.com/i-montes/clio-memory/releases/latest/download/install.sh"
    );
}
```

- [ ] **Step 2: Correr y ver que falla**

Run: `cargo test --locked --test update the_installer_comes`
Expected: FAIL (hoy apunta a `raw.githubusercontent.com/…/main/install.sh`).

- [ ] **Step 3: Implementar**

- `INSTALLER_URL` con ese valor; su comentario dice que el release es inmutable y que el instalador es el del mismo release que el binario que baja.
- `release.yml`, trabajo `publish`: agregar `actions/checkout` fijado por SHA (el mismo que usa `ci.yml`) antes de bajar los artefactos; en el paso `SHA256SUMS`, copiar antes `../install.sh` a `dist/` y calcular `sha256sum clio-memory-* install.sh > SHA256SUMS`; publicar con `gh release create "$TAG" clio-memory-* install.sh SHA256SUMS …`. La atestación sigue siendo de `dist/clio-memory-*`.
- `docs/releases.md`: quitar que la inmutabilidad está pendiente (verificado el 2026-11-15: `gh api repos/i-montes/clio-memory/immutable-releases` dice `"enabled": true` y v0.9.0 es inmutable) y agregar `install.sh` a la lista de archivos del release. `docs/instalador.md`: `update` usa el instalador del último release; la línea de `curl … | sh` sigue apuntando a `main`.

- [ ] **Step 4: Correr y ver que pasa**

Run: `cargo test --locked --test update && shellcheck install.sh`
Expected: PASS, sin avisos.

- [ ] **Step 5: Commit**

```bash
git add src/setup/mod.rs .github/workflows/release.yml tests/update.rs docs/instalador.md docs/releases.md
git commit -m "fix(update): the installer comes from the latest release, not from main"
```

---

### Task 5: Buscar encuentra por título y vectoriza la consulta una vez

**Files:**
- Modify: `src/engine.rs:2485-2605` (`recall_many`, `recall`, `recall_inner`)
- Test: `tests/recall.rs`

**Interfaces:**
- Produces (privadas en `Engine`): `fn query_vector(&self, query: &str) -> Option<Vec<f32>>` y `fn recall_inner(&self, scope: ScopeId, query: &str, opts: &RecallOpts, vector: Option<&[f32]>) -> Result<Recall>`. Las firmas públicas no cambian.

- [ ] **Step 1: Escribir las pruebas que fallan**

```rust
#[test]
fn a_word_only_in_the_title_is_found() {
    let f = fixture();
    let id = f
        .engine
        .propose(Proposal {
            title: Some("Zanahoria cuántica elegida".into()),
            ..prop(f.scope, f.person, "se eligió el proveedor de despliegue por costo")
        })
        .unwrap()
        .memory;
    let r = f.engine.recall(f.scope, "zanahoria cuántica", &RecallOpts::default()).unwrap();
    assert!(!r.abstained);
    assert_eq!(r.hits[0].memory.id, id);
}

#[test]
fn recall_many_embeds_the_query_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let f = fixture_with(Arc::new(CountingEmbedder {
        inner: HashEmbedder::new("hash", 16),
        calls: calls.clone(),
    }));
    let other = f.engine.create_scope("otro", "es").unwrap();
    let third = f.engine.create_scope("tercero", "es").unwrap();
    calls.store(0, Ordering::SeqCst);
    f.engine.recall_many(&[f.scope, other, third], "una pregunta", &RecallOpts::default()).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn a_query_with_no_letters_never_reaches_the_model_through_recall_many() {
    let calls = Arc::new(AtomicUsize::new(0));
    let f = fixture_with(Arc::new(CountingEmbedder {
        inner: HashEmbedder::new("hash", 16),
        calls: calls.clone(),
    }));
    let r = f.engine.recall_many(&[f.scope], "¿¿¿ ???", &RecallOpts::default()).unwrap();
    assert!(r.abstained);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
```

- [ ] **Step 2: Correr y ver que fallan**

Run: `cargo test --locked --test recall title_is_found recall_many_embeds no_letters`
Expected: FAIL las dos primeras (se abstiene; 3 llamadas). La tercera puede pasar ya: es la guarda del punto 5 de Review Focus.

- [ ] **Step 3: Implementar**

- `covered` recibe el texto que se indexa: el título, un salto de línea y el contenido cuando hay título (como `fold.rs:47-50`), no solo `h.memory.content`.
- `query_vector`: `None` sin modelo, con una consulta sin letras ni dígitos, o si el modelo falla (el `eprintln!` de hoy se queda aquí). `recall` la llama una vez y pasa el resultado a `recall_inner`; `recall_many` la llama una vez antes del bucle.

- [ ] **Step 4: Correr y ver que pasan**

Run: `cargo test --locked --all-features --test recall --test recall_eval`
Expected: PASS; `recall_eval` (solo texto) no baja de sus pisos.

- [ ] **Step 5: Commit**

```bash
git add src/engine.rs tests/recall.rs
git commit -m "fix(recall): a title-only word is found; the query is embedded once for every place"
```

---

### Task 6: El sueño corre cuando la memoria no se usa

**Files:**
- Modify: `src/sleep/mod.rs:87-243` y `:296-363`, `src/sleep/state.rs`, `src/setup/mod.rs:264-312`, `src/tools.rs:241` (`Session::call`), `src/bin/clio-memory/commands.rs:248-268`, `src/bin/clio-memory/args.rs:35-53` (texto de uso), `README.md`, `docs/motor.md`
- Test: `tests/sleep.rs`, `tests/sleep_setup.rs`, `tests/tools.rs`, `tests/cli.rs`

**Interfaces:**
- Produces:
  - `pub fn sleep::mark_used(data: &Path, now: i64)` — escribe `now` en `<data>/sleep/last-use` (0600, entero o nada); nunca falla hacia fuera
  - `pub fn sleep::last_used(data: &Path) -> i64` — 0 si no existe o no se lee
  - `pub fn sleep::overdue(data: &Path, now: i64) -> Option<i64>` — días desde el último sueño completo, solo si son más de 3 y hay operaciones ajenas después del cursor
  - `SleepState.last_merge: i64` con `#[serde(default)]`
  - constantes en `sleep/mod.rs`: `QUIET = 30 * 60` (igual), `MERGE_EVERY: i64 = 20 * 3600` (reemplaza a `SLEPT_WITHIN`), `HOUSEKEEPING: i64 = 24 * 3600`
  - `Slept::Skipped("nada cambió desde el último sueño")`

- [ ] **Step 1: Escribir las pruebas que fallan**

En `tests/sleep.rs` (usar los ayudantes del archivo: `night()`, `open()`, `write_state()`):

| Prueba | Preparación | Afirma |
|---|---|---|
| `it_skips_when_nothing_changed_since_the_last_sleep` (reemplaza a `it_skips_when_slept_less_than_20h_ago`) | un sueño completo; reloj +2 h; ninguna operación nueva | `Slept::Skipped("nada cambió desde el último sueño")` |
| `it_sleeps_again_the_same_day_when_something_changed` | un sueño completo; un guardado; reloj +1 h | `Slept::Done(r)` con `r.complete` |
| `a_read_in_the_last_30_minutes_counts_as_use` | un guardado hace 2 h; `mark_used(data, now - 60)` | `Slept::Skipped("la memoria está en uso")` |
| `a_day_of_housekeeping_runs_even_when_nothing_changed` | `write_state(&n, now - 25 * 3600)`, sin operaciones nuevas | `Slept::Done(_)` |
| `the_merger_runs_at_most_once_in_20_hours` | dos sueños separados 1 h, cada uno con un par casi igual nuevo y un `Merger` que cuenta sus llamadas | el contador vale 1 tras el segundo; el par del segundo queda en `pending` del estado |
| `two_sleeps_of_a_day_share_one_report` | el primer sueño manda un secreto a cuarentena; el segundo (mismo día, con otro guardado) no | `last_report(data)` conserva la cuarentena del primero y `complete` es `true` |

`tests/sleep_setup.rs`: el timer contiene `OnCalendar=*:0/30` y no contiene `23,00`; el plist contiene `<key>StartInterval</key><integer>1800</integer>` y no `StartCalendarInterval`.

`tests/tools.rs`: `a_read_marks_the_memory_as_used` — tras `session.call("memory_search", …)`, `clio_memory::sleep::last_used(data) > 0`.

`tests/cli.rs`: `doctor_fails_when_the_sleep_is_overdue` — un `sleep/state.json` con `last_complete` de hace 4 días y una operación posterior al cursor: `doctor` sale con 1 y una línea que empieza por `FAIL sueño: no corre desde hace 4 días`.

- [ ] **Step 2: Correr y ver que fallan**

Run: `cargo test --locked --all-features --test sleep --test sleep_setup --test tools --test cli`
Expected: FAIL en las nuevas.

- [ ] **Step 3: Implementar**

En `sleep::run`, en este orden:
1. Sin base o con otro sueño corriendo: igual que hoy. Se quita el salto «ya durmió esta noche».
2. `quick_check`: igual.
3. Uso: `activity = max(read.last_activity(system), state::last_used(data))`; si `!opts.now && now - activity < QUIET` → `Skipped("la memoria está en uso")`.
4. Algo que hacer: sea `changed` «hay alguna operación después de `state.cursor` que no firmó el sistema». Si `!opts.now && !changed && state.pending.is_empty() && state.rules_version == RULES_VERSION && now - state.last_complete < HOUSEKEEPING` → `Skipped("nada cambió desde el último sueño")`. Esto va antes de `model.get()`.
5. El informe del día se carga aunque esté completo y se sigue sumando en él (`complete = false` al empezar).
6. `Night::sleep`: el paso de unión corre solo si `opts.now || now - state.last_merge >= MERGE_EVERY`; si no, los recuerdos nuevos se dejan en `deferred` como cuando no hay modelo. `disturbed()` también cuenta `last_used(data) > self.activity`.
7. Al terminar bien: `last_merge` pasa a `now` solo si el paso de unión corrió; si no, conserva el del estado.

`mark_used`: `Session` lleva `marked: i64` y llama a `mark_used(&self.data, now)` al entrar en `call` cuando `now - self.marked >= 60`.

`sleep_units`: el timer es `[Unit]\nDescription=clio-memory: despierta el sueño cada media hora\n\n[Timer]\nOnCalendar=*:0/30\nAccuracySec=1min\n\n[Install]\nWantedBy=timers.target\n`. El comentario deja de decir «No `Persistent`…» y dice que el sueño decide solo si le toca. `launchd_plist`: `StartInterval` 1800 en lugar de `StartCalendarInterval`. `SLEEP_HOURS` se borra.

`commands.rs::slept`: si `sleep::overdue(data, now)` da `Some(d)`, la línea es `FAIL sueño: no corre desde hace {d} días y hay cambios sin revisar (¿está el timer? systemctl --user status clio-memory-sleep.timer)` y `doctor` sale con 1.

Textos: en `args.rs`, `setup sleep` «a user timer that runs `clio-memory sleep` every half hour» y `sleep` «only when nobody used the memory for 30 minutes and something changed since the last sleep (or a day went by)». `README.md` y `docs/motor.md`: el sueño ya no es nocturno; el aviso «Anoche (sueño): …» conserva su texto.

- [ ] **Step 4: Correr y ver que pasan**

Run: `cargo test --locked --all-features --no-fail-fast`
Expected: PASS. Las pruebas existentes de `tests/sleep.rs` que escribían `last_complete` para forzar el salto de 20 h se adaptan a la regla nueva (la de `a_manual_run_does_not_take_the_night` sigue valiendo: `--now` no mueve `last_complete`).

- [ ] **Step 5: Commit**

```bash
git add src/sleep src/setup/mod.rs src/tools.rs src/bin/clio-memory tests/sleep.rs tests/sleep_setup.rs tests/tools.rs tests/cli.rs README.md docs/motor.md
git commit -m "feat(sleep): it runs when the memory is idle, at any hour; reads count as use"
```

---

### Task 7: Versión 0.9.1 y comprobación en una máquina real

**Files:**
- Modify: `Cargo.toml:5`, `Cargo.lock`, `docs/motor.md:50` (dice cinco minutos; son 60 s, `src/idle.rs:11`), `docs/releases.md`

- [ ] **Step 1: Subir la versión**

`version = "0.9.1"`; `cargo build --locked` no debe pedir cambios en dependencias (solo la línea del paquete en `Cargo.lock`, con `cargo update -p clio-memory --offline` si hace falta).

- [ ] **Step 2: Comprobar todo como el CI**

Run: `cargo fmt --all --check && cargo clippy --locked --all-targets --all-features -- -D warnings && cargo test --locked --all-features --no-fail-fast && cargo clippy --locked --lib --no-default-features --features model2vec,granite,telemetry -- -D warnings && shellcheck install.sh`
Expected: todo en verde.

- [ ] **Step 3: Comprobar sobre una copia de una memoria real** (si existe `~/.local/share/clio/memory.db`; nunca sobre el archivo real)

```bash
T=$(mktemp -d) && sqlite3 -readonly ~/.local/share/clio/memory.db ".backup $T/memory.db"
cargo run --locked -q -- --db $T/memory.db export $T/e.jsonl
mkdir $T/r && cargo run --locked -q -- --db $T/r/memory.db import $T/e.jsonl   # antes: «import rejected: … was purged»
cargo run --locked -q -- --db $T/r/memory.db doctor                             # purged_scrubbed: ok
```

Expected: `import` termina bien y `doctor` no tiene ningún `FAIL`.

- [ ] **Step 4: Commit y PR**

```bash
git add Cargo.toml Cargo.lock docs
git commit -m "chore: release v0.9.1"
gh pr create --title "v0.9.1: sin telemetría, exportación restaurable, sueño por inactividad" --body "…"
```

El cuerpo del PR lista, para después de fusionar y etiquetar: (1) `clio-memory update` en la máquina; (2) reiniciar las sesiones de agente abiertas; (3) en Clio, subir el commit fijado, llamar a `clio_memory::private_runtime()` al empezar `main` y correr `clio memory setup sleep` para que el timer tome el horario nuevo.
