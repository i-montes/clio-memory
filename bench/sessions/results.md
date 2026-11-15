# Varias sesiones a la vez (2026-11-15)

¿Qué cuesta tener varias sesiones de agente abiertas, cada una con su `clio-memory mcp`? `run.py` lanza procesos MCP reales, como los lanza un agente, sobre una carpeta de datos temporal sembrada con los recuerdos de `bench/recall`, y mide memoria y tiempos.

```sh
python3 bench/sessions/run.py conc 5      # 5 sesiones buscando a la vez, 15 búsquedas cada una
python3 bench/sessions/run.py cold 5      # 5 sesiones que nunca usaron el modelo guardan a la vez
python3 bench/sessions/run.py idle        # una sesión: memoria antes, después, tras 110 s sin uso y hasta que el demonio sale
CLIO_BENCH_BIN=target/release/clio-memory python3 bench/sessions/run.py conc 5   # otro binario
```

Equipo: Ryzen 7 5700X (16 hilos), 32 GB, Linux. Modelo: Granite cuantizado en el procesador.

## v0.9.0 (línea base)

Cada proceso MCP carga su propia copia del modelo la primera vez que busca o guarda.

| Sesiones buscando a la vez | RAM de los procesos MCP | Primera búsqueda (p50 / máx.) | Siguientes (p50 / p95) | Búsquedas por segundo |
|---|---|---|---|---|
| 1 | 500 MB | 1,5 s | 42 / 54 ms | 7,1 |
| 3 | 1.502 MB | 1,9 / 1,9 s | 123 / 219 ms | 12,0 |
| 5 | 2.480 MB (pico 2.967) | 2,5 / 3,7 s | 152 / 338 ms | 12,7 |
| 8 | 3.964 MB (pico 4.118) | 3,8 / 5,1 s | 200 / 725 ms | 13,4 |

- La memoria crece ~500 MB por sesión (pico por proceso: 626 MB) y el rendimiento total se queda en ~13 búsquedas por segundo: más procesos no rinden más, solo ocupan más.
- **Sin uso:** a los 60 s el modelo se suelta, pero el proceso baja solo de ~510 MB a ~315 MB y ahí se queda hasta que muere. Al volver a buscar, recarga (1,4 s) y sube a ~570 MB. Ni `MALLOC_ARENA_MAX=1` ni fijar los umbrales de `mmap` lo cambian (324 y 253 MB); la causa no está identificada.
- **Guardados en frío a la vez:**

  | Sesiones | Guardados | Rechazados («otro proceso está escribiendo la memoria; reintenta») |
  |---|---|---|
  | 2 | 2 | 0 |
  | 3 | 2 | 1 |
  | 5 | 2 | 3 |

  Cada proceso retiene el candado de escritor mientras carga su modelo (~1,5 s) y los demás esperan 2 s como mucho. `bench/concurrency` no lo vio porque midió sin modelo.

## v0.10.0 (un solo proceso con el modelo, 2026-11-15)

Mismo equipo y mismo modelo que la línea base. Cada proceso `mcp` solo pasa las llamadas al demonio (uno por carpeta de datos); el demonio carga el modelo una vez y **lo conserva hasta que sale** (a diferencia de v0.9.0 y de la primera medición de v0.10.0, donde también lo soltaba a los 60 s). Binario de `cargo build --release --features model2vec,granite`.

| Sesiones buscando a la vez | RAM de los procesos MCP | RAM del demonio (pico) | Primera búsqueda (p50 / máx.) | Siguientes (p50 / p95) | Búsquedas por segundo |
|---|---|---|---|---|---|
| 1 | 7 MB | 509 MB (639) | 19 ms | 22 / 26 ms | 45,7 |
| 3 | 22 MB | 520 MB (639) | 54 / 78 ms | 73 / 81 ms | 40,9 |
| 5 | 36 MB | 524 MB (640) | 81 / 141 ms | 121 / 137 ms | 41,0 |
| 8 | 58 MB | 522 MB (641) | 132 / 201 ms | 190 / 210 ms | 42,1 |

- La memoria de los procesos `mcp` crece ~7 MB por sesión y el demonio se queda en ~520 MB (pico 641 MB) sea cual sea el número de sesiones. Con 5 sesiones: 36 + 524 = 560 MB, frente a 2.480 MB. Sin errores en ninguna prueba.
- El rendimiento total sube de ~13 a ~42 búsquedas por segundo, y el tiempo crece con las sesiones porque el demonio atiende las llamadas de una en una.
- **«Primera búsqueda» no es comparable con v0.9.0:** `seed` (que guarda los recuerdos antes de medir) ya había cargado el modelo en el demonio, así que aquí la primera búsqueda no paga la carga. En v0.9.0 cada sesión la pagaba (1,5 a 3,8 s). La carga la paga la primera llamada de todas al demonio (~1,4 s).
- **Guardados en frío a la vez** (`cold 5`): 5 guardados, 0 rechazados (ms: 25, 47, 78, 109, 138; procesos `mcp` 36 MB; demonio 503 MB, pico 641 MB). En v0.9.0 solo se guardaban 2 de 5.
- **Sin uso** (`idle`, una sesión): el demonio se queda en 498-499 MB (pico 639 MB) a los 30, 60, 75 y 110 s sin uso: ya no suelta el modelo. La búsqueda tras esa pausa tarda 19 ms (antes recargaba: 1,4 s y 867 MB). El proceso `mcp` se queda en 6-7 MB. Cerrada la sesión, el demonio siguió en 499 MB y en el control de los 603 s ya no quedaba ningún demonio.

### Aceptación para v0.10

Criterio, con 5 sesiones: los procesos MCP suman menos de 60 MB; hay un solo demonio y no pasa de 700 MB; `cold 5` guarda 5 de 5; tras 10 minutos sin uso no queda ningún demonio. Veredicto:

- Procesos `mcp` suman menos de 60 MB: **cumple**, 36 MB (con 8 sesiones, 58 MB).
- Un solo demonio y no pasa de 700 MB: **cumple**, un `daemon` con 524 MB y pico de 640 MB en `conc 5`; en ninguna prueba (`conc 1/3/5/8`, `cold 5`, `idle`) pasó de 641 MB de pico ni de 524 MB de RAM. El pico de ~640 MB es el de la carga del modelo.
- `cold 5` guarda 5 de 5: **cumple**, 5 guardados, 0 rechazados.
- Tras 10 minutos sin uso no queda ningún demonio: **cumple**, 0 `daemon` a los 603 s de cerrar la sesión (`run.py idle` espera hasta 12 minutos a que salga); mientras tanto el demonio no soltó el modelo (499 MB hasta el final).

### Corrección tras la primera medición

La primera medición de v0.10.0 dejaba que el demonio soltara el modelo a los 60 s: bajaba a 285 MB, pero al volver a buscar lo recargaba y llegaba a 861-867 MB (pico 1.054-1.058 MB), por encima de los 700 MB (reproducido dos veces con `idle`). Desde `fix(daemon): it keeps its model until it leaves` el demonio lo conserva hasta que sale a los 10 minutos sin uso, y esa subida desaparece (ver `idle`).
