# Escrituras concurrentes (2026-11-09, D11 y tarea 342 de Clio)

¿Qué pasa cuando varios procesos guardan a la vez en la misma memoria? Es el caso de varias sesiones de agente abiertas, cada una con su servidor MCP. `examples/concurrent_writes.rs` lanza K procesos que guardan al mismo tiempo, cada uno como lo hace una llamada MCP:
1. abre para escribir y espera el candado de escritor hasta 2 s (8 intentos de 250 ms);
2. guarda;
3. suelta el candado.

Uno de cada diez guardados es el mismo texto en todos los procesos, como un dato que dos agentes aprenden a la vez. Al final se relee el archivo.

Equipo: máquina virtual aarch64 (QEMU), 4 núcleos. Se corre con `cargo run --release --example concurrent_writes -- K N`.

| procesos × guardados | tiempo (s) | guardados | de ellos NOOP | se rindieron | faltan después | datos compartidos (copias / distintos) | guardar p50 / p95 / p99 (ms) |
|---|---|---|---|---|---|---|---|
| 2 × 100 | 2.8 | 200 | 10 | 0 | 0 | 10 / 10 | 12.6 / 15.0 / 22.8 |
| 8 × 50 | 5.9 | 393 | 30 | 7 | 0 | 5 / 5 | 12.7 / 15.4 / 763.8 |
| 16 × 50 | 11.9 | 762 | 62 | 38 | 0 | 5 / 5 | 12.9 / 15.7 / 765.9 |

En las corridas de 8 y 16 procesos, cada guardado que se rindió respondió «otro proceso está escribiendo la memoria; reintenta», tras unos 2 s.

## Qué dice
- **No se pierde nada ni se duplica nada.** Cada guardado que respondió «guardado» está en el archivo. Cada dato compartido quedó una sola vez: el primero lo agrega y los demás reciben NOOP («ya se sabía»).
- **Lo que falla, falla a la vista.** Con 8 o más procesos que guardan sin pausa, algunos guardados se rinden a los 2 s y el agente recibe un error claro que puede reintentar. Es un 2 % con 8 procesos y un 5 % con 16. El uso real (pocas sesiones, guardados espaciados) está lejos de esa carga.
- Un guardado normal tarda unos 13 ms. El p99 de ~0.76 s es la espera del candado.

## Sobre D11
D11 pedía comparar «con un gestor único» y «sin él». La segunda rama no se puede armar: el motor no deja entrar a un segundo escritor (el candado de escritor y SQLite en modo WAL). Así que «sin gestor único» ya significa en la práctica «cada proceso espera su turno». La medición dice que esa espera alcanza. Un proceso gestor (un demonio que reciba todos los guardados) solo evitaría los rendidos bajo carga extrema, y no se justifica en la fase 2. Este supuesto de lectura queda para que la persona lo confirme.
