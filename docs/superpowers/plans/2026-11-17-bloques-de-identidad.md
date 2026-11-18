# Bloques de identidad: los agentes los leen primero y no los pueden cambiar — plan

> **Para agentes:** se ejecuta tarea por tarea, con TDD y un commit por tarea. Pasos con casillas.

**Objetivo:** que lo que la persona puso en los bloques fijos (`identidad`, `reglas`,
`preferencias`) llegue primero a cualquier agente en `memory_context`, y que ningún agente pueda
cambiar un recuerdo que un bloque muestra.

**Arquitectura:** una sola compuerta en el motor (`Engine`) para todo cambio que no firma una
persona, con un error propio; `Memory::context` pone primero los bloques; el texto de la skill le
dice al agente qué son. Sin herramientas nuevas, sin categorías nuevas y sin cambio de esquema.

**Stack:** Rust 1.95, este repositorio (`clio-memory`).

**Spec:** `/home/imontes/Projects/personal/agent/clio/docs/specs/2026-11-17-clio-asistente-autonomo-design.md`
(sección «1. Recuerdos primordiales y presentación»). El diseño hablaba de tipos nuevos; el motor ya
tiene los bloques fijos, que hacen lo mismo, así que este plan los usa.

## Restricciones globales

- Texto exacto del rechazo: `ese recuerdo está en un bloque fijo de la persona: solo ella lo cambia`.
- Un actor `Person` conserva todos sus poderes sobre los bloques y sus recuerdos.
- Vale para cualquier bloque del ámbito, no solo los tres de fábrica (un anfitrión puede crear
  otro, por ejemplo `vigilancia`).
- En `memory_context` solo salen como bloque los tres de fábrica, en este orden: `identidad`,
  `reglas`, `preferencias`.
- Versión `0.13.0` en `Cargo.toml` y `Cargo.lock`; nota en `docs/releases.md` (en español, arriba).
- Si cambia `skill/SKILL.md`: el hash anterior entra en `KNOWN_SKILLS` y se pone el nuevo en
  `CURRENT_SKILL_SHA` (`src/setup/skill.rs`).
- Compuertas: `cargo fmt --all --check`; `cargo clippy --locked --all-targets --all-features -- -D warnings`;
  `cargo clippy --locked --lib --no-default-features --features model2vec,granite,telemetry -- -D warnings`;
  `cargo test --locked --all-features --no-fail-fast`.
- No se hace push ni etiqueta.

## Foco de revisión

1. **Cambio por tema.** Un agente guarda un recuerdo nuevo con el mismo tema que uno de un bloque:
   hoy el motor lo trata como corrección y reemplaza al del bloque. Debe rechazarse y el del bloque
   queda igual. → prueba en Tarea 1.
2. **Unir.** `memory_consolidate` con un recuerdo de bloque entre las fuentes: se rechaza y no se
   olvida ninguna de las otras fuentes. → prueba en Tarea 1.
3. **La versión vigente.** Si la persona corrigió un recuerdo del bloque, lo protegido es la versión
   que el bloque muestra hoy (y también el id que el bloque lista). → prueba en Tarea 1.
4. **El mensaje no se pierde.** `Session::change` (`src/tools.rs`) convierte casi todo error de
   olvidar o fijar en «ese recuerdo no existe o no se puede leer»: el rechazo nuevo debe llegar
   tal cual al agente. → prueba en Tarea 1.
5. **El tope de 4096 bytes** de una respuesta: los bloques van primero, así que nunca son lo que se
   corta; un bloque vacío no imprime nada. → prueba en Tarea 2.

---

### Task 1: Lo que un bloque muestra no lo cambia un agente

**Archivos:**
- Modificar: `src/engine.rs` (`propose_inner` ~1253-1470, `replace`, `forget` ~1748, `in_block` 1123,
  la unión que usa `memory_consolidate`)
- Modificar: donde vive `enum Error` (el de `Error::NotAllowed`) y `explain` en `src/agent/memory.rs:65-80`
- Modificar: `src/tools.rs` (`change()` ~607: que este rechazo no se enmascare)
- Test: `tests/agent_authority.rs`

**Interfaces:**
- Produce: `Error::HeldByBlock`, que `explain` traduce al texto exacto de las restricciones.
- Regla: si quien firma no es `ActorKind::Person` y el recuerdo que se va a reemplazar, olvidar o
  usar como fuente de una unión es uno que un bloque de su ámbito lista o muestra hoy (el listado
  seguido hasta su versión vigente), la operación falla con `HeldByBlock` y no cambia nada.

