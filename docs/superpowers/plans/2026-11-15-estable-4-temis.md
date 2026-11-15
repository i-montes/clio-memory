# Estable 4 de 5 — v0.12.0: el sueño decide con Temis — plan de implementación

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cuando la persona configura la URL y el token de Temis, el sueño de clio-memory le pregunta por cada par de recuerdos parecidos y dobla el que está contenido en el otro, sin gastar tokens de un modelo de lenguaje; sin Temis, el sueño sigue igual que hoy.

**Architecture:** Un cliente pequeño (`src/temis.rs`) habla con `POST <url>/v1/decide` por medio de `curl --config -` (el token nunca va en la línea de órdenes). El sueño recibe un juez de pares opcional (`sleep::PairJudge`), que Temis implementa, y un paso nuevo entre los repetidos y la unión con modelo de lenguaje. La configuración vive en `<datos>/temis.json` (0600) y la pone `clio-memory setup temis`.

**Tech Stack:** Rust 2024 (1.95.0), `serde_json`, `curl` del sistema (como las descargas de `setup`), `cliclack` para pedir el token sin eco (feature `cli`).

**Spec:** `docs/superpowers/specs/2026-11-15-estable-design.md` (sección 6 del diseño) y `bench/temis/results.md` (de dónde salen los umbrales). Léelos antes de cada tarea: el diseño manda sobre este plan si chocan.

## Global Constraints

- Requiere v0.9.1 fusionada (plan 1); no depende de los planes 2 y 3. Una rama, `feat/temis`, desde `main`; un commit por tarea; un PR al final. Iván fusiona y etiqueta.
- TDD con verificación por mutación anotada en el commit. Antes de cada commit, las cuatro órdenes de comprobación de los planes anteriores.
- `unsafe_code = "forbid"`, `unwrap_used = "deny"`. Ninguna dependencia nueva.
- **Las pruebas nunca llaman a Temis ni a la red:** un `Post` falso o un `curl` falso en un `PATH` de prueba. El token real no aparece en ningún archivo del repositorio, mensaje de commit, registro ni informe.
- Contrato de Temis, verificado el 2026-11-15 contra `https://api.imontes.dev/temis` (`temis-2.4.0`):
  - `POST <url>/v1/decide`, cabeceras `Content-Type: application/json` y `Authorization: Bearer <token>`.
  - Cuerpo: `{"model":"<modelo>","state":{…},"questions":{…}}`.
  - Respuesta 200: `{"model":"temis-2.4.0","answers":{"<id>":{"type":"noul","noul":0.79}},"usage":{…}}`; para `choice`: `{"type":"choice","choice":"conflicts_with","confidence":0.97,"probabilities":{…}}`.
  - Errores (400, 401, 413, 429, 503): `{"detail":{"error_type":"authentication_error","message":"…"}}`.
  - `GET <url>/health` sin token: `{"status":"ok","model":"temis-2.4.0"}`. `GET <url>/v1/model` con token.
- Preguntas, con este texto exacto (es el del entrenamiento de Temis):

  | Decisión | `state` | `questions` |
  |---|---|---|
  | contenido | `{"original": A, "unido": B}` | `{"se_conserva":{"type":"noul","instructions":"Todo lo que dice `original` está también en `unido`, aunque sea con otras palabras."}}` |
  | relación | `{"propuesta": P, "otra": O}` | `{"relacion":{"type":"choice","instructions":"Decide cómo se relaciona `propuesta` con `otra`.","criteria":{"conflicts_with":"Se contradicen: no pueden ser ciertas a la vez.","scoped":"`propuesta` vale en un caso más concreto que `otra` (este proyecto, este caso).","compatible":"Hablan de lo mismo y pueden ser ciertas a la vez.","related":"Tratan un tema cercano, sin más.","ninguna":"No tienen que ver."}}}` |

- Valores exactos:

  | Qué | Valor |
  |---|---|
  | Archivo de configuración | `<datos>/temis.json`, modo 0600: `{"url","token","model"}` |
  | Variables que mandan sobre el archivo | `CLIO_MEMORY_TEMIS_URL`, `CLIO_MEMORY_TEMIS_TOKEN` (las dos juntas) |
  | Modelo por defecto | `temis-latest` |
  | Largo mínimo del token | 24 caracteres |
  | Tiempo límite por llamada | 10 s |
  | Texto enviado por recuerdo | su contenido, hasta 4000 caracteres |
  | Umbral de «contenido» (`PRESERVED_MIN`) | 0,80 |
  | Veto por contradicción (`CONFLICT_VETO`) | `conflicts_with` con confianza ≥ 0,50 |
  | Pares por sueño (`MAX_JUDGED`) | 40 |
  | Fallos seguidos que apagan a Temis por ese proceso | 3 |
  | Razón de la cuarentena | `sueño: contenido en m<N>` |

