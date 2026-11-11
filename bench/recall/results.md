# Evaluación del recuerdo (2026-11-09)

¿Encuentra la memoria lo que se le pregunta, se calla cuando no lo tiene y reconoce lo que ya sabe? Datos inventados, en español:
- `memories.jsonl`: 120 recuerdos, como los guardaría un agente: el perfil y las preferencias de una persona (26), tres proyectos (`tienda`, `clima`, `app-citas`: 66) y lecciones aprendidas en ellos (28), con título y categoría.
- `queries.jsonl`: 159 preguntas.
  - 95 paráfrasis con otras palabras.
  - 24 que mezclan inglés y español («scope» frente a «ámbito», «rate limiting», «feature flag»).
  - 8 que se responden desde otro proyecto.
  - 20 sin respuesta en el banco, pero del mismo mundo («¿qué versión de Node usa la tienda?»).
  - 12 de fuera («capital de Australia»).
- `duplicates.jsonl`: 28 guardados de algo que ya está.
  - 8 repeticiones exactas.
  - 12 paráfrasis del mismo hecho.
  - 8 que solo se parecen, sin decir lo mismo («Wompi cobra una comisión del 2,65 %» al lado de «los pagos van por Wompi»).

Se mide lo que ve el agente: `memory_search` (`Memory::search_all`, los 5 primeros, sobre todos los lugares). Equipo: servidor ARM de 4 núcleos, sin GPU.

```sh
cargo test --test recall_eval                    # solo texto, como en el CI
CLIO_MEMORY_MODEL_DIR=~/.cache/clio-models/granite-embedding-311m-multilingual-r2 \
  CLIO_MEMORY_EVAL_OUT=informe.json \
  cargo test --release --features granite --test recall_eval -- --ignored --nocapture
```

Con Granite tarda unos 4 minutos, casi todo en vectorizar.

## 1. Lo que encontró: con Granite, el recuerdo nunca se callaba

El umbral de relevancia de los vectores (`min_vector_score` 0,30) se fijó con el modelo estático anterior (potion), con el que un texto sin relación queda en 0,11 a 0,20. **Granite pone cualquier par de textos en español por encima de 0,78**, así que con 0,30 todo vector cuenta como relevante: el recuerdo devolvía cinco resultados a cualquier pregunta, también a «receta de arepas de choclo».

Mejor coseno de cada pregunta, solo por vectores:

| Preguntas | mín. | p10 | p50 | p90 | máx. |
|---|---|---|---|---|---|
| con respuesta (127) | 0,848 | 0,865 | 0,908 | 0,939 | 0,984 |
| sin respuesta (32) | 0,778 | 0,784 | 0,852 | 0,891 | 0,903 |

Las dos se solapan entre 0,85 y 0,90: ningún umbral separa todo, y lo que no separa el vector lo tiene que separar el texto.

## 2. El barrido

El umbral de texto en 0,5, que es el que está (0,34 da un punto más de respuestas y unos menos de silencio; 0,67 al revés):

| vector ≥ | con respuesta: la correcta entre los 5 | con respuesta: se calló | sin respuesta: se calló |
|---|---|---|---|
| 0,30 (antes) | 91 % | 0 % | 0 % |
| 0,80 | 93 % | 0 % | 19 % |
| 0,82 | 96 % | 0 % | 31 % |
| **0,84 (ahora)** | **97 %** | **0 %** | **38 %** |
| 0,85 | 96 % | 0 % | 41 % |
| 0,86 | 92 % | 4 % | 47 % |
| 0,88 | 81 % | 11 % | 59 % |
| 0,90 | 73 % | 20 % | 72 % |

Subir el umbral a 0,84 **mejora las dos cosas a la vez**: quita del medio vectores que no tienen que ver, así que los aciertos por texto suben de puesto (97 % contra 91 %), y empieza a callarse cuando no hay nada. Desde 0,86 el silencio sigue subiendo, pero se empiezan a perder respuestas. Entre 0,82 y 0,85 los números casi no cambian: es una meseta, no un pico de este banco.

## 3. Con el umbral nuevo (el valor por defecto de Granite desde este cambio)

