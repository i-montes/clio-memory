# Prueba de relaciones entre recuerdos (2026-11-04)

¿Cómo se consigue que el modelo pase de un recuerdo a los que tienen que ver con él, hasta tener lo que necesita? Datos inventados:
- los 60 recuerdos de `bench/embeddings`, más 4 que contradicen o acotan a otros (`extra.jsonl`);
- `pairs.jsonl`: 50 pares que una persona enlazaría, con su tipo (3 contradicciones);
- `tasks.jsonl`: 15 preguntas que necesitan 2 a 4 recuerdos a la vez.

Vectores: Granite 311M a 384 dimensiones. Medida: de lo que cada pregunta necesita, cuánto se alcanza leyendo como mucho K recuerdos.

Se corre con `uv run --python 3.12 --with sentence-transformers bench/relations/run.py`. El juez local se corre con `llm_judge.py`, y el reranker con `run.py --reranker`.

## 1. Relaciones guardadas, sin juez

| Enlaces | Precisión | Cobertura de los pares buenos | Lo alcanzado con K=4 | K=6 | Tareas completas con K=6 |
|---|---|---|---|---|---|
| Solo búsqueda (sin relaciones) | – | – | 69 % | 72 % | 33 % |
| El más cercano por vectores | 45 % | 42 % | 75 % | 79 % | 53 % |
| Los 2 más cercanos | 34 % | 62 % | 69 % | 83 % | 60 % |
| Mutuos entre 3 (CSLS) + el más cercano | 35 % | 54 % | 73 % | 81 % | 67 % |
| Vectores + palabras raras compartidas | 28–32 % | 48–64 % | 67–71 % | 77–79 % | 53 % |
| Las 50 marcadas a mano | 100 % | 100 % | 90 % | 98 % | 93 % |

- Con Granite casi todo parece parecido: los pares buenos tienen una similitud mediana de 0,825 y el resto de 0,769. Un umbral absoluto no separa unos de otros.
- Un tercio de los pares buenos no está ni entre los 3 vecinos más cercanos. Son relaciones por algo compartido (un procedimiento que usa la base, los correos de los pedidos), no por parecido.
- Las relaciones guardadas sin juez ayudan poco, de 72 % a 79–83 %. Las buenas llevan casi al 98 %: el valor está en juzgar bien.

## 2. Jueces automáticos sobre los 5 vecinos

Los 5 vecinos de cada recuerdo dan 216 pares candidatos, de los que 37 están entre los 50 buenos.

| Juez | Precisión | Cobertura | K=6 | Completas con K=6 |
|---|---|---|---|---|
| Juez perfecto (el límite) | 76 % | 74 % | 90 % | 73 % |
| Reranker bge-reranker-v2-m3 (local) | 29–44 % | 36–58 % | 72–79 % | 47–53 % |
| Qwen3-1.7B (local, pregunta cerrada) | 17 % | 74 % | 73 % | 40 % |

- El reranker no mejora el vecino más cercano.
- Qwen3-1.7B dice que casi todo está relacionado y no reconoce ninguna contradicción: las tres le parecen «un caso más concreto».
- Un juez local pequeño no sirve. Uno grande pesaría varios GB más.

## 3. El lector como juez: ver los vecinos de lo que abre

Aquí no se guarda ninguna relación. Cada recuerdo que el lector abre trae sus N vecinos más cercanos (id y comienzo del texto), y el lector abre los que le sirven para su pregunta.

| Lector | Vecinos a la vista | Lo alcanzado con K=4 | K=6 | Completas con K=6 | Lecturas |
|---|---|---|---|---|---|
| Ideal (abre justo lo que hace falta) | 3 | 85 % | 89 % | 73 % | 6 |
| Ideal | 5 | 96 % | 98 % | 93 % | 6 |
| Ideal | 8 | 98 % | 100 % | 100 % | 6 |
| Torpe (abre el más parecido a la pregunta) | 5 | 67 % | 76 % | 40 % | 6 |
| **Real: un agente Claude, a ciegas** | 5 | – | **89 %** | **73 %** | **3,6 de media** |