- URL: solo `https://`, salvo `http://localhost`, `http://127.0.0.1` o `http://[::1]` (con o sin puerto), o cualquier `http://` con `--allow-http`.
- Textos para la persona en español con tildes; código y comentarios en inglés.
- Versión objetivo `0.12.0`.

## Review Focus

1. **Temis caído, lento o con el token vencido.** El sueño termina completo con lo demás hecho; los pares quedan pendientes; hay una sola alarma, sin el token ni el cuerpo de la respuesta. Prueba en la Tarea 3.
2. **Un par que dice lo mismo salvo una cifra o una negación** («puerto 5432» y «puerto 6432»; «activado» y «desactivado»). No se dobla: lo paran las marcas o el veto. Prueba en la Tarea 3.
3. **El token en un lugar donde se lee.** Ni en la línea de órdenes de `curl`, ni en `doctor`, ni en el informe del sueño, ni en un mensaje de error. Pruebas en las Tareas 1 y 4.
4. **Un recuerdo con algo que parece un secreto** no sale hacia Temis aunque la cuarentena lo haya dejado (es de la persona o está fijado). Prueba en la Tarea 3.
5. **Una respuesta de Temis con forma rara** (sin `answers`, `noul` fuera de 0 a 1, una opción que no se pidió). Cuenta como sin respuesta, nunca como un sí. Prueba en la Tarea 1.

---

### Task 1: El cliente de Temis

**Files:**
- Create: `src/temis.rs`
- Modify: `src/lib.rs` (`pub mod temis;`)
- Test: `tests/temis.rs` (nuevo)

**Interfaces:**
- Produces:
  - `pub struct Config { pub url: String, pub token: String, pub model: String }` (su `Debug` escribe `token: "…"`)
  - `pub fn config_path(data: &Path) -> PathBuf`
  - `pub fn load(data: &Path, env: &dyn Fn(&str) -> Option<String>) -> Option<Config>`
  - `pub fn save(data: &Path, cfg: &Config) -> Result<(), String>` y `pub fn remove(data: &Path) -> Result<bool, String>`
  - `pub fn check_url(url: &str, allow_http: bool) -> Result<String, String>` — devuelve la URL sin barra final
  - `pub type Post = dyn Fn(&Config, &str, Option<&str>) -> Result<(u16, String), String> + Send + Sync;` — (configuración, ruta como `/v1/decide`, cuerpo JSON o `None` para GET) → (código HTTP, cuerpo)
  - `pub fn curl_post(curl: PathBuf) -> Box<Post>`
  - `pub fn curl_config(cfg: &Config, path: &str, body: Option<&str>) -> String` — el texto que `curl` lee por la entrada estándar
  - `pub struct Temis`; `pub fn Temis::new(cfg: Config, post: Box<Post>) -> Self`
  - `pub fn Temis::decide(&self, state: &serde_json::Value, questions: &serde_json::Value) -> Result<serde_json::Value, String>` — el objeto `answers`
  - `pub fn Temis::health(&self) -> Result<String, String>` (el nombre del modelo, sin token) y `pub fn Temis::model(&self) -> Result<String, String>` (con token)
  - `pub fn Temis::preserved(&self, original: &str, merged: &str) -> Option<f32>` y `pub fn Temis::relation(&self, proposal: &str, other: &str) -> Option<(Relation, f32)>` (`ninguna` → `Relation::NotConflict`)

- [ ] **Step 1: Escribir las pruebas que fallan** (`tests/temis.rs`; `fake(answers)` devuelve un `Box<Post>` que guarda cada llamada en un `Arc<Mutex<Vec<(String, Option<String>)>>>` y responde lo que se le dio)

