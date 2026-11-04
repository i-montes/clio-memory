# Prueba de modelos de vectores (2026-11-04)

Recuerdos y preguntas en español (`memories.jsonl`, `queries.jsonl`, datos inventados): cada pregunta es una paráfrasis, con otras palabras, de uno o dos recuerdos. Equipo: RTX 4060 (8 GB), 16 hilos de CPU. Se corre con `uv run --python 3.12 --no-project --with sentence-transformers --with model2vec --with numpy bench/embeddings/run.py`.

| Modelo | Dim. | Acierto 1.º | Entre los 5 primeros | MRR | ms por recuerdo (CPU) | ms por recuerdo (GPU) | Carga (s) |
|---|---|---|---|---|---|---|---|
| potion-multilingual-128M (actual) | 256 | 40% | 65% | 0.522 | 0.2 | — | 3.8 |
| granite-97m-multilingual-r2 | 384 | 72% | 95% | 0.834 | 29.7 | 6.0 | 27.6 |
| granite-311m-multilingual-r2 · 768 | 768 | 88% | 98% | 0.928 | 101.9 | 9.7 | 55.1 |
| granite-311m-multilingual-r2 · 384 | 384 | 90% | 98% | 0.940 | 101.4 | 9.7 | 1.2 |
| granite-311m-multilingual-r2 · 256 | 256 | 85% | 98% | 0.908 | 102.1 | 9.8 | 1.2 |
| Qwen3-Embedding-0.6B (referencia) | 1024 | 85% | 98% | 0.917 | 403.1 | 20.0 | 98.7 |

**Lectura:**
- Granite 311M acierta a la primera el 88–90 % frente al 40 % del modelo actual; entre los cinco primeros, 98 % frente a 65 %.
- Recortado a 384 dimensiones (Matryoshka) rinde igual que a 768, con la mitad de espacio; a 256 pierde un poco.
- En GPU, unos 10 ms por recuerdo; en CPU, unos 100 ms. Los tiempos de CPU son de PyTorch cargado en bfloat16: en float32 o con ONNX Runtime (lo que usaría clio-memory) pueden ser otros, y hay que medirlos allí.
- La carga de 55 s de la primera fila incluye la descarga; desde disco tarda ~1 s.
- 40 preguntas: diferencias de uno o dos puntos están dentro del ruido.

## Fallos por modelo


potion-multilingual-128M (actual): 24
  - «¿qué tan largas le gustan las contestaciones?» · puesto 3 · salió m58
  - «¿se puede mezclar directo a la rama principal?» · puesto 5 · salió m38
  - «¿qué base de datos guarda las órdenes de compra?» · puesto 4 · salió m6
  - «¿cuánto dura una sesión guardada en memoria intermedia?» · puesto 8 · salió m16
  - «¿en qué proveedor y con qué herramienta se pone en marcha la aplicación?» · puesto 16 · salió m44
  - «¿qué comando de Rust no debo correr porque descarga gigas?» · puesto 22 · salió m27
  - «¿en qué carpeta está el modelo que convierte texto en números?» · puesto 13 · salió m50
  - «¿qué pasa cuando se le pide al agente que olvide algo?» · puesto 16 · salió m60
  - «¿qué tarjeta de video tiene el equipo?» · puesto 2 · salió m30
  - «¿con qué se cobra a los clientes?» · puesto 8 · salió m54
  - «el mismo pago se registraba dos veces» · puesto 2 · salió m22
  - «¿cómo inician sesión los usuarios?» · puesto 14 · salió m40
  - «¿qué servicio manda los emails automáticos?» · puesto 2 · salió m55
  - «pasos para sacar una nueva versión» · puesto 11 · salió m38
  - «¿cómo recupero los datos si se daña la base?» · puesto 11 · salió m57
  - «¿a qué se dedica la persona además de programar?» · puesto 31 · salió m47
  - «¿cómo deben empezar los videos?» · puesto 24 · salió m30
  - «la ventana de conversación se quedaba pegada con textos muy grandes» · puesto 16 · salió m17
  - «¿dónde se almacenan las fotos de los artículos?» · puesto 2 · salió m42
  - «¿puedo agregar una librería al proyecto sin consultar?» · puesto 16 · salió m50
  - «¿a qué hora es mejor proponerle una llamada?» · puesto 2 · salió m58
  - «¿dónde se guardan las contraseñas y llaves del proyecto?» · puesto 22 · salió m24
  - «¿con qué transportadora se despachan los paquetes?» · puesto 4 · salió m25
  - «los mensajes de confirmación llegan a la bandeja de correo no deseado» · puesto 3 · salió m25