- El agente real vio solo la herramienta (2 resultados de búsqueda y luego abrir por id, con 5 vecinos por recuerdo), sin saber qué contaba.
- Alcanzó el 89 % con la mitad de lecturas que la búsqueda sola. Por su cuenta leyó también los recuerdos que contradicen a otros (MySQL frente a PostgreSQL, Render frente a Hetzner).
- Lo que no alcanzó es discutible: el puerto 8080 para desplegar, o la regla de las ramas para publicar una versión.
- El agente rompió dos veces la regla de abrir solo lo visto; esas dos lecturas no se cuentan.

## 4. Contradicciones

- Las 3 contradicciones son en los tres casos el vecino más cercano del otro recuerdo, con similitud de 0,88 a 0,94.
- Por encima de 0,88 hay 5 pares en todo el conjunto: las 3 contradicciones y 2 recuerdos muy relacionados (TDD con pruebas de mutación, respuestas cortas con tablas cortas). Los cinco valen la pena verlos al guardar.

## 5. El umbral, medido con Granite en Rust (ONNX Runtime)

`examples/relations_floor.rs` da los mismos pares que Python:

| Similitud | Par | Qué son |
|---|---|---|
| 0,937 | m2–m62 | contradicción |
| 0,930 | m5–m61 | contradicción |
| 0,892 | m2–m58, m4–m20 | muy relacionadas |
| 0,8801 | m8–m63 | contradicción |
| 0,879 – 0,871 | m15–m33, m2–m40, m58–m62, m31–m64, m18–m19 | relacionadas |

La contradicción más baja está a 0,8801, justo en el borde de 0,88. Por eso clio-memory avisa desde **0,87**. Lo que queda entre 0,87 y 0,88 también vale la pena verlo al guardar, y un aviso de más solo cuesta una línea en la respuesta.

## 6. Jev (TypeSafe System One) como juez, en Clio

Los mismos 217 pares candidatos (los 5 vecinos de cada recuerdo con Granite en Rust). El recuerdo con el número mayor es el que se guarda. Lo corre la prueba ignorada `jev_relations_on_the_bench` de Clio, con el baúl abierto.

| Jev dice | Veces | Aciertos (precisión) |
|---|---|---|
| contradice | 4 | 3 (75 %) |
| relacionada | 10 | 1 (10 %) |
| caso concreto | 3 | 0 |
| compatible | 2 | 0 |
| ninguna | 198 | – |

| Marcado a mano | Entre los candidatos | Jev lo dice (cobertura) |
|---|---|---|
| contradice | 3 | 3 (100 %) |
| relacionada | 30 | 1 (3 %) |
| compatible | 3 | 0 |
| caso concreto | 2 | 0 |

- Las 3 contradicciones las da con 0,95 a 0,99 de confianza. La falsa (m58 «tabla corta» frente a m62 «respuestas largas») la da con 0,77.
- Con un piso de 0,9 quedan las 3 contradicciones y ninguna falsa.
- Para los otros tipos Jev casi siempre dice «ninguna», y cuando dice otra cosa casi nunca coincide con lo marcado.
- Por eso Clio deja que Jev anote solo «contradice», con 0,9 o más, entre lo nuevo y sus 3 vecinos más cercanos. Lo demás lo resuelven los vecinos que ve el modelo.

## Conclusión

- Guardar relaciones automáticas sin un juez que entienda da poco.
- Lo que sí funciona es mostrar, con cada recuerdo, sus vecinos más cercanos y dejar que el modelo que lee elija cuál seguir. Ese modelo ya entiende la pregunta, así que no hace falta otro juez.
- Ningún recuerdo queda suelto: todo recuerdo con vector tiene vecinos.
- Las relaciones con tipo necesitan un juez que entienda. Jev reconoce bien las contradicciones (las 3, sin falsas por encima de 0,9) pero no los otros tipos, así que en Clio solo anota contradicciones.