```rust
#[test]
fn the_question_is_the_one_temis_was_trained_on() {
    let (post, calls) = fake(vec![(200, r#"{"model":"temis-2.4.0","answers":{"se_conserva":{"type":"noul","noul":0.79}},"usage":{"input_tokens":1,"output_tokens":0}}"#)]);
    let t = Temis::new(cfg(), post);
    assert_eq!(t.preserved("uno", "uno y dos"), Some(0.79));
    let (path, body) = calls.lock().unwrap()[0].clone();
    let body: serde_json::Value = serde_json::from_str(&body.unwrap()).unwrap();
    assert_eq!(path, "/v1/decide");
    assert_eq!(body["model"], "temis-latest");
    assert_eq!(body["state"], json!({"original": "uno", "unido": "uno y dos"}));
    assert_eq!(
        body["questions"]["se_conserva"]["instructions"],
        "Todo lo que dice `original` está también en `unido`, aunque sea con otras palabras."
    );
}

#[test]
fn the_token_goes_to_curl_on_stdin_escaped_and_never_as_an_argument() {
    let c = Config { url: "https://h/temis".into(), token: "tok\"en\\x-0123456789abcdef0123".into(), model: "temis-latest".into() };
    let text = curl_config(&c, "/v1/decide", Some(r#"{"a":"b \"c\""}"#));
    assert!(text.contains(r#"url = "https://h/temis/v1/decide""#));
    assert!(text.contains(r#"header = "Authorization: Bearer tok\"en\\x-0123456789abcdef0123""#));
    assert!(text.contains(r#"data-binary = "{\"a\":\"b \\\"c\\\"\"}""#));
    assert!(text.contains("max-time = 10"));
}
```

| Prueba | Afirma |
|---|---|
| `a_choice_maps_to_a_relation` | con `choice: "conflicts_with", confidence: 0.97` → `Some((Relation::ConflictsWith, 0.97))`; `ninguna` → `NotConflict`; `scoped`, `compatible`, `related` → sus variantes |
| `an_odd_answer_is_no_answer` | sin `answers`; `noul: 1.4`; `noul: "sí"`; `choice: "otra_cosa"`; JSON roto → `None` en todos |
| `an_error_names_its_kind_and_never_the_token` | 401 con `{"detail":{"error_type":"authentication_error","message":"…"}}` → `decide` es `Err` que contiene `authentication_error` y `401` y no contiene el token |
| `three_failures_in_a_row_turn_it_off` | tres respuestas 503: la cuarta llamada a `preserved` da `None` sin que `post` se llame (el registro tiene 3 llamadas) |
| `a_success_resets_the_count` | 503, 503, 200, 503, 503 → la sexta sí llama |
| `long_texts_are_cut_at_4000_characters` | un texto de 5000 «á» → el `state.original` enviado tiene 4000 caracteres |
| `the_file_is_private_and_the_environment_wins` | `save` deja el archivo en 0600; `load` con las dos variables devuelve las de las variables; con una sola, el archivo; sin archivo ni variables, `None` |
| `only_https_or_this_machine` | `check_url`: `https://api.x/temis/` → `Ok("https://api.x/temis")`; `http://localhost:8765` y `http://127.0.0.1:1` → `Ok`; `http://api.x` → `Err`; con `allow_http` → `Ok`; `ftp://x`, `api.x` → `Err` |
| `debug_never_prints_the_token` | `format!("{cfg:?}")` no contiene el token |
| `curl_runs_with_the_config_on_stdin` | con un `curl` falso (un guion `sh` que copia su entrada a un archivo y sus argumentos a otro, e imprime `{"status":"ok","model":"t"}\n200`): `health()` es `Ok("t")`; los argumentos son exactamente `--config -`; la entrada contiene `url = "…/health"` y **no** contiene `Authorization` |

- [ ] **Step 2: Correr y ver que fallan**

Run: `cargo test --locked --all-features --test temis`
Expected: FAIL (no existe `clio_memory::temis`).

- [ ] **Step 3: Implementar**

`curl_config`, línea por línea: `url = "<url><path>"`, `request = "POST"` (solo con cuerpo), `max-time = 10`, `silent`, `show-error`, `header = "Content-Type: application/json"` (solo con cuerpo), `header = "Authorization: Bearer <token>"` (salvo para `/health`), `data-binary = "<cuerpo>"` (solo con cuerpo), `write-out = "\n%{http_code}"`. En cada valor, `\` pasa a `\\` y `"` a `\"`. `curl_post` corre `curl --config -` con ese texto en la entrada estándar y separa la última línea de la salida como código. `decide` devuelve `Err(format!("Temis respondió {code} ({error_type})"))` con cualquier código que no sea 200. El contador de fallos seguidos es un `AtomicU32`.

