# Temis sobre los pares del banco de recall (2026-11-15)

¿Sirve Temis para decidir, sin un modelo de lenguaje, si dos recuerdos parecidos dicen lo mismo? Se le preguntó por los 28 guardados repetidos de `bench/recall/duplicates.jsonl` frente al recuerdo que repiten: 8 copias exactas, 12 paráfrasis del mismo hecho y 8 que solo se parecen («Wompi cobra una comisión del 2,65 %» al lado de «los pagos van por Wompi»). Datos inventados.

```sh
CLIO_MEMORY_TEMIS_URL=https://api.imontes.dev/temis CLIO_MEMORY_TEMIS_TOKEN=… python3 bench/temis/run.py
```

Servicio: `temis-2.4.0` (`v1112-mrbert-cortos`). 84 llamadas en 94 s (incluye la pausa de 0,6 s entre llamadas); ~0,5 s por llamada de ida y vuelta, ~150 ms en el servidor. Cero tokens de modelo de lenguaje.

Repetida el 2026-11-16, antes de publicar v0.12.0: las 28 filas y la tabla de umbrales salieron idénticas. Ese día `GET /v1/model` respondió `{"model":"temis-2.4.0","name":"Temis",…}`, y `clio-memory setup temis` lo leyó bien (`Temis listo: temis-2.4.0 en https://api.imontes.dev/temis`).

Dos preguntas, con el texto con que Temis fue entrenado:
- `memory_preserved`: «Todo lo que dice `original` está también en `unido`, aunque sea con otras palabras.» (sí o no, de 0 a 1), en los dos sentidos.
- `memory_relation`: `conflicts_with`, `scoped`, `compatible`, `related` o `ninguna`.

## Lo que respondió

| tipo | gemela | lo nuevo está en lo viejo | lo viejo está en lo nuevo | relación (confianza) |
|---|---|---|---|---|
| exacta | m7 | 0.44 | 0.44 | compatible (0.80) |
| exacta | m19 | 0.55 | 0.55 | compatible (0.94) |
| exacta | m25 | 0.72 | 0.72 | compatible (0.91) |
| exacta | m53 | 0.84 | 0.84 | compatible (0.96) |
| exacta | m67 | 0.65 | 0.65 | compatible (0.92) |
| exacta | m77 | 0.40 | 0.40 | compatible (0.95) |
| exacta | m83 | 0.66 | 0.66 | compatible (0.92) |
| exacta | m116 | 0.79 | 0.79 | compatible (0.92) |
| parafrasis | m7 | 0.24 | 0.24 | conflicts_with (0.84) |
| parafrasis | m8 | 0.85 | 0.82 | compatible (0.95) |
| parafrasis | m19 | 0.86 | 0.88 | compatible (0.94) |
| parafrasis | m22 | 0.48 | 0.81 | compatible (0.88) |
| parafrasis | m25 | 0.94 | 0.75 | compatible (0.86) |
| parafrasis | m54 | 0.43 | 0.34 | compatible (0.82) |
| parafrasis | m58 | 0.96 | 0.94 | compatible (0.97) |
| parafrasis | m70 | 0.41 | 0.53 | compatible (0.50) |
| parafrasis | m75 | 0.73 | 0.89 | compatible (0.93) |
| parafrasis | m95 | 0.87 | 0.85 | compatible (0.95) |
| parafrasis | m116 | 0.17 | 0.29 | compatible (0.92) |
| parafrasis | m32 | 0.74 | 0.81 | compatible (0.96) |
| relacionada | m19 | 0.05 | 0.15 | related (0.83) |
| relacionada | m22 | 0.13 | 0.36 | related (0.51) |
| relacionada | m7 | 0.17 | 0.14 | scoped (0.96) |
| relacionada | m58 | 0.33 | 0.13 | compatible (0.73) |
| relacionada | m116 | 0.07 | 0.05 | conflicts_with (0.96) |
| relacionada | m95 | 0.15 | 0.05 | related (0.47) |
| relacionada | m25 | 0.10 | 0.06 | compatible (0.58) |
| relacionada | m70 | 0.24 | 0.17 | conflicts_with (0.91) |

| se dobla si algún sentido ≥ | exactas | paráfrasis | solo parecidas |
|---|---|---|---|
| 0.5 | 6 de 8 | 9 de 12 | 0 de 8 |
| 0.7 | 3 de 8 | 8 de 12 | 0 de 8 |
| 0.8 | 1 de 8 | 8 de 12 | 0 de 8 |
| 0.9 | 0 de 8 | 2 de 12 | 0 de 8 |
| 0.95 | 0 de 8 | 1 de 12 | 0 de 8 |

## Qué dice

- **«Está contenido» separa bien lo que se parece de lo que dice lo mismo.** Ningún par solo parecido pasa de 0,36; ocho de las doce paráfrasis llegan a 0,81 o más en algún sentido. Con el umbral en **0,80** se doblan 8 de 12 paráfrasis y ninguna de las 8 parecidas. Con 0,90 (el umbral que Clio usa con Jev) solo 2 de 12.
- **Con textos idénticos responde bajo** (0,40 a 0,84). No importa: una copia exacta nunca llega a Temis, la resuelve el motor al guardar y el sueño por vectores (≥ 0,97).
- **Las contradicciones que ve no son de fiar.** Marcó `conflicts_with` con 0,96 en «El cumpleaños de Camilo, su socio, es el 2 de agosto» frente a «Su cumpleaños es el 14 de marzo», con 0,91 en dos hechos distintos sobre las citas, y con 0,84 en una paráfrasis. Tres falsas de 20 pares sin contradicción. Por eso el sueño no anota relaciones de Temis: su `conflicts_with` solo sirve para **no doblar** un par y dejarlo para revisar.
- 28 pares son pocos y los escribió la misma mano que los recuerdos. El umbral es una primera medida; hay que repetirla cuando haya uso real.