- [ ] **Paso 1: pruebas que fallan** en `tests/agent_authority.rs` (con los accesorios que ya usa
  ese archivo):
  - `an_agent_cannot_replace_what_a_block_shows`: la persona guarda `m` y hace
    `block_set(identidad, [m])`; `Memory::update(m, …)` del agente da `Err` con el texto exacto;
    `block_current` sigue mostrando el texto original.
  - `an_agent_cannot_forget_what_a_block_shows`.
  - `an_agent_cannot_replace_a_block_member_by_topic`: el agente propone un recuerdo con el mismo
    tema que `m` → `Err`; `m` sigue activo e igual.
  - `an_agent_cannot_consolidate_a_block_member`: `memory_consolidate` con `[m, otro]` → `Err`;
    `otro` sigue activo.
  - `the_current_version_of_a_block_member_is_protected_too`: la persona reemplaza `m` por `m2` (el
    bloque sigue a `m2`); `Memory::update(m2, …)` del agente → `Err`.
  - `the_person_still_changes_her_blocks`: la persona reemplaza y olvida un recuerdo de bloque.
  - `the_refusal_reaches_the_agent_as_written`: `Session::call("memory_update", …)` y
    `Session::call("memory_forget", …)` devuelven exactamente el texto del rechazo.
  - `an_agent_replaces_a_persons_memory_by_topic` (ya existe) sigue pasando: fuera de un bloque no
    cambia nada.
- [ ] **Paso 2:** `cargo test --all-features --test agent_authority` → fallan las nuevas.
- [ ] **Paso 3:** implementar la compuerta en el motor (un solo lugar que consulten reemplazar,
  olvidar y unir) y dejar pasar el mensaje en `change()`.
- [ ] **Paso 4:** las compuertas de las restricciones globales, completas.
- [ ] **Paso 5:** commit `feat(engine): what a fixed block shows is the person's to change`.

### Task 2: Los bloques van primero en el contexto, y el agente sabe qué son

**Archivos:**
- Modificar: `src/agent/memory.rs` (`context()` 945-985; nueva `block`)
- Modificar: `src/tools.rs` (la respuesta de `memory_context`, ~430 y `render`)
- Modificar: `skill/SKILL.md`, `src/setup/skill.rs` (`KNOWN_SKILLS`, `CURRENT_SKILL_SHA`),
  `src/mcp.rs` (`INSTRUCTIONS`), `README.md` (sección de herramientas), `docs/motor.md`,
  `docs/releases.md`, `Cargo.toml`, `Cargo.lock`
- Test: `tests/tools.rs` (o el archivo donde ya se prueba `memory_context`)

**Interfaces:**
- Produce: `pub fn block(&self, name: &str) -> Res<Vec<MemoryRow>>` en `Memory`: lo que el bloque
  `name` del ámbito de la persona muestra hoy, en su orden; vacío si no existe. Funciona con un
  enlace de solo lectura.
- `Memory::context()` devuelve primero las filas de `identidad`, luego `reglas`, luego
  `preferencias`, y después lo de hoy, sin repetir ids.
- En la respuesta de `memory_context`, cada bloque no vacío va bajo una línea `## identidad`,
  `## reglas` o `## preferencias`, antes de todo lo demás.
- Texto que se agrega a `skill/SKILL.md`, en «When to read it», tal cual:

  > - **What `memory_context` shows first, under `## identidad` and `## reglas`, is who you are for
  >   this person and how you work: adopt it.** The person set it. `## preferencias` is who the
  >   person is. You cannot change these entries; only the person does.

  El mismo sentido, en una frase, en `INSTRUCTIONS` de `src/mcp.rs`.

- [ ] **Paso 1: pruebas que fallan:**
  - `the_context_starts_with_the_fixed_blocks`: la persona llena `identidad` con A y `reglas` con
    B, y hay diez recuerdos recientes del agente; en la respuesta de `memory_context` el orden de
    aparición es `## identidad`, A, `## reglas`, B y luego el primero de los recientes.
  - `an_empty_block_prints_nothing`: sin bloques llenos, la respuesta no contiene `## `.
  - `only_the_three_blocks_are_headers`: un bloque `vigilancia` con un recuerdo no produce
    `## vigilancia`.
  - `the_blocks_survive_the_answer_cap`: con bloques llenos y muchos recuerdos largos, la respuesta
    (≤ 4096 bytes) contiene A y B completos.
- [ ] **Paso 2:** correrlas → fallan.
- [ ] **Paso 3:** implementar; textos; hash de la skill; versión `0.13.0`; nota de versión:
  «v0.13.0: los bloques fijos de la persona van primero en `memory_context`, y un agente ya no puede
  corregir, olvidar ni unir un recuerdo que un bloque muestra».
- [ ] **Paso 4:** las compuertas de las restricciones globales, completas (incluida
  `the_skill_is_the_pinned_one`).
- [ ] **Paso 5:** commit `feat: the fixed blocks come first in the context; v0.13.0`.