- [ ] **Step 4: Correr y ver que pasan**

Run: `cargo test --locked --all-features --test temis`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/temis.rs src/lib.rs tests/temis.rs
git commit -m "feat(temis): a client for Temis's typed decisions; the token never on a command line"
```

---

### Task 2: `clio-memory setup temis`

**Files:**
- Modify: `src/bin/clio-memory/args.rs` (uso y `Command::SetupTemis`), `src/bin/clio-memory/main.rs`, `src/bin/clio-memory/commands.rs` (`doctor`), `src/bin/clio-memory/main.rs:350` (`MemoryFiles.files` suma `temis.json`)
- Test: `tests/cli.rs`

**Interfaces:**
- Consumes: `temis::{Config, check_url, save, remove, load, curl_post, Temis}` (Tarea 1).
- Produces: `Command::SetupTemis { url: Option<String>, model: Option<String>, allow_http: bool, remove: bool, check: bool }`. Órdenes: `setup temis --url URL [--model NOMBRE] [--allow-http]`, `setup temis --check`, `setup temis --remove`.

- [ ] **Step 1: Escribir las pruebas que fallan** (binario real; un `curl` falso en el `PATH` que responde `/health` y `/v1/model` y guarda lo que recibió)

| Prueba | Afirma |
|---|---|
| `setup_temis_saves_what_it_could_verify` | `printf 'un-token-de-prueba-de-32-caracteres\n' \| clio-memory setup temis --url https://h/temis` → código 0, stdout `Temis listo: temis-2.4.0 en https://h/temis`; existe `temis.json` en 0600; ni stdout ni stderr contienen el token |
| `a_token_that_temis_refuses_saves_nothing` | el `curl` falso responde 401 a `/v1/model` → código 1, stderr contiene `authentication_error`; no existe `temis.json` |
| `a_short_token_or_a_plain_http_url_is_refused_before_any_call` | token de 10 caracteres → código 2 y `al menos 24`; `--url http://api.x` → código 2 y `https`; el `curl` falso no fue llamado |
| `the_token_is_never_an_argument` | `setup temis --url … --token X` → código 2 (`unknown option --token`) |
| `remove_and_check` | `--remove` borra el archivo y dice `Temis quitado`; `--check` sin configuración sale con 1 y `Temis no está configurado` |
| `doctor_says_whether_temis_is_set_without_calling_it` | con archivo: `info temis: configurado (https://h/temis)`; sin él: `info temis: no configurado (clio-memory setup temis --url URL)`; el `curl` falso no fue llamado; la línea no contiene el token |

- [ ] **Step 2: Correr y ver que fallan**

Run: `cargo test --locked --all-features --test cli temis`
Expected: FAIL.

- [ ] **Step 3: Implementar**

El token: con la entrada estándar en una terminal, `cliclack::password("Token de Temis")`; si no, la primera línea de la entrada estándar. Orden de `--url`: validar URL y token (errores de uso, código 2) → `health()` → `model()` → `save` → `Temis listo: <modelo> en <url>`. Si `health` o `model` fallan: `error: Temis no respondió bien (<motivo>): no se guardó nada`, código 1. El texto de uso gana tres líneas bajo `setup`, que dicen que el token se lee de la entrada estándar y que solo se acepta `https`.

- [ ] **Step 4: Correr y ver que pasan**

Run: `cargo test --locked --all-features --test cli`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/bin/clio-memory tests/cli.rs
git commit -m "feat(setup): `setup temis` keeps the URL and the token of Temis, once it answered"
```

---

### Task 3: El sueño dobla lo que está contenido

**Files:**
- Modify: `src/sleep/mod.rs:26-71` (rasgo, informe), `:125-243` (`run`), `:296-363` (`Night::sleep`), `:774-815` (`morning_line`), `src/temis.rs` (implementa el rasgo)
- Test: `tests/sleep.rs`

**Interfaces:**
- Consumes: `Temis::{preserved, relation}` (Tarea 1); `near_duplicate_floor` (`src/agent/memory.rs:29`); `marks`, `trust`, `Night::{rank, kept, contradicted, quarantine_one, writer}` (ya en `sleep/mod.rs`).
- Produces:
  - `pub trait sleep::PairJudge { fn preserved(&self, original: &str, merged: &str) -> Option<f32>; fn relation(&self, proposal: &str, other: &str) -> Option<(Relation, f32)>; }`
  - `impl PairJudge for temis::Temis`
  - `pub fn sleep::run_judged(data: &Path, model: &Arc<EmbedModel>, merger: Option<&dyn Merger>, judge: Option<&dyn PairJudge>, opts: &Options) -> Result<Slept, String>`; `sleep::run` no cambia de firma y llama a `run_judged(…, None, …)`
  - `Report.contained: Vec<(i64, i64)>` (el doblado y el que queda) y `Report.judged: u32`, los dos con `#[serde(default)]`
  - constantes `PRESERVED_MIN: f32 = 0.80`, `CONFLICT_VETO: f32 = 0.50`, `MAX_JUDGED: usize = 40`

