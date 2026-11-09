# Escala de la búsqueda por vectores (2026-11-09, tarea 305 de Clio)

¿Hasta cuántos recuerdos alcanza la búsqueda plana (comparar la pregunta con todos los vectores) antes de necesitar un índice? `examples/vector_scale.rs` arma una memoria de N recuerdos sintéticos con vectores de 384 dimensiones (los de Granite), hechos por un modelo falso que reparte las palabras al azar. Así la prueba mide el motor y no el modelo. Después mide:
- abrir el archivo;
- el primer recuerdo y 50 más en caliente;
- los mismos 50 solo por texto;
- la memoria del proceso.

Equipo: máquina virtual aarch64 (QEMU), 4 núcleos, sin GPU. Se corre con `cargo run --release --example vector_scale -- 5000 50000 500000`.

| recuerdos | armar (s) | archivo (MB) | abrir (ms) | 1.er recuerdo (ms) | en caliente p50 / p95 (ms) | solo texto p50 (ms) | RSS después (MB) | RSS que suman los vectores (MB) |
|---|---|---|---|---|---|---|---|---|
| 5 000 | 30.4 | 27 | 0.4 | 23.9 | 12.2 / 12.6 | 9.3 | 43 | 35 |
| 50 000 | 337.0 | 270 | 0.4 | 206.9 | 97.7 / 98.8 | 71.0 | 338 | 330 |
| 500 000 | 5056.6 | 2700 | 0.4 | 2146.7 | 2192.6 / 2210.2 | 693.5 | 220 | 212 |

## Qué dice
- **Hasta 50 000, todo crece en línea con N.** Un recuerdo en caliente cuesta unos 2 ms por cada mil recuerdos, y el primero, unos 4 ms por cada mil, porque carga los vectores.
- **A 500 000 se rompe.** Un recuerdo tarda 2,2 s, unas 22 veces lo de 50 000, y el armado tardó 84 minutos. La parte de vectores pasa de unos 27 ms a unos 1,5 s, unas 55 veces más.
- La memoria del proceso a 500 000 sale *menor* que a 50 000 (212 MB frente a 330 MB), aunque los vectores en bruto ocupan 730 MB. Seguramente el motor no guarda todos los vectores decodificados en memoria a ese tamaño y los relee de SQLite en cada búsqueda. Eso explicaría el salto, pero no se comprobó.
- **La mayor parte del tiempo no es el vector.** Por texto solo, el mismo recuerdo cuesta unos 70 % del total. El corpus sintético es el peor caso para el texto: todos los recuerdos comparten palabras («usa», «número»), así que la búsqueda de texto ordena casi todo el archivo. Con texto real, el texto pesa menos. Los vectores suman unos 0.5 ms por cada mil recuerdos.
- **La memoria del proceso** sube unos 7 KB por recuerdo, más que los 1.5 KB de un vector de 384 `f32`. No se investigó a qué se debe (caché de SQLite, filas cargadas).
- Una persona con un agente guarda decenas de recuerdos al día. 5 000 recuerdos son años de uso, y ahí un recuerdo tarda 12 ms. Con 50 000 tarda 0.1 s, tolerable para una herramienta que el agente llama pocas veces por sesión.

## Decisión para la tarea 305
La búsqueda plana basta hasta unos 50 000 recuerdos en esta máquina, que es lenta. A 500 000 no basta (2,2 s), pero una persona no llega ahí en años. No hace falta un índice de vectores (HNSW, sqlite-vec) en la fase 2. Conviene revisarlo si una memoria real pasa de unos 20 000 recuerdos. Antes de un índice, habría que mirar por qué se dispara a 500 000 y el costo del texto.
