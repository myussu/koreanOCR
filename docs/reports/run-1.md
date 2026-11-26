# Maru — Run 1: estabilizar y medir

Informe de la corrida desatendida del 25 y 26 de noviembre de 2026 (hora de Lima). Rama: `stabilize/run-1`, creada desde
`4e101cf` (`diagnostics/live-subtitle-latency`). Todo lo que dice "medido" o "verificado" lo observó el agente en esta PC;
lo que no, está en la sección «Sin verificar».

## En corto

* **El trabajo de semanas está a salvo.** El árbol sin commitear quedó en 10 commits lógicos más uno para
  `.gitattributes`; el árbol final es idéntico, byte a byte, al que había (comprobado con el hash del árbol).
* **La ruta «minar una línea a Anki» funciona contra un Anki real** (perfil de prueba «User 2», mazo descartable): tarjeta
  de palabra, tarjeta de frase, sonido, recuperación de un alta a medias, actualizar progreso, sonidos que faltan. No
  apareció ningún defecto en esa ruta.
* **Tarjetas bilingües**: con el 한국어기초사전 instalado, cada tarjeta y la hoja «Make cards» llevan significado en inglés y
  en español. Ya está instalado en tu Maru y tu configuración apunta al mazo `Coreano::2 Media` sin tarjeta inversa.
* **Defectos encontrados y corregidos**: 7 (lista abajo). Los dos más serios: el build de Windows se rompía con Rust 1.100
  (CI en rojo, y tu PC se habría roto al actualizar Rust), y Claude devolvía líneas vacías en subtítulos en vivo.
* **Gasto en APIs: unos USD 0,36** de un tope de 3.
* **Dos páginas esperan tu juicio** (no elegí ganador en ninguna): significados de tarjetas y voces.
* **Lo que no pude hacer**: medir Gemini de verdad. Tu clave de Gemini es de nivel gratuito: agotó su cuota a los pocos
  pedidos y los modelos fuertes (Gemini 4 Carbon, Neon, Argon, 3.1 Pro) la rechazan siempre.

## Estado inicial (tarea 0)

| | |
|---|---|
| Rama y HEAD | `diagnostics/live-subtitle-latency`, `4e101cf`, igual que `origin` |
| `git status` (git de Windows, `core.autocrlf=true`) | 73 archivos modificados (2.934 inserciones, 492 eliminaciones) y 22 sin seguimiento |
| Finales de línea | el índice tenía LF en todo; 157 archivos con CRLF en disco; sin `.gitattributes` |
| `main` local | 4 commits detrás de `origin/main` (no lo toqué; ver «Git» abajo) |
| Herramientas | rustc y cargo 1.99.0, clippy 0.1.99, node 24.18.0, git 2.55.0, Python 3.12.10, gh 2.97.0, ffmpeg 8.1.1 |
| Anki | 26.9.4, AnkiConnect API 6, perfil activo «User 2» (vacío al empezar) |

## Qué cambió

Commits de la rama, en orden:

| Commit | Qué |
|---|---|
| `e3e493f` | Conexiones guardadas con claves (Azure, Gemini), respuestas cortadas por el límite, keep-alive de Ollama |
| `47e49ae` | Subtítulos en vivo: una sola vez por línea, traductor de respaldo, modelo retenido en el otro PC, OCR en DirectML |
| `0c13e1f` | Historial en vivo persistente y líneas con estrella |
| `41d9088` | Voz: pedir otra toma cuando una se alarga demasiado |
| `a895440` | Cliente de Anki: notas de frase, sonidos añadidos después, lotes seguros, prueba opcional con Anki real |
| `2b5a1a4` | «Make cards»: tarjetas desde una línea del historial |
| `a232df6` | Devolver el foco al juego tras el selector y los controles |
| `2d52b87` | Arreglos menores encontrados probando |
| `bc6295a` | Documentación de todo lo anterior |
| `fe69e4e` | `run-maru.cmd` |
| `025c616` | `.gitattributes` (LF en el repositorio para cualquier git) |
| `147e12e` | Clippy en el código solo-Windows de captura |
| `6fad827` | Pruebas de Anki real detrás de un guardián de perfil y mazo |
| `0f7d7cb`, `8254cea` | DirectML opcional y su reversión (diagnóstico equivocado; ver defecto 1) |
| `7bd3151`, `b1b2f23` | Build con Rust 1.100: copia parcheada de `ort-sys` en `vendor/` |
| `9834d64` | Prueba de voz con carrera |
| `4a79d7b`, `9d74437` | Diccionario 한국어기초사전 y significados en inglés y español en las tarjetas |
| `e70c50d` | `maru connection …`, `maru bench --live --connection …`, Anthropic tratado como servicio alojado |
| `ead6d9b` | Claude sin «pensar» en respuestas cortas; herramientas de comparación de significados |
| `5a4de29` | `maru tts-eval --candidate` y servidor de Supertonic 3 para comparar voces |