- [ ] **Step 1: Escribir las pruebas que fallan** (`tests/sleep.rs`, con lo que el archivo ya tiene: `night()`, `open()`, `Night::save`, `pair(&n, &e, texto)` —que guarda `texto` y `texto local`, dos recuerdos con coseno entre 0,87 y 0,97 para el modelo de prueba— y las constantes `DOCKER`, `POSTGRES`, `REDIS`)

Dos ayudantes nuevos en el archivo:
- `Scripted`: un `PairJudge` de prueba. `Scripted::new()` responde `Some(0.0)` a todo `preserved` y `None` a `relation`; `.preserved(original, merged, valor)` fija una respuesta para ese par exacto de textos; `.silent()` hace que `preserved` devuelva `None`; `.relation(rel, conf)` fija la respuesta de `relation`; guarda cada texto que recibió (`.seen() -> Vec<String>`) y cuenta las llamadas a `preserved` (`.asked() -> usize`).
- `Night::report_judged(&self, at: i64, judge: &dyn PairJudge) -> Report`: como `report`, con `run_judged(…, None, Some(judge), …)` y `now: true`.

```rust
#[test]
fn a_memory_another_one_keeps_whole_is_folded_into_it() {
    let n = night();
    let (short, long) = {
        let e = open(&n.data, NOW - DAY);
        pair(&n, &e, DOCKER) // DOCKER, and DOCKER + " local"
    };
    let longer = format!("{DOCKER} local");
    let judge = Scripted::new().preserved(DOCKER, &longer, 0.93).preserved(&longer, DOCKER, 0.10);
    let r = n.report_judged(NOW, &judge);
    assert_eq!(r.contained, vec![(short.0, long.0)]);
    assert_eq!(n.get(short).state, State::Quarantine);
    assert_eq!(n.get(long).state, State::Active);
    let why = format!("sueño: contenido en m{}", long.0);
    assert!(r.quarantined.iter().any(|(id, w)| *id == short.0 && *w == why), "{r:?}");
}
```

| Prueba | Preparación | Afirma |
|---|---|---|
| `below_the_floor_nothing_is_folded` | `pair` de `DOCKER`; `preserved` 0,79 en los dos sentidos | `contained` vacío; los dos activos |
| `when_both_keep_each_other_the_lesser_one_goes` | el primero de la persona (`n.person`), el segundo del agente; 0,90 en los dos sentidos | se va el del agente; el de la persona sigue activo |
| `a_different_number_stops_the_fold` | `"{POSTGRES} puerto 5432"` y `"{POSTGRES} puerto 6432"` (coseno 0,90); `preserved` 0,95 en los dos sentidos | `contained` vacío: las marcas de uno no están en el otro |
| `a_contradiction_stops_the_fold_and_goes_to_review` | `pair` de `REDIS`; `preserved` 0,95 y `.relation(Relation::ConflictsWith, 0.60)` | `contained` vacío; `review` contiene `parecidos, pero Temis ve una contradicción`; `relations(a)` está vacío |
| `the_persons_or_a_pinned_memory_is_never_the_one_folded` | el corto es de la persona; `preserved(corto, largo)` 0,95 y el otro sentido 0,10 | los dos activos |
| `temis_down_leaves_the_pairs_for_later_and_the_night_completes` | `pair` de `DOCKER`; `.silent()` | `r.complete`; `alarms` contiene `Temis no respondió: los parecidos quedan para el próximo sueño`; `contained` vacío; el `pending` de `sleep/state.json` contiene el id del nuevo |
| `at_most_40_pairs_a_sleep` | 45 pares con textos distintos (`pair` de `"{DOCKER} caso{i}"`) | `judge.asked() <= 80`; `sleep/state.json` tiene pendientes |
| `a_text_that_looks_like_a_secret_is_never_sent` | un par de la persona donde los dos textos llevan `sk-` seguido de 24 letras y cifras (el sueño no los manda a cuarentena: solo alarma) | `judge.seen()` está vacío |
| `without_a_judge_the_night_is_as_before` | `pair` de `DOCKER`; `n.report(NOW, None)` | `contained` vacío; nada en cuarentena |
| `the_morning_line_counts_what_was_folded` | un informe con `contained: [(3, 5), (7, 9)]` | la línea contiene `2 contenidas en otras` |
| `an_older_report_without_the_new_fields_still_loads` | un JSON de informe de v0.9 | `serde_json::from_str::<Report>` da `contained` vacío y `judged` 0 |

