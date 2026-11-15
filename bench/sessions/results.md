# Varias sesiones a la vez (2026-11-15)

¿Qué cuesta tener varias sesiones de agente abiertas, cada una con su `clio-memory mcp`? `run.py` lanza procesos MCP reales, como los lanza un agente, sobre una carpeta de datos temporal sembrada con los recuerdos de `bench/recall`, y mide memoria y tiempos.

```sh
python3 bench/sessions/run.py conc 5      # 5 sesiones buscando a la vez, 15 búsquedas cada una
python3 bench/sessions/run.py cold 5      # 5 sesiones que nunca usaron el modelo guardan a la vez
python3 bench/sessions/run.py idle        # una sesión: memoria antes, después y tras 110 s sin uso
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

## Aceptación para v0.10 (un solo proceso con el modelo)

Con 5 sesiones: los procesos MCP suman menos de 60 MB; hay un solo demonio y no pasa de 700 MB; `cold 5` guarda 5 de 5; tras 10 minutos sin uso no queda ningún demonio.