granite-97m-multilingual-r2: 11
  - «¿en qué proveedor y con qué herramienta se pone en marcha la aplicación?» · puesto 6 · salió m44
  - «el escáner de credenciales me bloquea una prueba con una clave falsa» · puesto 4 · salió m24
  - «¿en qué carpeta está el modelo que convierte texto en números?» · puesto 2 · salió m51
  - «¿qué pasa cuando se le pide al agente que olvide algo?» · puesto 9 · salió m33
  - «¿con qué se cobra a los clientes?» · puesto 2 · salió m53
  - «¿qué servicio manda los emails automáticos?» · puesto 2 · salió m54
  - «¿cómo deben empezar los videos?» · puesto 2 · salió m27
  - «¿el agente puede conectarse a internet cuando ejecuta comandos?» · puesto 2 · salió m60
  - «¿puedo agregar una librería al proyecto sin consultar?» · puesto 3 · salió m57
  - «¿dónde se guardan las contraseñas y llaves del proyecto?» · puesto 2 · salió m57
  - «los mensajes de confirmación llegan a la bandeja de correo no deseado» · puesto 2 · salió m25

granite-311m-multilingual-r2 · 768: 5
  - «¿en qué proveedor y con qué herramienta se pone en marcha la aplicación?» · puesto 2 · salió m44
  - «el escáner de credenciales me bloquea una prueba con una clave falsa» · puesto 8 · salió m24
  - «¿en qué carpeta está el modelo que convierte texto en números?» · puesto 2 · salió m51
  - «¿con qué se cobra a los clientes?» · puesto 2 · salió m53
  - «¿el agente puede conectarse a internet cuando ejecuta comandos?» · puesto 2 · salió m60

granite-311m-multilingual-r2 · 384: 4
  - «¿en qué proveedor y con qué herramienta se pone en marcha la aplicación?» · puesto 2 · salió m44
  - «el escáner de credenciales me bloquea una prueba con una clave falsa» · puesto 12 · salió m24
  - «¿en qué carpeta está el modelo que convierte texto en números?» · puesto 2 · salió m50
  - «¿el agente puede conectarse a internet cuando ejecuta comandos?» · puesto 2 · salió m60

granite-311m-multilingual-r2 · 256: 6
  - «¿en qué proveedor y con qué herramienta se pone en marcha la aplicación?» · puesto 2 · salió m44
  - «el escáner de credenciales me bloquea una prueba con una clave falsa» · puesto 7 · salió m24
  - «¿en qué carpeta está el modelo que convierte texto en números?» · puesto 3 · salió m51
  - «¿con qué se cobra a los clientes?» · puesto 2 · salió m43
  - «¿el agente puede conectarse a internet cuando ejecuta comandos?» · puesto 2 · salió m60
  - «¿cómo funciona el buscador del catálogo con tildes?» · puesto 3 · salió m12

Qwen3-Embedding-0.6B (referencia): 6
  - «¿en qué proveedor y con qué herramienta se pone en marcha la aplicación?» · puesto 6 · salió m44
  - «¿qué pasa cuando se le pide al agente que olvide algo?» · puesto 2 · salió m33
  - «¿qué servicio manda los emails automáticos?» · puesto 2 · salió m55
  - «¿puedo agregar una librería al proyecto sin consultar?» · puesto 2 · salió m57
  - «¿dónde se guardan las contraseñas y llaves del proyecto?» · puesto 2 · salió m57
  - «¿con qué transportadora se despachan los paquetes?» · puesto 2 · salió m25

## En Rust (clio-memory, ONNX Runtime 1.31 en CPU)

Medido con `examples/granite_time.rs`, un recuerdo de ~150 caracteres, mediana de 30:

| Archivo | Dispositivo | ms por recuerdo | Carga | Acierto 1.º (`tests/granite.rs`) |
|---|---|---|---|---|
| `model.onnx` (completo) | CPU | 26 | 2,3 s | 90 % |
| `model_quint8_avx2.onnx` (cuantizado) | CPU | 18 | 1,5 s | 85 % |

Decisión: el completo siempre (GPU o CPU); el cuantizado solo si es el único archivo. Los vectores en Rust coinciden con los de sentence-transformers en float32 (coseno ≥ 0,999, `tests/fixtures/granite_reference.json`).