- [ ] **Step 2: Correr y ver que fallan**

Run: `cargo test --locked --all-features --test sleep judged contained folded temis`
Expected: FAIL (no existe `run_judged`).

- [ ] **Step 3: Implementar**

Paso nuevo `Night::contained`, llamado en `Night::sleep` después de `repeats` y antes de la unión, solo con modelo de vectores, con juez y sin `dry_run` (en `dry_run` no se pregunta a Temis). El algoritmo, que las firmas no determinan:

```text
floor = near_duplicate_floor(modelo en uso) o NEAR
para cada r de `fresh` que no se fue, con vector y sin contradicción abierta:
  para cada vecino n de r (mismo lugar, NEIGHBORS), con floor <= coseno < REPEAT,
      que no se fue, del mismo tipo y la misma confianza (`trust`), sin contradicción abierta:
    si has_secret(r.content) o has_secret(n.content): siguiente vecino
    si preguntados == MAX_JUDGED: deja r en `deferred` y termina el paso
    preguntados += 1; report.judged += 1
    a = judge.preserved(r.content, n.content)     # n guarda todo lo de r
    b = judge.preserved(n.content, r.content)     # r guarda todo lo de n
    si a o b es None: deja r en `deferred`, una alarma
        «Temis no respondió: los parecidos quedan para el próximo sueño», y termina el paso
    candidatos a irse:
        r si a >= PRESERVED_MIN y marks(r) ⊆ marks(n)
        n si b >= PRESERVED_MIN y marks(n) ⊆ marks(r)
    si los dos: se va el de menor `rank`; si ninguno: siguiente vecino
    si el que se iría es `kept` (de la persona o fijado): siguiente vecino
    si judge.relation(nuevo.content, viejo.content) es (ConflictsWith, c) con c >= CONFLICT_VETO:
        report.review(«m{menor} y m{mayor}: parecidos, pero Temis ve una contradicción: revísalos»)
        siguiente vecino                              # nuevo = el de id mayor
    si no queda cupo de cuarentenas (`left`): deja r en `deferred` y termina el paso
    quarantine_one(se va, «sueño: contenido en m{queda}»); copy_relations(se va → queda)
    report.contained.push((se va, queda)); gone.insert(se va); left -= 1
    si el que se fue es r: sale del bucle de vecinos
```

Los textos se cortan a 4000 caracteres dentro de `Temis`, no aquí. `morning_line`: tras las cuarentenas y antes de las uniones, `1 contenida en otra` o `N contenidas en otras`; las cuarentenas con razón `contenido en` no se listan además en «a cuarentena».

- [ ] **Step 4: Correr y ver que pasan**