| Preguntas | n | 1.º | entre los 5 | recall@5 | MRR@5 | se calló |
|---|---|---|---|---|---|---|
| paráfrasis | 95 | 75 % | 97 % | 93 % | 0,846 | 0 % |
| inglés y español | 24 | 79 % | 100 % | 100 % | 0,875 | 0 % |
| desde otro proyecto | 8 | 62 % | 88 % | 88 % | 0,729 | 0 % |
| sin respuesta, del mismo mundo | 20 | — | — | — | — | 5 % |
| de fuera | 12 | — | — | — | — | 92 % |
| **con respuesta** | 127 | **75 %** (antes 59 %) | **97 %** (91 %) | 94 % (88 %) | **0,844** (0,688) | 0 % |
| **sin respuesta** | 32 | — | — | — | — | **38 %** (0 %) |

- El inglés mezclado ya no es un problema con Granite: 24 de 24.
- Lo que no se calla es lo que no tiene respuesta pero se parece a algo que sí (19 de las 20 preguntas «del mismo mundo» traen algo). Eso no lo arregla un umbral; es para el modelo que lee los resultados, que los recibe como datos con su lugar.
- **Cuidado:** el umbral se eligió mirando este mismo banco. Las paráfrasis las escribió la misma mano que los recuerdos. Hay que volver a medir con uso real (la tarea 341 de Clio) antes de darlo por bueno.

## 4. Solo texto (sin modelo, como en el CI)

| | entre los 5 | se calló |
|---|---|---|
| con respuesta | 60 % | 31 % |
| sin respuesta | — | 72 % |

La prueba del CI exige al menos 55 % y 65 %: si un cambio en la búsqueda por texto baja de ahí, falla.

## 5. Guardar lo que ya está

- Las 8 repeticiones exactas dieron NOOP: no se duplica nada.
- Ninguna de las 20 paráfrasis o parecidas dio NOOP: se guardan como recuerdos nuevos. Lo que avisa es el aviso de «muy parecida a mN» al guardar, y lo que une es el sueño (≥ 0,97).
- Ningún guardado quedó pendiente de revisión (0 de 148): todo entra activo, como dice S-61.

El aviso de «muy parecida» usa 0,87 con Granite (medido en `bench/relations`, donde las contradicciones quedan entre 0,880 y 0,937):

| piso | paráfrasis con su gemela avisada | parecidas avisadas sin serlo |
|---|---|---|
| 0,86 | 12 de 12 | 7 de 8 |
| 0,87 (hoy) | 12 de 12 | 5 de 8 |
| 0,88 | 12 de 12 | 4 de 8 |
| 0,90 | 12 de 12 | 1 de 8 |
| 0,92 | 11 de 12 | 0 de 8 |

Con 0,90 el aviso acertaría lo mismo con muchas menos falsas alarmas, pero dejaría de avisar de una contradicción por debajo de 0,90 (en `bench/relations` hay una en 0,880). No se cambia: avisar de más solo cuesta una línea, y callar una contradicción cuesta un recuerdo falso. Queda para decidir con más datos.

## 5. Con el modelo cuantizado (2026-11-11, desde la 0.9)

Desde la 0.9, `setup` instala Granite cuantizado (`model_quint8_avx2.onnx`) y corre en el procesador (`bench/embeddings/results.md`). Sus vectores son otro espacio (`granite-embedding-311m-multilingual-r2-q8@384`), así que tiene sus umbrales. Medido igual, en el procesador de un equipo de 16 hilos, en 128 s.

Mejor coseno de cada pregunta, solo por vectores: todo queda alrededor de 0,01 más abajo que con el completo.

| Preguntas | mín. | p10 | p50 | p90 | máx. |
|---|---|---|---|---|---|
| con respuesta (127) | 0,838 | 0,859 | 0,897 | 0,932 | 0,982 |
| sin respuesta (32) | 0,779 | 0,790 | 0,847 | 0,891 | 0,895 |

El barrido, con el umbral de texto en 0,5:

| vector ≥ | con respuesta: la correcta entre los 5 | con respuesta: se calló | sin respuesta: se calló |
|---|---|---|---|
| 0,30 | 91 % | 0 % | 0 % |
| 0,82 | 97 % | 0 % | 31 % |
| **0,83 (su umbral)** | **97 %** | **0 %** | **31 %** |
| 0,84 | 95 % | 0 % | 34 % |
| 0,85 | 93 % | 0 % | 41 % |
| 0,86 | 86 % | 6 % | 50 % |

Con 0,83 encuentra lo mismo que el completo con 0,84 (97 % entre los 5) y se calla algo menos (31 % contra 38 %). Su aviso de «muy parecida» es 0,86 (medido en `bench/relations`, donde sus tres contradicciones quedan entre 0,863 y 0,939); aquí avisa 12 de 12 paráfrasis y 6 de 8 parecidas sin serlo (el completo con 0,87: 12 de 12 y 5 de 8).