Cada commit intermedio de los 10 primeros compila (workspace y app), comprobado uno por uno en un worktree aparte. Algunas
pruebas del commit `47e49ae` solo pasan desde `0c13e1f` (lo dice su mensaje).

**Un cambio que hice a tu código antes de publicar:** el repositorio es público y las pruebas nuevas usaban el nombre real
de tu recurso de Azure (7 veces). Lo cambié por `contoso` en `settings-connection.test.mjs` y `external_servers.rs`. El
árbol original, sin ese cambio, quedó en la referencia local `refs/backup/run-1-worktree` (no se subió).

### Tarjetas bilingües (tarea 4b)

* `tools/build_nikl.py` convierte el XML del diccionario en un archivo que `maru dict nikl` importa a `nikl.db`, un archivo
  aparte del `dict.db` de Wiktionary (que no se toca). Sigue el enfoque de `tools/build_dictionary.py` de koreanDiary. Usé
  la copia del 2019-08-02 (la misma que koreanDiary): 48.353 palabras, 67.653 acepciones.
* Con el diccionario instalado, «Make cards» ya no pide al modelo que escriba significados. Una palabra con **una sola
  acepción** toma inglés y español del diccionario, sin pedido al modelo. Las de **varias acepciones** van juntas en un
  solo pedido por línea, y el modelo solo contesta el número de acepción; una palabra que el diccionario no tiene recibe
  ahí mismo un significado corto en los dos idiomas. Sin respuesta se muestra la primera acepción con aviso de revisarla.
  En la hoja puedes elegir otra acepción o escribir la tuya.
* El tipo de nota `Maru Korean` gana el campo `MeaningEs` (diseño v2). A un tipo de nota anterior se le añade el campo al
  final sin tocar sus notas; si Anki no puede añadirlo, sigue todo como antes. La tarjeta de frase lista
  `palabra — inglés · español`.
* Un alta desde el panel («Add to Anki») solo lleva español si la palabra tiene una única acepción; si tiene varias, el
  campo queda vacío (no adivina). Queda para la corrida 2 (ver propuestas).
* Fuente y licencia: 국립국어원 «한국어기초사전», <https://krdict.korean.go.kr>, CC BY-SA 2.0 KR. Citado en el README, en
  el archivo generado y en la propia hoja. Los datos no están en el repositorio.

**Cobertura sobre tu historial** (las 115 líneas distintas de una copia de `history.db`, con Kiwi real):

| | Por aparición | Palabras distintas |
|---|---:|---:|
| Palabras por aprender | 478 | 267 |
| Están en el diccionario | 424 (89 %) | 224 (84 %) |
| Una sola acepción: no hace falta modelo | 104 (22 %) | 71 (27 %) |
| Líneas que no necesitan ningún pedido al modelo | 7 de 115 (6 %) | |

Es decir: el diccionario cubre casi todo, pero casi todas las líneas necesitan un pedido al modelo, porque las palabras
frecuentes del diálogo (하다, 오다, 알다…) tienen muchas acepciones.

### Tu configuración y tus datos (tarea 4a)

* `config.toml`: `learning.anki.deck = "Coreano::2 Media"` y `reverse = "off"` (Anki ya estaba activado). Copia previa:
  `%APPDATA%\maru\Maru\config\config.toml.before-run-1.bak`. No creé ese mazo en Anki.