Run: `cargo test --locked --all-features --test sleep --test sleep_engine --test sleep_signals`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/sleep/mod.rs src/temis.rs tests/sleep.rs
git commit -m "feat(sleep): with Temis, a memory another one keeps whole is folded into it"
```

---

### Task 4: `clio-memory sleep` usa a Temis cuando está configurado

**Files:**
- Modify: `src/bin/clio-memory/main.rs:404-462` (`sleep`), `src/temis.rs` (una función para anfitriones), `README.md`, `docs/motor.md`
- Test: `tests/cli.rs`

**Interfaces:**
- Produces: `pub fn temis::from_data(data: &Path) -> Option<Temis>` — la configuración (`load` con el entorno real) y `curl_post("curl")`; es lo que Clio usará para pasar el mismo juez a `run_judged`.

- [ ] **Step 1: Escribir las pruebas que fallan** (binario real; `temis.json` en la carpeta de datos; un `curl` falso que responde a `/v1/decide` y anota cada entrada)

| Prueba | Afirma |
|---|---|
| `sleep_asks_temis_when_it_is_set` | con un par casi igual sembrado (el modelo `model2vec` de prueba o `--model`, como las demás pruebas de `sleep` de `tests/cli.rs`) y el `curl` falso respondiendo `noul: 0.95` y luego `choice: "compatible"`: `clio-memory sleep --now` imprime `1 contenida en otra`; el `curl` falso recibió al menos una entrada con `/v1/decide` |
| `sleep_report_never_holds_the_token` | tras lo anterior, ningún archivo de `<datos>/sleep/` contiene el token, ni stdout ni stderr |
| `sleep_without_temis_is_silent_about_it` | sin `temis.json`: la salida no menciona a Temis y el `curl` falso no fue llamado |
| `dry_run_never_calls_temis` | `sleep --now --dry-run` con `temis.json`: el `curl` falso no fue llamado |

Si `tests/cli.rs` no tiene hoy cómo sembrar un par casi igual sin un modelo real, la primera prueba se escribe en `tests/sleep.rs` contra `run_judged` con `temis::Temis::new(cfg, post_falso)` y aquí se deja solo la comprobación de que el binario construye el juez (`sleep --now` con `temis.json` y sin pares termina con código 0).

- [ ] **Step 2: Correr y ver que fallan**

Run: `cargo test --locked --all-features --test cli sleep`
Expected: FAIL en las nuevas.

- [ ] **Step 3: Implementar**

`main.rs::sleep` llama a `run_judged(data, &model, None, temis::from_data(data).as_ref().map(|t| t as &dyn PairJudge), &opts)`. `README.md` y `docs/motor.md`: qué pregunta el sueño a Temis, qué hace con la respuesta, que sin Temis nada cambia, que lo que se envía es el contenido de los pares parecidos (nunca lo que parece un secreto) y a qué servidor (el que la persona configuró), y los números de `bench/temis/results.md`.

- [ ] **Step 4: Correr y ver que pasan**

Run: `cargo test --locked --all-features --no-fail-fast`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/bin/clio-memory/main.rs src/temis.rs README.md docs/motor.md tests
git commit -m "feat(sleep): `clio-memory sleep` asks Temis when the person set it up"
```

---

### Task 5: Versión 0.12.0 y la comprobación con el Temis real

**Files:**
- Modify: `Cargo.toml:5`, `Cargo.lock`, `bench/temis/results.md`

- [ ] **Step 1: Comprobar con el servicio real** (solo si en el entorno están `CLIO_MEMORY_TEMIS_URL` y `CLIO_MEMORY_TEMIS_TOKEN`; si no, se anota en el PR como pendiente para Iván y se sigue)

```bash
python3 bench/temis/run.py > /tmp/temis.md          # la misma medición de results.md, para ver que el servicio no cambió
T=$(mktemp -d) && sqlite3 -readonly ~/.local/share/clio/memory.db ".backup $T/memory.db"
cargo run --locked -q --release --features model2vec,granite -- --db $T/memory.db sleep --now --dry-run
```

Expected: la tabla de `bench/temis/run.py` da, con umbral 0,8, «8 de 12» paráfrasis y «0 de 8» parecidas (o se actualiza `results.md` con lo que dé y se revisa `PRESERVED_MIN` con Iván antes de fusionar). El `--dry-run` sobre la copia no llama a Temis y no cambia nada.

- [ ] **Step 2: Subir la versión y comprobar como el CI**

`version = "0.12.0"`. Run: las cuatro órdenes de comprobación. Expected: todo en verde.

- [ ] **Step 3: Commit y PR**

```bash
git add Cargo.toml Cargo.lock bench/temis/results.md
git commit -m "chore: release v0.12.0"
gh pr create --title "v0.12.0: el sueño decide con Temis" --body "…"
```

El cuerpo del PR dice cómo se activa (`clio-memory setup temis --url https://api.imontes.dev/temis`, con el token por la entrada estándar), y lo que queda para Clio: pasar `clio_memory::temis::from_data(&datos)` a `run_judged` en `clio memory sleep`, y, si quiere que sus propias decisiones (guardia, lugar, relación) las tome Temis en vez de Jev, admitir la URL de Temis en su adaptador.