* `nikl.db` nuevo en `%APPDATA%\maru\Maru\data\` (16 MB). Las demás bases de datos solo se leyeron o se copiaron.
* Carpeta nueva `%APPDATA%\maru\Maru\data\run-1\` con las dos páginas de revisión.
* Dos frases quedaron en tu caché de voz (`maru tts` en la tarea 3).

## Qué está verificado y cómo

| Qué | Cómo | Resultado |
|---|---|---|
| Comandos de «definition of done» | en esta PC, tras cada tarea y sobre el árbol final | verde: clippy (workspace y app), 690 pruebas del workspace (9 opcionales omitidas), 43 de la app, 170 de UI, 40 de `tools/` |
| CI | GitHub Actions en la rama | verde en Windows y Linux desde `b1b2f23`; los commits posteriores estaban en curso al escribir esto (ver «Dónde paré») |
| Cliente de Anki contra Anki real | `maru-srs --test real_anki`, perfil «User 2» | pasó (4,4 s); no dejó mazo, notas ni sonidos |
| Ruta del motor contra Anki real (M-9, M-24, M-39, M-40) | `maru-engine --test real_anki`, mismo perfil, voz real de Qwen3-TTS para los sonidos | pasó (11,5 s, tres veces) |
| Actualización del tipo de nota (campo `MeaningEs`) en un Anki real | las mismas pruebas, con el tipo de nota en el diseño anterior | pasó: campo añadido al final, diseño v2, tarjeta con su español |
| Conexiones reales | `maru connection test` con cada conexión guardada | local, NAS, Azure y Claude responden; Gemini responde mal (cuota) |
| Hover y screen scan | compilan, sus pruebas pasan, sus demos cargan en Chromium | capturas: [hover](run-1/hover-panel.png), [scan](run-1/scan-demo.png) |
| Hoja «Make cards» con inglés y español | demo del historial en Chromium | captura: [hoja](run-1/mine-sheet.png) |

Las pruebas con Anki real escriben a través de un **guardián** nuevo (`maru-srs/tests/support/guarded_anki.rs`): antes de
cada cambio pregunta a Anki qué perfil está abierto y se niega si no es el indicado en `MARU_REAL_ANKI_PROFILE`; solo deja
pasar cambios dentro del mazo descartable; nunca reenvía un `sync` ni un cambio de perfil. Tiene sus propias pruebas.

## Benchmark A: velocidad de subtítulos en vivo

30 líneas reales de una copia de tu historial × 3 pasadas por conexión, cada línea con las anteriores como contexto, un
pedido a la vez, a través del cliente de Maru (`maru bench --live --connection …`). Tiempos en ms. Mide solo el modelo: sin
OCR ni juego.

| Conexión | Modelo | Primera palabra p50 | p95 | Línea completa p50 | p95 | Más lenta | Fallos | Cortadas |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| local (RTX 5070 Ti) | `unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL` | 237 | 462 | 370 | 572 | 5.883 | 0 de 90 | 0 |
| NAS (Ollama, 3060 Ti) | `gemma4-12b-3060ti` | 204 | 279 | 376 | 643 | 5.038 | 0 de 90 | 0 |
| Azure | `gpt-4.1-mini` | 854 | 1.309 | 944 | 1.391 | 2.019 | 0 de 90 | 0 |
| Claude | `claude-haiku-5-5` | 580 | 717 | 663 | 836 | 985 | 0 de 90 | 0 |
| Claude | `claude-sonnet-5-5` | 1.336 | 4.420 | 1.573 | 4.564 | 8.307 | 0 de 90 | 0 |
| Gemini (clave gratuita) | `gemini-3.8-flash` | 11.298 | 20.234 | 11.443 | 20.283 | 20.283 | **85 de 90** | 0 |

* Gemini: solo 5 líneas respondidas (los tiempos son de esas 5). Fallos: 78 × HTTP 429 (cuota del nivel gratuito), 5 × HTTP
  503 («high demand»), 2 sin respuesta en 60 s.
* Claude, **antes del arreglo** (misma prueba): Haiku 10 de 90 líneas vacías y 13 cortadas; Sonnet **72 de 90 vacías** y 74
  cortadas. Ver defecto 3.
* La primera línea de cada conexión (calentamiento) no se cronometra.

## Benchmark B: calidad de significados de tarjetas (a ciegas)

50 líneas de tu historial: 223 palabras, de las cuales 142 tienen varias acepciones, 29 no están en el diccionario y 52
tienen una sola (iguales para todos). 48 de las 50 líneas necesitan una decisión. Cada candidato decidió igual que la hoja
(un pedido por línea). **No hay ganador: la calidad la juzgas tú en la página.**

| Candidato | Modelo o herramienta | Pedido p50 ms | p95 ms | Fallos | Costo por línea |
|---|---|---:|---:|---:|---:|
| Solo diccionario (primera acepción) | — | 0 | 0 | — | 0 |
| Diccionario + reranker en CPU | `Alibaba-NLP/gte-multilingual-reranker-base` | 2.847 | 9.520 | 0 | 0 |
| Modelo local | `unsloth/gemma-4-12B-it-qat-GGUF:UD-Q4_K_XL` | 675 | 1.238 | 0 de 48 | 0 |
| NAS | `gemma4-12b-3060ti` | 838 | 1.635 | 0 de 48 | 0 |
| Azure | `gpt-4.1-mini` | 1.058 | 1.571 | 0 de 48 | USD 0,0002 |
| Claude Haiku | `claude-haiku-5-5` | 817 | 1.343 | 0 de 48 | USD 0,0001 |
| Claude Sonnet | `claude-sonnet-5-5` | 1.350 | 2.108 | 0 de 48 | USD 0,0017 |
| Gemini Flash | `gemini-3.8-flash` | — | — | **48 de 48** (HTTP 429) | 0 (gratuito) |
| Gemini más fuerte que la clave puede usar | `gemini-4-hydrogen` | 8.251 | 42.770 | **39 de 48** | 0 (gratuito) |

* El reranker lista el coreano entre sus idiomas, pesa 306 M de parámetros (Apache-2.0), cargó en 15 s y puntúa todas las
  acepciones de cada palabra; no puede dar significado a una palabra que el diccionario no tiene.
* Los modelos ven como máximo 8 acepciones por palabra (hay palabras con hasta 41): si la correcta está más allá, no pueden
  elegirla. Es un límite del pedido, no del modelo.
* «El más fuerte de Gemini» debía ser `gemini-4-carbon` («our most capable reasoning model» según la página de precios de
  Google), pero tu clave no puede llamarlo. Usé `gemini-4-hydrogen`, el único Gemini 4 que la clave gratuita acepta.

## Dónde están las dos páginas de revisión

Fuera del repositorio, porque llevan líneas de tus juegos:

1. **Significados** — `%APPDATA%\maru\Maru\data\run-1\card-meanings-review.html`. 50 líneas, 9 candidatos como A…I, con las
   letras barajadas en cada línea. Clic en un significado: correcto (✓); otro clic: incorrecto (✗); otro: borrar. «Save
   marks» descarga `marks.json`. **La clave está aparte**: `…\run-1\card-meanings\key.json` (no la abras antes de terminar).
   Para contar después: `python tools/build_card_review.py score marks.json "%APPDATA%\maru\Maru\data\run-1\card-meanings\key.json"`.
2. **Voces** — `%APPDATA%\maru\Maru\data\run-1\voices\index.html`: las 8 frases de prueba con tu voz (Qwen3-TTS, sohee) y
   con Supertonic 3 (voz F1).

| Voz | Listo en | Memoria (pico) | VRAM (pico) | Velocidad |
|---|---|---|---|---|
| Qwen3-TTS 1.7B CustomVoice Q8, voz sohee (la configurada; `tts-server`, Vulkan) | 10,2 s y 11,7 s | 2.447 MB en uso, 4.480 MB privados | 3.996 MB | 0,10–0,26 × tiempo real |
| Supertonic 3, voz F1 (`Supertone/supertonic-3`, ONNX en CPU) | 0,9 s (8,4 s la primera vez, con descarga) | 519 MB en uso, 1.246 MB privados | 0 | 0,17–0,26 × tiempo real |

No cambié la voz por defecto. Nadie ha escuchado la página.

## Dinero gastado

| Qué | Tokens entrada / salida | USD |
|---|---|---:|
| Benchmark A: Azure | 26.502 / 877 | 0,012 |
| Benchmark A: Claude Haiku y Sonnet, antes del arreglo | 38.682 / 2.261 y 35.211 / 5.709 | 0,133 |
| Benchmark A: Claude Haiku y Sonnet, después | 39.163 / 1.515 y 39.361 / 1.561 | 0,099 |
| Benchmark B: Azure, Haiku, Sonnet | 25.053 / 1.000, 33.995 / 2.210, 33.995 / 1.638 | 0,101 |
| Pruebas de conexión y sondeos sueltos | unos 8.000 / 500 | 0,013 |
| Gemini | nivel gratuito | 0 |
| **Total** | | **≈ 0,36** |

Precios por millón de tokens: Claude Sonnet 5.5 2/10, Claude Haiku 5.5 0,10/0,50 (lista de Anthropic); Azure
`gpt-4.1-mini` 0,40/1,60 (precio de lista de OpenAI, **supuesto** para tu despliegue: revísalo en tu factura). Los tokens
son los que informó cada servidor.

## Defectos encontrados y corregidos

1. **El build de Windows fallaba con Rust 1.100** (todo el CI de Windows en rojo). El script de build de `ort-sys`
   (ONNX Runtime) copia `DirectML.dll` subiendo tres carpetas desde `OUT_DIR`; Cargo 1.100 añadió un nivel y la copia
   caía en carpetas que no existen. Tu PC tiene Rust 1.99 y por eso no se veía, pero se habría roto al actualizar.
   Arreglo: copia parcheada del crate en `vendor/ort-sys` (ver `vendor/README.md`). *Mi primer diagnóstico fue
   equivocado*: creí que era por activar DirectML y lo volví opcional (`0f7d7cb`); el CI siguió fallando, lo revertí
   (`8254cea`) y DirectML sigue activado por defecto como lo tenías. Después hizo falta `vendor/** -text` porque git en
   Windows convertía a CRLF la tabla de hashes del crate.
2. **Claude por el endpoint compatible con OpenAI se trataba como un servidor propio**: el primer pedido de cada arranque
   se rechazaba por `temperature` y se repetía, y era candidato a llamadas de keep-alive y descarga de Ollama. Ahora se
   reconoce `api.anthropic.com` y se trata como servicio alojado; tu conexión guardada funciona sin editarla.
3. **Claude devolvía líneas vacías en subtítulos en vivo** (72 de 90 en Sonnet 5.5, 10 de 90 en Haiku 5.5): ahí «pensar»
   está activado por defecto, el endpoint ignora `reasoning_effort` y lo pensado se descuenta del límite corto por línea.
   Con el esfuerzo en `none` (por defecto) Maru ahora pide no pensar en la forma que acepta cada modelo (`disabled`, o
   `between_tools` cuando el rechazo lo nombra). Las mismas 90 líneas respondieron todas.
4. **Clippy fallaba en esta PC** en el código solo-Windows de captura (el CI solo hace lint en Linux). Corregido.
5. **Prueba de voz con carrera** (`an_overlong_take_is_asked_for_again…`): falló una vez en el CI de Linux; el servidor de
   prueba contaba el pedido después de responder. Corregido.
6. **Las pruebas con Anki real no comprobaban el perfil**: podían escribir en el que estuviera abierto. Ahora pasan por el
   guardián.
7. **El conteo de tokens** nuevo sumaba de más con servidores que repiten el conteo en cada trozo (Gemini). Corregido
   antes de usarlo para los costos.

Observado y no corregido:

* `maru bench --live` sin `--connection` pasa por el motor, que ahora guarda cada línea en el historial persistente: un
  benchmark así ensuciaría tu `history.db`. No lo ejecuté sobre tus datos; el modo nuevo con `--connection` no abre las
  bases de datos.
* La primera vez que arranqué el modelo local en la sesión tardó 33 s en la primera palabra y luego perdió la conexión una
  vez. No se repitió en las cinco ejecuciones siguientes.
* `package.json` (`{}`) y `package-lock.json` sueltos en la raíz, sin seguimiento: parecen restos de un `npm` accidental.
  No los commiteé ni los borré.

## Sin verificar

* **Ninguna tarjeta fue vista ni escuchada por una persona.** Las pruebas comprueban campos, mazos, etiquetas y que el
  sonido quedó adjunto.
* **Tu perfil real de Anki y el mazo `Coreano::2 Media`**: todo se probó en «User 2».
* **AnkiWeb**: nunca se pidió sincronizar. No sé si Anki pedirá una sincronización completa tras añadir el campo
  `MeaningEs` a un tipo de nota con tarjetas.
* **Si las acepciones elegidas son correctas**: eso es la página de revisión.
* **La app en un juego real**: nada de esta corrida se probó con un juego, con foco de ventana ni con los botones de la
  app; las hojas y páginas se vieron en Chromium con datos de demostración.
* **Gemini**: velocidad y calidad reales, por la cuota.
* **El build con Rust 1.100 en tu PC**: lo verificó el CI de Windows, no esta máquina (tiene 1.99).
* Los tipos de nota `Maru Korean` (ahora con 13 campos) y `Maru Korean Sentence` **quedaron en el perfil «User 2»**:
  AnkiConnect no puede borrar tipos de nota.

## Lo que solo tú puedes comprobar

1. **Calificar significados** (unos 30–40 min): abre `card-meanings-review.html`, marca, «Save marks», y corre el comando
   `score` de arriba.
2. **Escuchar las voces**: abre `voices\index.html` y compara sohee con Supertonic 3 en cada frase; fíjate en los cambios
   de sonido (같이, 독립, 값이) y en los números.
3. **Una tarjeta de verdad**: reinicia Maru (para que cargue el diccionario; Estado debe decir «Learner's dictionary:
   한국어기초사전»). Con Anki en tu perfil normal, Historial → una línea → **Make cards**. Mira que cada palabra traiga
   inglés y español y de dónde salió; añade una palabra y la frase. En Anki: mazo `Coreano::2 Media`, una sola tarjeta por
   palabra (sin inversa), español bajo el inglés, sonido de palabra y de frase.
4. **La sincronización**: después de esa primera alta, sincroniza. Si Anki pide sincronización completa, elige **Subir a
   AnkiWeb** desde esta PC. Dime si lo pidió.
5. **Si no quieres el cambio de configuración**: copia `config.toml.before-run-1.bak` sobre `config.toml`.
6. **Gemini**: activa la facturación en la clave (o crea una de pago) en <https://aistudio.google.com> y luego
   `maru connection test "Gemini=gemini-4-carbon"`. Si responde, se pueden repetir los dos benchmarks solo para Gemini.
7. **Un juego real** con subtítulos en vivo, estrella (`Ctrl+Alt+B`) y Make cards: los pasos están en `docs/MANUAL_TESTS.md`,
   M-37 a M-41.
8. **El precio de Azure** de tu despliegue, para corregir la tabla de costos si no es el de lista.

## Git

* `main` local está 4 commits detrás de `origin/main`; no lo reconcilié.
* `stabilize/run-1` parte de un punto anterior a esos 4 commits de `origin/main` (evaluación de OCR y set de pruebas): el
  pull request puede tener conflictos con `main` que habrá que resolver al fusionar. No fusioné nada.
* Referencias locales de respaldo, sin subir: `refs/backup/run-1-worktree` (tu árbol exacto) y
  `refs/backup/run-1-worktree-public` (con el nombre de Azure cambiado).

## Propuestas para la corrida 2

1. **Elegir el modelo para significados** con tus marcas de la revisión, y fijarlo como modelo de «snapshot».
2. **Español en las altas desde el panel** cuando la palabra tiene varias acepciones (hoy queda vacío): el mismo pedido
   de elección con la frase en que se vio la palabra.
3. **Más de 8 acepciones**: para palabras como 하다 (41), elegir en dos pasos o filtrar antes con el reranker.
4. **Proveedor nativo de Anthropic** (`/v1/messages`): el endpoint compatible sirve, pero sin caché de prompts ni control
   de esfuerzo; con el nativo, Sonnet podría bajar su latencia.
5. **Quitar `vendor/ort-sys`** cuando `oar-ocr` use un `ort-sys` que compile con Cargo actual.
6. **Que `maru bench` nunca escriba en el historial** (modo de solo lectura para el motor).
7. Lo que quedó fuera de alcance: aviso de cola en Make cards, lectura de subtítulos de YouTube, rediseño de UI y
   controles, cambio de voz por defecto según lo que oigas.

## Dónde paré

Todas las tareas 0 a 9 se hicieron. Lo único incompleto es Gemini en los dos benchmarks, por la cuota de la clave. El
estado final del CI y del pull request está en la descripción del propio pull request.
