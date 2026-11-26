# Maru — Run 3: OCR con subtítulos reales de juegos

Informe de la corrida desatendida del 26 de noviembre de 2026 (hora de Lima). Rama: `feature/run-3-ocr`, creada desde
`6583757` (la punta de `feature/run-2`) con `origin/main` fusionado. Todo lo que dice «medido» lo midió el agente en esta
PC; lo que no, está en «Sin verificar». La corrida se detuvo una vez por el límite de uso y se retomó donde estaba.

## En corto

* **No cambié ningún valor por defecto.** Con etiquetas provisionales el encargo lo prohíbe, y con las 30 líneas de oro
  ninguna diferencia sale del azar. Todo queda como opción apagada, con sus números.
* **Lo que Maru usa hoy no es lo que el encargo suponía:** el detector instalado es el PP-OCRv5 **server** (88 MB), no el
  mobile, y sus archivos ONNX son una conversión de la comunidad (`monkt/paddleocr-onnx`), no de PaddlePaddle. El lector
  PaddleOCR-VL-1.5 sí es el GGUF oficial. Tu configuración: PP-OCR en CPU más ese lector.
* **El set de pruebas existe:** 544 imágenes de subtítulos de Elden Ring, Sekiro, Stellar Blade, Ori y Xenoblade en
  `%APPDATA%\maru\Maru\data\evalset\raw`. 89 son de oro (las 30 líneas verificadas), 408 llevan etiqueta
  **provisional** y 47 te esperan.
* **Hallazgo principal (provisional):** en estos subtítulos, que son letra limpia, **el lector PaddleOCR-VL-1.5 estropea
  más líneas de las que arregla**. PP-OCR solo lee igual o mejor que el híbrido de hoy (mitad de juicio: mejor en 26
  subtítulos, peor en 13, p = 0,053), tarda 98 ms en vez de 166 y no usa memoria de vídeo. La excepción es Xenoblade
  (letra estilizada, 15 subtítulos): ahí el híbrido gana.
* **PaddleOCR-VL-1.6 lee mejor que 1.5 dentro del híbrido** (mitad de juicio: mejor en 17, peor en 6, p = 0,035), con el
  mismo tiempo y la misma memoria. Es la candidata más clara: **lista para cambiar cuando tus etiquetas lo confirmen.**
* **Detectores PP-OCRv6 (small, medium) y v5 mobile:** leen como el detector de hoy, dentro del azar, y tardan entre un
  35 % y un 60 % menos (38 a 64 ms frente a 98). Son una ganancia de velocidad, no de lectura.
* **Qota-OCR** (reconocedor de la comunidad): sin diferencia demostrable frente al reconocedor de hoy.
* **Ampliar ×2 no ayuda** (en el híbrido, peor en 6 subtítulos y mejor en ninguno). **GLM-OCR, Nanonets-OCR2 y Surya 2.5
  leen peor** que PaddleOCR-VL como lector del híbrido.
* **Las etiquetas provisionales favorecen a los lectores de hoy**: una imagen solo recibe una cuando PP-OCR o el híbrido
  ya la leyeron bien. Sirven para ver que un candidato es peor; apenas pueden mostrar que es mejor. Por eso tus
  etiquetas importan, sobre todo en las 47 imágenes que esperan.
* **Un defecto en la división dev / held-out** que venía de `main`: todas las escenas de un vídeo caían del mismo lado
  (la «mitad de juicio» era Stellar Blade entero y nada más). Corregido, con prueba.
* **Gasto en APIs: USD 0,82** de un tope de 3 (Claude Sonnet como lector externo; las imágenes salieron de esta PC).
* **Un juego quedó abierto durante las mediciones** (`ssbu.exe`, algo más de un núcleo y un 14 % de la gráfica). No lo
  cerré. Los tiempos absolutos son probablemente algo lentos; la comparación entre motores es bajo la misma carga.

## Estado inicial (tarea 0)

| | |
|---|---|
| Rama y HEAD | `feature/run-2` en `6583757`; árbol limpio salvo `.claude/`, `package.json` y `package-lock.json` sin seguimiento (no los toqué) |
| Pruebas de `feature/run-2` antes de tocar nada | verde: clippy (workspace y app); 767 pruebas del workspace (9 opcionales omitidas), 45 de la app, 178 de UI |
| Fusión de `origin/main` | commit de fusión `31e7385` (sin rebase); 13 archivos en conflicto, resueltos conservando los dos lados |
| Pruebas tras la fusión | verde: 790 del workspace, 45 de la app, 187 de UI, las de `tools/` |
| Herramientas | rustc 1.99.0, node 24, Python 3.12 (numpy 2.5), ffmpeg, llama.cpp build 11540 |
| Tu `[ocr]` (leído, no modificado) | `backend = auto`, `device = cpu`, `vision_reader = true`; sin `cpu_ocr`, sin ampliación, sin archivos de modelo propios |

Cómo quedó la fusión, donde los dos lados tocaban lo mismo:

* **Leer en vivo.** La corrida 2 leía con `recognize_now` (no esperar a un lector que aún carga) y `main` con
  `recognize_scaled` (ampliar para el detector). Ahora el vigilante usa `recognize_now_scaled`, que hace las dos cosas;
  el híbrido lo implementa y los demás motores lo heredan.
* **Guardar lo que lee** no se activa en una corrida de solo lectura (`maru bench`), que no debe dejar nada.
* **Settings por flujo:** «Save the subtitles it reads» y «Enlarge the subtitle area before reading it» están en Live
  subtitles, como ajustes avanzados, con una frase de ayuda y su «Used by». Vistos en Edge sin interfaz con datos de
  demostración; nadie hizo clic.
* **Pruebas manuales:** los M-14, M-15 y M-16 de `main` chocaban con números que esta rama ya usaba; aquí son M-50
  (ampliación), M-48 (guardar lo leído) y M-49 (página de etiquetado).

## Qué cambió

| Commit | Qué |
|---|---|
| `31e7385` | Fusión de `origin/main`: guardar lo leído, página de etiquetado, informe de `eval-ocr`, `ocr_scale` |
| `9912b07` | Herramientas y opciones para comparar piezas de OCR (detalle abajo) |
| último commit | La división dev / held-out corregida, `maru eval-report`, este informe y `MANUAL_TESTS.md` (M-51 a M-54) |

**Herramientas nuevas (`tools/`, todas con pruebas):**

* `find_subtitles.py` encuentra cuándo hay subtítulos en una grabación mirando los píxeles (trazos finos y claros que se
  quedan quietos en el centro de la banda inferior), **sin OCR**, para que el set no lo elija el lector que se va a
  medir. Comprobado contra las 30 líneas de oro: encontró 29.
* `sample_subtitle_frames.py` gana `--prefix` (el nombre lleva el tramo de cinco minutos, que la división mantiene
  entero) y filas `?` (imagen sin etiqueta).
* `provisional_labels.py` escribe `name.provisional.txt` donde el lector externo y uno local leyeron las mismas letras;
  `--check` aplica la regla a las imágenes ya etiquetadas; `--set-aside` aparta vacías y repetidas.
* `compare_ocr.py` es **el comando único**: corre el motor de hoy y cada candidato y imprime las tablas.

**En `maru eval-ocr`:** `--det`, `--rec`, `--dict`, `--det-tuning`, `--device`, `--reader-dir`, `--reader-html` (cambiar
una pieza para una corrida); `--engine hybrid-endpoint`; `--engine connection --connection NOMBRE` (una conexión
guardada lee las imágenes: salen de la PC, y se imprimen los tokens); `--labels person|provisional|all`; percentil 95 y
picos de memoria y de memoria de vídeo del programa y de su servidor de modelo. Ahora arranca en solo lectura.
`maru eval-report a.json b.json …` imprime las tablas de corridas guardadas.

**Opciones nuevas en `[ocr]`, todas sin valor por defecto** (`docs/config.example.toml`): `det_score_threshold`,
`det_box_threshold`, `det_unclip_ratio` (umbrales propios del detector); `reader_dir`, `reader_prompt`, `reader_args`,
`reader_html` (otro modelo lector en lugar de PaddleOCR-VL-1.5). `det_model`, `rec_model` y `dict_path` ya existían.

## El set de pruebas (tareas 1 y 2)

**De dónde salió.** De `vidTestsKoreanSubs\korean_videos` (solo lectura; no se escribió nada ahí). Los cortes se hicieron
con `tools/sample_subtitle_frames.py` sobre los momentos que encontró `find_subtitles.py`, repartidos por todo el vídeo
(un momento por tramo igual de tiempo, no los primeros minutos). Una imagen por subtítulo, salvo las 30 líneas de oro
(tres fotogramas cada una). Las imágenes no están en el repositorio.

**Las cajas.** Comprobé cada caja de `metadata.json` contra fotogramas reales antes de usarla:

| Juego | Caja en `metadata.json` (720p) | ×1,5 a 1080p | La que usé | Por qué |
|---|---|---|---|---|
| Elden Ring | `220:610:840:75` | `330:915:1260:112` | la misma | Los subtítulos del juego caen dentro (y = 952–976 en el vídeo de jefes; 950–1016 en el de historia) |
| Sekiro | `200:590:880:90` | `300:885:1320:135` | la misma | Una y dos líneas entre y = 896 y 976 |
| Stellar Blade | `200:580:880:110` | `300:870:1320:165` | la misma | Una línea en y = 944–968 |
| Ori | `220:600:840:80` | `330:900:1260:120` | **`300:765:1320:235`** | La caja era errónea: el diálogo ocupa de una a tres líneas entre y = 780 y 995, y la caja cortaba las de arriba |
| Xenoblade | no figura | — | **`300:820:1320:180`** | Dos líneas entre y = 848 y 968, medido en tu grabación |

**Cuántos subtítulos por juego.**

| Juego | Momentos hallados | Imágenes cortadas | Apartadas | **Quedan** | Oro | Provisional | Te esperan |
|---|---:|---:|---:|---:|---:|---:|---:|
| Elden Ring (2 vídeos) | 92 + 416 | 210 + 110 | 97 repetidas, 38 rótulos cortados, 1 de oro sin subtítulo | **184** | 89 (30 líneas) | 86 | 9 |
| Sekiro | 389 | 150 | 26 rótulos del autor, 1 vacía, 1 repetida | **122** | — | 114 | 8 |
| Stellar Blade | 449 | 125 | 1 vacía | **124** | — | 118 | 6 |
| Ori | 198 | 140 | 36 vacías, 6 repetidas | **98** | — | 75 | 23 |
| Xenoblade (75 s, todo) | 19 | 16 | — | **16** | — | 15 | 1 |
| **Total** | | 751 | 207 | **544** | **89** | **408** | **47** |

Subtítulos distintos con etiqueta: 30 de oro y 408 provisionales. Lo apartado está en
`…\data\evalset\set-aside\`, con sus borradores, por si quieres revisarlo:

* **Rótulos del autor del vídeo** (no son subtítulos del juego). El vídeo de Sekiro lleva la narración del autor en
  letra grande con borde azul, a veces más ancha que la caja: 26 imágenes apartadas por una regla de píxeles (línea de
  más de 36 px de alto y trazos gruesos casi blancos). El vídeo de historia de Elden Ring lleva la narración más abajo
  que los subtítulos del juego, y la caja la cortaba: 38 imágenes. **Esas 38 entraron por un error mío** (dejé 8 px de
  tolerancia al filtrar por posición); lo vi en los comentarios del lector externo («the strip cuts off the subtitle»)
  y las saqué antes de medir nada sobre ellas. Ninguna había recibido etiqueta provisional.
* **Vacías** (38): ningún lector, tampoco el externo, encontró coreano. Casi todas de Ori (su mapa y su HUD).
* **Repetidas** (104): el mismo texto que una imagen anterior del mismo juego. «Los fotogramas de un subtítulo cuentan
  una vez.»
* **Un fotograma de oro sin subtítulo:** el conjunto verificado da la línea 8 («…나를 더 높일지어다») hasta el segundo
  74,55, pero en el vídeo desaparece antes; mi tercer fotograma (≈ 73,3 s) no muestra texto. Lo aparté. En seis de las
  30 líneas, `start_seconds`/`end_seconds` y `first_visible`/`first_absent` no coinciden; usé el tramo común.

**Las etiquetas.**

* **Oro:** el texto de `elden_ring_verified_dataset.json`, en `name.txt`, con la etiqueta `gold`. No usé ningún
  `*_dialogue.json`, ni como etiqueta ni como pista.
* **Borradores de tres lectores independientes** junto a cada imagen: `name.hybrid.txt` (el híbrido de hoy),
  `name.paddle.txt` (PP-OCR solo) y `name.outside.txt` (Claude Sonnet 5.5 por tu conexión «Claude»).
* **Provisional** (`name.provisional.txt`): el texto del lector externo cuando al menos uno de los dos locales leyó las
  mismas letras y dígitos (sin contar espacios ni puntuación). Ningún candidato en prueba fue fuente de etiquetas.
* **La barra de error, medida en las líneas de oro:** aplicada a las 89 imágenes de oro, la regla habría etiquetado 85
  (las 30 líneas, al menos un fotograma de cada una) y **ninguna de las 85 etiquetas habría sido incorrecta**. Con 30
  líneas, cero errores es compatible con un error real de hasta un 10 % (límite al 95 %), y son todas de Elden Ring, el
  juego de letra más limpia. En esas mismas imágenes cada lector acertó la línea entera: externo 86 de 89, híbrido 85,
  PP-OCR solo 87. Es decir, **el lector externo no es mejor que los locales en letra limpia**: se equivocó tres veces
  («불쨰로», «불일이») donde los locales acertaron.
* **El sesgo de las provisionales**, que importa para leer las tablas: una imagen solo tiene etiqueta provisional si
  PP-OCR solo o el híbrido de hoy la leyeron igual que el externo. Entre esos dos la comparación es limpia (la regla es
  simétrica). Para cualquier otro candidato juega en contra: donde los de hoy acertaron solo puede empatar o perder, y
  las imágenes donde los de hoy fallan, que es donde podría ganar, son justo las 47 sin etiqueta.

**Listo para `tools/label_review.html`.** Abre la carpeta `evalset\raw`: las imágenes donde los lectores discrepan van
primero, y el cuadro empieza con el texto provisional (o, si no hay, con el del lector externo). Lo que confirmes con
Enter pasa a `name.txt` y cuenta como tuyo.

## Los candidatos (tarea 3)

Cada uno quedó como opción, apagada. «Cómo se activa» es lo que habría que escribir bajo `[ocr]`.

| | Candidato | De dónde | Cómo se activa |
|---|---|---|---|
| a | Lector PaddleOCR-VL-1.6 (misma arquitectura que 1.5) | GGUF oficial de PaddlePaddle, con su proyector | `reader_dir = "…/models/vlm/paddleocr-vl-1.6"` |
| b | Detector PP-OCRv5 mobile | ONNX oficial de PaddlePaddle | `det_model = "…/ocr-candidates/pp-ocrv5-mobile-det/det.onnx"` |
| b | Detector PP-OCRv5 server, archivo oficial | ONNX oficial (el instalado es la conversión comunitaria del mismo modelo) | `det_model = …/pp-ocrv5-server-det/det.onnx` |
| b | Detectores PP-OCRv6 small y medium | ONNX oficial | `det_model = …`, y opcionalmente sus umbrales: `det_score_threshold = 0.2`, `det_box_threshold = 0.45`, `det_unclip_ratio = 1.4` |
| c | Reconocedor Qota-OCR | Comunidad (`atonlee/Qota-OCR`, Apache-2.0): solo su ONNX y su lista de caracteres en JSON, datos planos | `rec_model = …/qota-ocr/rec.onnx`, `dict_path = …/qota-ocr/dict.txt` |
| d | Ampliación ×2 | — | `[continuous] ocr_scale = 2` (ya existía) |
| e | Modos: PP-OCR solo, híbrido, «CPU PP-OCR» en vivo | — | `vision_reader`, `[continuous] cpu_ocr` (ya existían) |
| f | Lectores GLM-OCR, Surya 2.5, Nanonets-OCR2 1.5B dentro del híbrido | ver abajo | `reader_dir`, `reader_prompt` (y `reader_html = true` para Surya) |

Notas:

* **Los reconocedores PP-OCRv6 no se usaron para coreano**, como pedía el encargo: su lista de caracteres no tiene
  ningún hangul.
* **Qota-OCR:** su licencia permite el uso y sus archivos cumplen la regla 6 (ONNX y JSON; ningún pickle, ningún código
  del modelo se ejecutó). Su lista trae un defecto (un apóstrofo duplicado en la posición 7) que corregí al escribir
  `dict.txt`. Sus números publicados no se usaron para nada.
* **Los detectores v6 se publican con otros umbrales** (0,2 / 0,45 / 1,4) que los de v5; los medí de las dos maneras.
* **Elegir los tres lectores de (f)**, mirando las fichas de los modelos el día de la corrida (requisitos: coreano en la
  lista de idiomas, GGUF para `llama-server`, unos 3 GB de memoria de vídeo como mucho):

| Modelo | Decisión | Motivo |
|---|---|---|
| GLM-OCR (Zhipu) | **tomado** | Coreano en su lista; GGUF de ggml-org; MIT; 1,4 GB |
| Surya 2.5 (Datalab) | **tomado**, con reservas | Coreano en su tabla; GGUF oficial; 0,8 GB. Responde en HTML (Maru le quita las etiquetas). Licencia: investigación, uso personal y empresas pequeñas |
| Nanonets-OCR2 1.5B | **tomado** como tercero | Coreano en su lista; Apache-2.0; solo hay GGUF de la comunidad (un archivo de datos, admitido por la regla 6) |
| HunyuanOCR-1.5 | fuera | Su ficha no nombra el coreano, y su licencia excluye expresamente a Corea del Sur, la UE y el Reino Unido |
| OvisOCR | fuera | Ni la ficha ni el informe técnico nombran el coreano; solo GGUF de la comunidad |
| TeleOCR | fuera | Su ficha lista solo chino e inglés |
| DeepSeek-OCR 2 | fuera | No lista el coreano; sin GGUF oficial de la versión 2 (el de la versión 1 pasa de 3 GB) |

## Números (tarea 4)

**Cómo se midió.** `python tools/compare_ocr.py` sobre las imágenes con etiqueta, un motor cada vez, en una carpeta de
Maru aparte (configuración por defecto, sin claves, modelos enlazados): PP-OCR en CPU, RTX 5070 Ti. Una lectura sin
cronometrar antes de cada motor. «Error de letras» ignora espacios y puntuación; los promedios son por subtítulo
distinto. Los tiempos son **por imagen** (una imagen es un subtítulo de una a tres líneas). La memoria es el pico del
programa más el servidor del modelo lector. Una comparación «mejor / peor» cuenta subtítulos, y la p es la prueba de
signos. **Había un juego abierto** (ver «En corto»). La salida de cada motor, imagen por imagen (qué leyó, cuánto
tardó), está en `…\data\run-3\reports\*.json` y `…\data\run-3\reports-gold\*.json`.

### Etiquetas de oro (Elden Ring, 30 líneas, 89 imágenes)

Las únicas que ha comprobado una persona. **Ninguna diferencia sale del azar**, salvo que GLM-OCR y Nanonets son peores.

| Motor | Subtítulos | Error de letras | Líneas exactas | No leídas | Dígitos perdidos | Negación | p50 ms | p95 ms | Memoria MB | Memoria de vídeo MB | Mejor / peor / igual que hoy | p |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| **Hoy: híbrido (PP-OCR + PaddleOCR-VL-1.5)** | 30 | 0,6 % | 95,6 % | 0 | 0 | 0 | 126 | 141 | 2410 | 2251 | — | — |
| PP-OCR solo (también el modo «solo lector rápido» en vivo) | 30 | 0,3 % | 97,8 % | 0 | 0 | 0 | 95 | 101 | 926 | 0 | 3 / 1 / 26 | 0,625 |
| PP-OCR solo, en la tarjeta gráfica | 30 | 0,3 % | 97,8 % | 0 | 0 | 0 | 38 | 60 | 1036 | 598 | 3 / 1 / 26 | 0,625 |
| PaddleOCR-VL-1.5 solo | 30 | 0,4 % | 96,7 % | 0 | 0 | 0 | 120 | 134 | 2149 | 2251 | 1 / 0 / 29 | 1,000 |
| Híbrido, ampliado ×2 | 30 | 0,4 % | 96,7 % | 0 | 0 | 0 | 125 | 140 | 2407 | 2251 | 1 / 0 / 29 | 1,000 |
| PP-OCR solo, ampliado ×2 | 30 | 0,1 % | 98,9 % | 0 | 0 | 0 | 105 | 111 | 926 | 0 | 4 / 0 / 26 | 0,125 |
| Híbrido con lector PaddleOCR-VL-1.6 | 30 | 0,5 % | 95,6 % | 0 | 0 | 0 | 128 | 141 | 2410 | 2251 | 1 / 1 / 28 | 1,000 |
| PP-OCR solo, detector v5 server (archivo oficial) | 30 | 0,3 % | 97,8 % | 0 | 0 | 0 | 95 | 102 | 922 | 0 | 3 / 1 / 26 | 0,625 |
| PP-OCR solo, detector v5 mobile | 30 | 1,5 % | 91,1 % | 0 | 0 | 0 | 25 | 40 | 688 | 0 | 2 / 4 / 24 | 0,688 |
| PP-OCR solo, detector v6 small | 30 | 0,2 % | 98,9 % | 0 | 0 | 0 | 26 | 34 | 683 | 0 | 4 / 1 / 25 | 0,375 |
| PP-OCR solo, detector v6 small, sus umbrales | 30 | 0,2 % | 98,9 % | 0 | 0 | 0 | 29 | 37 | 686 | 0 | 4 / 1 / 25 | 0,375 |
| PP-OCR solo, detector v6 medium | 30 | 0,9 % | 95,6 % | 0 | 0 | 0 | 51 | 58 | 815 | 0 | 4 / 3 / 23 | 1,000 |
| PP-OCR solo, detector v6 medium, sus umbrales | 30 | 0,9 % | 96,7 % | 0 | 0 | 0 | 52 | 65 | 828 | 0 | 4 / 3 / 23 | 1,000 |
| Híbrido, detector v5 mobile | 30 | 1,5 % | 93,3 % | 0 | 0 | 0 | 122 | 136 | 2174 | 2251 | 1 / 2 / 27 | 1,000 |
| Híbrido, detector v6 medium, sus umbrales | 30 | 1,2 % | 94,4 % | 0 | 0 | 0 | 122 | 137 | 2315 | 2251 | 1 / 2 / 27 | 1,000 |
| PP-OCR solo, reconocedor coreano v5 (archivo oficial) | 30 | 0,3 % | 97,8 % | 0 | 0 | 0 | 95 | 103 | 927 | 0 | 3 / 1 / 26 | 0,625 |
| PP-OCR solo, reconocedor Qota-OCR | 30 | 0,2 % | 97,8 % | 0 | 0 | 0 | 89 | 95 | 984 | 0 | 4 / 2 / 24 | 0,688 |
| Híbrido, reconocedor Qota-OCR | 30 | 0,4 % | 96,7 % | 0 | 0 | 0 | 124 | 140 | 2465 | 2251 | 1 / 0 / 29 | 1,000 |
| Híbrido con lector GLM-OCR | 30 | 15,1 % | 16,7 % | 0 | 0 | 1 | 96 | 104 | 3110 | 2437 | 0 / 26 / 4 | 0,000 |
| Híbrido con lector Surya 2.5 | 30 | 2,0 % | 82,2 % | 0 | 0 | 0 | 131 | 142 | 5611 | 1451 | 3 / 5 / 22 | 0,727 |
| Híbrido con lector Nanonets-OCR2 1.5B | 30 | 6,9 % | 61,1 % | 0 | 0 | 0 | 96 | 106 | 2734 | 2487 | 1 / 14 / 15 | 0,001 |

Esta tabla es de una segunda medición, solo sobre las imágenes de oro y ya sin el fotograma que no muestra subtítulo
(con él dentro, todos los motores «fallaban» esa imagen y el híbrido marcaba 1,7 %). Sirve también para ver el ruido
de los tiempos con un juego abierto: PP-OCR solo marcó 73 ms aquí la primera vez y 95 ms la segunda. La memoria es
menor que en las otras tablas porque la corrida es corta (el servidor del lector crece con las imágenes leídas).

### Etiquetas provisionales, mitad para afinar (dev)

**Provisional**: nadie ha comprobado estas etiquetas.

| Motor | Subtítulos | Error de letras | Líneas exactas | No leídas | Dígitos perdidos | Negación | p50 ms | p95 ms | Memoria MB | Memoria de vídeo MB | Mejor / peor / igual que hoy | p |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| **Hoy: híbrido (PP-OCR + PaddleOCR-VL-1.5)** | 196 | 1,6 % | 90,3 % | 0 | 0 | 0 | 160 | 244 | 4752 | 2263 | — | — |
| PP-OCR solo (también el modo «solo lector rápido» en vivo) | 196 | 0,7 % | 93,9 % | 0 | 0 | 1 | 98 | 162 | 1078 | 0 | 19 / 12 / 165 | 0,281 |
| PP-OCR solo, en la tarjeta gráfica | 196 | 0,7 % | 93,9 % | 0 | 0 | 1 | 75 | 84 | 1041 | 917 | 19 / 12 / 165 | 0,281 |
| PaddleOCR-VL-1.5 solo | 196 | 26,9 % | 87,8 % | 0 | 0 | 0 | 145 | 230 | 4359 | 2263 | 3 / 8 / 185 | 0,227 |
| Híbrido, ampliado ×2 | 196 | 2,0 % | 89,8 % | 0 | 0 | 0 | 150 | 233 | 4747 | 2263 | 1 / 2 / 193 | 1,000 |
| PP-OCR solo, ampliado ×2 | 196 | 0,9 % | 94,4 % | 0 | 0 | 1 | 97 | 181 | 1078 | 0 | 19 / 10 / 167 | 0,136 |
| Híbrido con lector PaddleOCR-VL-1.6 | 196 | 0,5 % | 95,4 % | 0 | 0 | 0 | 151 | 232 | 4721 | 2263 | 13 / 2 / 181 | 0,007 |
| PP-OCR solo, detector v5 server (archivo oficial) | 196 | 0,7 % | 93,9 % | 0 | 0 | 1 | 123 | 186 | 1072 | 0 | 19 / 12 / 165 | 0,281 |
| PP-OCR solo, detector v5 mobile | 196 | 1,0 % | 94,4 % | 0 | 0 | 1 | 42 | 62 | 733 | 0 | 19 / 11 / 166 | 0,200 |
| PP-OCR solo, detector v6 small | 196 | 0,5 % | 96,4 % | 0 | 0 | 1 | 37 | 56 | 760 | 0 | 16 / 3 / 177 | 0,004 |
| PP-OCR solo, detector v6 small, sus umbrales | 196 | 0,7 % | 95,9 % | 0 | 0 | 1 | 43 | 67 | 773 | 0 | 17 / 7 / 172 | 0,064 |
| PP-OCR solo, detector v6 medium | 196 | 1,4 % | 95,4 % | 0 | 0 | 1 | 63 | 94 | 917 | 0 | 17 / 8 / 171 | 0,108 |
| PP-OCR solo, detector v6 medium, sus umbrales | 196 | 1,1 % | 96,9 % | 0 | 0 | 1 | 70 | 105 | 928 | 0 | 18 / 5 / 173 | 0,011 |
| Híbrido, detector v5 mobile | 196 | 2,2 % | 87,2 % | 0 | 0 | 0 | 154 | 239 | 4389 | 2263 | 0 / 6 / 190 | 0,031 |
| Híbrido, detector v6 medium, sus umbrales | 196 | 2,3 % | 88,8 % | 0 | 0 | 0 | 154 | 243 | 4601 | 2263 | 1 / 4 / 191 | 0,375 |
| PP-OCR solo, reconocedor coreano v5 (archivo oficial) | 196 | 0,7 % | 93,9 % | 0 | 0 | 1 | 129 | 201 | 1082 | 0 | 19 / 12 / 165 | 0,281 |
| PP-OCR solo, reconocedor Qota-OCR | 196 | 1,2 % | 92,9 % | 0 | 0 | 1 | 117 | 174 | 1210 | 0 | 19 / 13 / 164 | 0,377 |
| Híbrido, reconocedor Qota-OCR | 196 | 1,9 % | 89,8 % | 0 | 0 | 0 | 155 | 241 | 4912 | 2263 | 2 / 3 / 191 | 1,000 |
| Híbrido con lector GLM-OCR | 196 | 11,0 % | 28,1 % | 0 | 0 | 3 | 132 | 198 | 10339 | 2439 | 3 / 131 / 62 | 0,000 |
| Híbrido con lector Surya 2.5 | 196 | 2,6 % | 80,1 % | 0 | 0 | 0 | 156 | 231 | 10411 | 1455 | 6 / 26 / 164 | 0,001 |
| Híbrido con lector Nanonets-OCR2 1.5B | 196 | 8,9 % | 41,8 % | 0 | 0 | 2 | 129 | 203 | 4915 | 2495 | 6 / 105 / 85 | 0,000 |

### Etiquetas provisionales, mitad de juicio (held-out)

**Provisional.** Nada se afinó con esta mitad: no hubo parámetros que elegir.

| Motor | Subtítulos | Error de letras | Líneas exactas | No leídas | Dígitos perdidos | Negación | p50 ms | p95 ms | Memoria MB | Memoria de vídeo MB | Mejor / peor / igual que hoy | p |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| **Hoy: híbrido (PP-OCR + PaddleOCR-VL-1.5)** | 212 | 1,4 % | 87,7 % | 0 | 0 | 0 | 166 | 253 | 4752 | 2263 | — | — |
| PP-OCR solo (también el modo «solo lector rápido» en vivo) | 212 | 0,4 % | 93,9 % | 0 | 0 | 0 | 98 | 167 | 1078 | 0 | 26 / 13 / 173 | 0,053 |
| PP-OCR solo, en la tarjeta gráfica | 212 | 0,3 % | 94,3 % | 0 | 0 | 0 | 74 | 85 | 1041 | 917 | 26 / 12 / 174 | 0,034 |
| PaddleOCR-VL-1.5 solo | 212 | 2,2 % | 87,7 % | 0 | 0 | 0 | 151 | 229 | 4359 | 2263 | 3 / 2 / 207 | 1,000 |
| Híbrido, ampliado ×2 | 212 | 1,7 % | 84,9 % | 0 | 0 | 0 | 154 | 235 | 4747 | 2263 | 0 / 6 / 206 | 0,031 |
| PP-OCR solo, ampliado ×2 | 212 | 0,6 % | 92,0 % | 0 | 0 | 0 | 99 | 181 | 1078 | 0 | 26 / 17 / 169 | 0,222 |
| Híbrido con lector PaddleOCR-VL-1.6 | 212 | 0,8 % | 92,0 % | 0 | 0 | 0 | 158 | 239 | 4721 | 2263 | 17 / 6 / 189 | 0,035 |
| PP-OCR solo, detector v5 server (archivo oficial) | 212 | 0,4 % | 93,9 % | 0 | 0 | 0 | 124 | 190 | 1072 | 0 | 26 / 13 / 173 | 0,053 |
| PP-OCR solo, detector v5 mobile | 212 | 0,3 % | 95,3 % | 0 | 0 | 0 | 43 | 61 | 733 | 0 | 25 / 9 / 178 | 0,009 |
| PP-OCR solo, detector v6 small | 212 | 0,6 % | 94,8 % | 0 | 0 | 0 | 38 | 56 | 760 | 0 | 24 / 9 / 179 | 0,014 |
| PP-OCR solo, detector v6 small, sus umbrales | 212 | 0,5 % | 95,8 % | 0 | 0 | 0 | 45 | 67 | 773 | 0 | 26 / 9 / 177 | 0,006 |
| PP-OCR solo, detector v6 medium | 212 | 0,4 % | 96,7 % | 0 | 0 | 0 | 64 | 94 | 917 | 0 | 25 / 6 / 181 | 0,001 |
| PP-OCR solo, detector v6 medium, sus umbrales | 212 | 0,3 % | 96,7 % | 0 | 0 | 0 | 72 | 104 | 928 | 0 | 26 / 7 / 179 | 0,001 |
| Híbrido, detector v5 mobile | 212 | 1,4 % | 87,7 % | 0 | 0 | 0 | 159 | 242 | 4389 | 2263 | 0 / 0 / 212 | 1,000 |
| Híbrido, detector v6 medium, sus umbrales | 212 | 1,5 % | 87,3 % | 0 | 0 | 0 | 160 | 243 | 4601 | 2263 | 0 / 1 / 211 | 1,000 |
| PP-OCR solo, reconocedor coreano v5 (archivo oficial) | 212 | 0,4 % | 93,9 % | 0 | 0 | 0 | 131 | 199 | 1082 | 0 | 26 / 13 / 173 | 0,053 |
| PP-OCR solo, reconocedor Qota-OCR | 212 | 0,2 % | 97,2 % | 0 | 0 | 0 | 118 | 177 | 1210 | 0 | 25 / 5 / 182 | 0,000 |
| Híbrido, reconocedor Qota-OCR | 212 | 1,2 % | 88,2 % | 0 | 0 | 0 | 162 | 247 | 4912 | 2263 | 2 / 1 / 209 | 1,000 |
| Híbrido con lector GLM-OCR | 212 | 11,8 % | 22,2 % | 0 | 0 | 2 | 132 | 206 | 10339 | 2439 | 6 / 158 / 48 | 0,000 |
| Híbrido con lector Surya 2.5 | 212 | 2,1 % | 78,3 % | 0 | 0 | 0 | 161 | 227 | 10411 | 1455 | 14 / 34 / 164 | 0,006 |
| Híbrido con lector Nanonets-OCR2 1.5B | 212 | 11,3 % | 29,7 % | 0 | 0 | 3 | 131 | 204 | 4915 | 2495 | 6 / 142 / 64 | 0,000 |

Xenoblade cayó entero en la mitad para afinar (es una sola escena de 75 segundos).

### Por juego (todas las provisionales; error de letras / líneas exactas)

**Provisional.**

| Juego (subtítulos) | Hoy: híbrido | PP-OCR solo | Híbrido con VL-1.6 | PP-OCR solo, v5 mobile | PP-OCR solo, v6 small (sus umbrales) | PP-OCR solo, v6 medium | PP-OCR solo, Qota |
|---|---:|---:|---:|---:|---:|---:|---:|
| Elden Ring (86) | 1,7 % / 87,2 % | 0,5 % / 95,3 % | 0,8 % / 93,0 % | 1,7 % / 91,9 % | 0,6 % / 95,3 % | 3,1 % / 91,9 % | 0,1 % / 98,8 % |
| Ori (75) | 0,7 % / 92,0 % | 0,7 % / 88,0 % | 0,6 % / 97,3 % | 0,2 % / 96,0 % | 0,1 % / 98,7 % | 0,1 % / 98,7 % | 0,6 % / 92,0 % |
| Sekiro (114) | 1,8 % / 87,7 % | 0,3 % / 96,5 % | 0,6 % / 94,7 % | 0,5 % / 95,6 % | 0,5 % / 97,4 % | 0,5 % / 96,5 % | 0,3 % / 94,7 % |
| Stellar Blade (118) | 1,7 % / 89,0 % | 0,2 % / 95,8 % | 0,6 % / 90,7 % | 0,3 % / 94,9 % | 0,9 % / 94,9 % | 0,2 % / 97,5 % | 1,3 % / 96,6 % |
| Xenoblade (15) | 0,4 % / 93,3 % | 3,8 % / 80,0 % | 1,0 % / 93,3 % | 0,0 % / 100 % | 2,3 % / 80,0 % | 1,2 % / 93,3 % | 3,0 % / 80,0 % |

Tiempo por imagen (p50) de hoy frente a PP-OCR solo: Elden Ring 127 / 74 ms, Ori 240 / 152, Sekiro 148 / 97, Stellar
Blade 177 / 99, Xenoblade 187 / 135. En ningún juego hubo dígitos perdidos; las líneas no leídas fueron cero salvo el
fotograma de oro sin texto. **Xenoblade es una muestra diminuta: no baso nada en ella.**

### Cada pieza frente a PP-OCR solo (mismo modo de lectura)

Las tablas de arriba comparan con el híbrido de hoy, así que cualquier variante de «PP-OCR solo» hereda la ventaja de
no usar el lector 1.5. Para saber qué aporta **la pieza**, esta tabla compara con PP-OCR solo (mejor / peor, p):

| Pieza cambiada | Dev (196) | Held-out (212) | Oro (30) |
|---|---:|---:|---:|
| Detector v5 server, archivo oficial | 0 / 0 | 0 / 0 | 0 / 0 |
| Detector v5 mobile | 9 / 9 | 10 / 7 (p = 0,63) | 2 / 6 (p = 0,29) |
| Detector v6 small | 11 / 5 (p = 0,21) | 10 / 8 (p = 0,82) | 2 / 1 |
| Detector v6 small, sus umbrales | 10 / 5 (p = 0,30) | 10 / 6 (p = 0,45) | 2 / 1 |
| Detector v6 medium | 8 / 5 (p = 0,58) | 11 / 6 (p = 0,33) | 2 / 3 |
| Detector v6 medium, sus umbrales | 9 / 4 (p = 0,27) | 11 / 6 (p = 0,33) | 2 / 3 |
| Reconocedor coreano v5, archivo oficial | 0 / 0 | 0 / 0 | 0 / 0 |
| Reconocedor Qota-OCR | 5 / 9 (p = 0,42) | 10 / 3 (p = 0,09) | 2 / 2 |
| Ampliación ×2 | 8 / 7 | 6 / 11 (p = 0,33) | 2 / 0 |
| PP-OCR en la tarjeta gráfica | 0 / 0 | 1 / 0 | 0 / 0 |
| Híbrido con VL-1.6 (frente a PP-OCR solo) | 12 / 9 (p = 0,66) | 12 / 16 (p = 0,57) | 1 / 3 |

(La columna de oro es de la primera medición, con el fotograma sin subtítulo todavía dentro; cuenta subtítulos, así que
no cambia.) **Ninguna pieza se distingue de la actual en lectura.** Se distinguen en tiempo: detector v6 small 38 ms,
v5 mobile 43 ms, v6 medium 64 ms, frente a 98 ms del detector de hoy (p50, held-out, PP-OCR solo en CPU).

### Las 47 imágenes sin etiqueta (complementario, no es una puntuación)

Ahí los dos lectores de hoy discrepan del externo, por definición. En 34 de ellas el externo dio una transcripción.
¿Cuántas veces coincide cada motor con él, en letras? El externo no es una etiqueta (se equivoca más o menos como los
locales), así que esto solo dice **dónde mirar**:

| Motor | Coincide con el lector externo |
|---|---:|
| Hoy: híbrido; PP-OCR solo; archivos oficiales de detector y reconocedor | 0 de 34 |
| PP-OCR solo, Qota-OCR | 15 de 34 |
| PP-OCR solo, detector v5 mobile; v6 small; v6 medium | 13 de 34 cada uno |
| PP-OCR solo, v6 small con sus umbrales / v6 medium con sus umbrales | 12 / 11 de 34 |
| PP-OCR solo, ampliado ×2 | 9 de 34 |
| Híbrido con detector v5 mobile; con Qota-OCR | 8 de 34 cada uno |
| PaddleOCR-VL-1.5 solo; híbrido con detector v6 medium | 7 de 34 cada uno |
| Híbrido con lector Surya 2.5 | 6 de 34 |
| Híbrido con lector PaddleOCR-VL-1.6 | 5 de 34 |
| Híbrido ampliado ×2 | 4 de 34 |
| Híbrido con GLM-OCR / con Nanonets | 1 / 0 de 34 |

Los otros detectores y Qota leen igual que el externo en un tercio largo de las imágenes donde los de hoy no. Si tus
etiquetas le dan la razón al externo ahí, esos candidatos saldrán mejor que en las tablas provisionales.

### Lo que cuesta cada modo de lectura (tarea 3e)

Sobre las 408 imágenes provisionales:

| Modo | p50 ms | p95 ms | Memoria (pico) | Memoria de vídeo (pico) |
|---|---:|---:|---:|---:|
| Híbrido (hoy): PP-OCR en CPU + PaddleOCR-VL-1.5 | 163 | 253 | 4.752 MB | 2.263 MB |
| PP-OCR solo, en CPU | 98 | 164 | 1.078 MB | 0 |
| PP-OCR solo, en la tarjeta gráfica (DirectML) | 75 | 85 | 1.041 MB | 917 MB |
| «CPU PP-OCR» en vivo (`continuous.cpu_ocr`) | 98 | 164 | como PP-OCR solo, **más** el lector, que sigue cargado para capturas | 2.263 MB (no se libera) |

En tu PC, «CPU PP-OCR» en vivo y «PP-OCR solo» son el mismo motor (tu `ocr.device` ya es `cpu`): leen igual. La
diferencia es que con el interruptor el lector 1.5 sigue cargado para las capturas, de modo que ahorra tiempo pero no
memoria de vídeo.

## Qué leyó mal cada motor

Ejemplos de las corridas guardadas (etiqueta → lo leído). Salvo indicación, etiquetas provisionales.

* **Híbrido de hoy (lector PaddleOCR-VL-1.5): cambia letras que PP-OCR había leído bien.** 48 imágenes donde el híbrido
  falla y PP-OCR solo acierta: «늑대여, 잘 찾아주었다» → «느낌에, 잘 찾아주었다»; «왜 울지?» → «왜 을지?»; «하오나…» →
  «하오니»; «좋군그래» → «종군그래»; «…신향인가» → «…선향인가»; «아아, 얘야…» → «아아, 애야…».
* **PP-OCR solo: lee marcas del fondo como letras.** 26 imágenes donde falla y el híbrido acierta, casi todas por letras
  latinas o cifras sueltas que añade al principio o al final: «원숭이가 셋…» → «M 원숭이가 셋»; «비옥한 땅인 굴라조차» →
  «비옥한 땅인 굴라조차 0 0»; «영원한 여왕 마리카는 숨었고» → «… 숨었고 C». Con la letra de Xenoblade también falla
  letras: «그건 우리뿐만이 아니에요» → «근건우리뿐만이 아니에요 0 0». (El filtro de marcas sueltas de los subtítulos en
  vivo quita parte de esto antes de traducir; aquí se mide la lectura cruda.)
* **PaddleOCR-VL-1.5 solo (sin detector): a veces se desboca.** En el fotograma de oro sin subtítulo escribió
  «1000000000…» hasta el límite, y en una línea de Stellar Blade «0.0000000…». Por eso su error medio en las tablas es
  enorme con casi las mismas líneas exactas. Dentro del híbrido esas respuestas se descartan.
* **Híbrido con PaddleOCR-VL-1.6:** arregla 29 imágenes que el 1.5 estropea (las mismas de arriba: «늑대여…», «하오나»,
  «좋군그래») y estropea 10 que el 1.5 leía bien: «폐하? 왜 그러시죠?» → «떼하? …»; «아시나를 벗어나야 합니다» →
  «아시니를 …»; y dos fotogramas de oro: «첫 엘데의 왕이니라» → «찻엘데의 왕이니라», «…모르고트의 이름을!» →
  «…묘르고트의 이름을!».
* **Detectores v5 mobile y v6 medium:** encuentran más «texto» en el fondo de Elden Ring: «전사이니라!» → «WE
  전사이니라!» / «W 전사이니라! CEANN»; (oro) «빛바랜 자여» → «GA 빛바랜 자여».
* **Qota-OCR:** como el reconocedor de hoy, con algún añadido («우와... 여긴...» → «HE HHH 우와... 여긴.») y alguna letra
  («나무가 기억한 거야.» → «나무가 기역한 거야.»).
* **Ampliación ×2:** añade marcas («우린 버려진 것이다...» → «N7 우린 버려진 것이다..») y parte palabras («태어나렴» →
  «태어나 럼»).
* **GLM-OCR:** falla letras en casi todas las líneas: «이, 이겼어. 굉장해.» → «이, 이것어. 김정해.»; «이브! 괜찮아?» →
  «이브! 패창아?».
* **Nanonets-OCR2:** mezcla otros alfabetos: «…언젠가 다시, 함께 돌아가리…» → «ns가 다시, 함께 들어가리…»; «이 올빼미,
  황자님의…» → «이 올控行이, 환자님의…».
* **Surya 2.5:** mejor que los dos anteriores y peor que PaddleOCR-VL: «왜 울지?» → «왜 올지?»; «………그런…가» → «그런».
* **Lector externo (Claude Sonnet 5.5):** en las imágenes de oro, «불째로» → «불쨰로» y «볼일이» → «불일이». En once
  imágenes de Ori contestó con un comentario en inglés en vez de no escribir nada, pese a la instrucción.

## Decisión (tarea 5)

**No se adopta nada.** La regla del encargo era: mejor en la mitad de juicio con prueba de signos significativa, y
tiempo y memoria de vídeo dentro del 25 % de los de hoy; y con etiquetas provisionales, aun así, no cambiar.

**Listas para cambiar cuando tus etiquetas lo confirmen** (cumplen la regla con etiquetas provisionales):

1. **Lector PaddleOCR-VL-1.6 en lugar de 1.5.** Mitad de juicio: mejor en 17, peor en 6 (p = 0,035); dev: 13 a 2
   (p = 0,007). Tiempo 158 ms frente a 166; memoria de vídeo idéntica (2.263 MB). Es el mismo modelo reentrenado, GGUF
   oficial. En oro: 1 a 1. Es un cambio de una línea (`reader_dir`).
2. **Para subtítulos en vivo, PP-OCR solo** (el interruptor que ya existe, «Read live subtitles with the quick reader
   only»), opcionalmente con un detector más ligero. Mitad de juicio frente a hoy: con detector v6 medium, mejor en 25 y
   peor en 6 (p = 0,001); v6 small con sus umbrales, 26 a 9 (p = 0,006); v5 mobile, 25 a 9 (p = 0,009); con el detector
   de hoy, 26 a 13 (p = 0,053, no llega). Menos tiempo (38 a 98 ms frente a 166) y nada de memoria de vídeo propia.
   **Dos reservas:** la ganancia viene de no usar el lector 1.5, no del detector (ver la tabla «frente a PP-OCR solo»),
   y cambiar `det_model` cambia también el detector del escaneo de pantalla, que esta corrida no midió.

Las dos van en direcciones distintas (un lector mejor, o ningún lector en vivo). Con tus etiquetas se verá cuál: en la
mitad de juicio, el híbrido con 1.6 frente a PP-OCR solo queda 12 a 16.

**Disponibles como opción, apagadas, sin motivo para encenderlas:** Qota-OCR, los archivos ONNX oficiales (leen
idéntico a los instalados y, en esta PC, tardan un 25 % más), la ampliación ×2, y los lectores GLM-OCR, Surya 2.5 y
Nanonets-OCR2.

**El comando único**, para los números finales cuando hayas etiquetado (cierra los juegos antes; tarda unos 25 minutos):

```
python tools\compare_ocr.py "%APPDATA%\maru\Maru\data\evalset\raw"
```

Usa solo tus etiquetas (`name.txt`), corre el motor de hoy y los 20 candidatos e imprime las mismas tablas: todo, la
mitad para afinar y la mitad de juicio. `--tables` las vuelve a imprimir sin releer nada; `--only "1.6" "v6 medium"`
corre unos pocos; `--labels all` añade las provisionales, marcadas.

## Descargas

Todo se bajó de Hugging Face, sin credenciales, y cada archivo grande se comprobó contra el sha256 que publica el Hub.
Están en `%APPDATA%\maru\Maru\data\models\` (6,06 GB); la lista completa, con el hash entero, en
`…\data\run-3\downloads.json`.

| Archivo (bajo `models\`) | Repositorio | Editor oficial | Licencia | Bytes | sha256 |
|---|---|---|---|---:|---|
| `ocr-candidates/pp-ocrv5-mobile-det/det.onnx` | `PaddlePaddle/PP-OCRv5_mobile_det_onnx` | sí | Apache-2.0 | 4.826.518 | `a431985659dc9219…` |
| `ocr-candidates/pp-ocrv5-server-det/det.onnx` | `PaddlePaddle/PP-OCRv5_server_det_onnx` | sí | Apache-2.0 | 88.116.791 | `10803475a591f7dc…` |
| `ocr-candidates/pp-ocrv6-small-det/det.onnx` | `PaddlePaddle/PP-OCRv6_small_det_onnx` | sí | Apache-2.0 | 9.880.512 | `d73e0058b7a8086b…` |
| `ocr-candidates/pp-ocrv6-medium-det/det.onnx` | `PaddlePaddle/PP-OCRv6_medium_det_onnx` | sí | Apache-2.0 | 62.032.837 | `eb13b44b25bb36f8…` |
| `ocr-candidates/korean-pp-ocrv5-mobile-rec/rec.onnx` | `PaddlePaddle/korean_PP-OCRv5_mobile_rec_onnx` | sí | Apache-2.0 | 13.418.787 | `92f0b7785e64fc90…` |
| `ocr-candidates/qota-ocr/rec.onnx` | `atonlee/Qota-OCR` | no (comunidad) | Apache-2.0 | 26.743.431 | `27ccd15d20a75365…` |
| `ocr-candidates/qota-ocr/preprocessor_config.json` | `atonlee/Qota-OCR` | no (comunidad) | Apache-2.0 | 333.099 | `fffcd60e1d556f25…` |
| `vlm/paddleocr-vl-1.6/PaddleOCR-VL-1.6.gguf` | `PaddlePaddle/PaddleOCR-VL-1.6-GGUF` | sí | Apache-2.0 | 935.769.056 | `f3ae46ec885050ac…` |
| `vlm/paddleocr-vl-1.6/PaddleOCR-VL-1.6-mmproj.gguf` | `PaddlePaddle/PaddleOCR-VL-1.6-GGUF` | sí | Apache-2.0 | 881.770.560 | `204d757d7610d9b3…` |
| `vlm/glm-ocr/GLM-OCR-Q8_0.gguf` | `ggml-org/GLM-OCR-GGUF` | sí | MIT | 950.433.408 | `45bc244a6446aff8…` |
| `vlm/glm-ocr/mmproj-GLM-OCR-Q8_0.gguf` | `ggml-org/GLM-OCR-GGUF` | sí | MIT | 484.403.648 | `9c4b58e33e316ed1…` |
| `vlm/surya-2.5/surya-2.5-Q8_0.gguf` | `datalab-to/surya-ocr-2.5-gguf` | sí | AI Pubs OpenRAIL-M modificada (investigación. uso personal. empresas pequeñas) | 628.680.096 | `5ab34c76845f66d2…` |
| `vlm/surya-2.5/surya-2.5-mmproj.gguf` | `datalab-to/surya-ocr-2.5-gguf` | sí | AI Pubs OpenRAIL-M modificada (investigación. uso personal. empresas pequeñas) | 207.345.920 | `f245dbf784cf7368…` |
| `vlm/nanonets-ocr2-1.5b/Nanonets-OCR2-1.5B-exp.Q8_0.gguf` | `mradermacher/Nanonets-OCR2-1.5B-exp-GGUF` | no (comunidad) | Apache-2.0 | 1.049.711.040 | `0d9587029ad1fd35…` |
| `vlm/nanonets-ocr2-1.5b/mmproj-Nanonets-OCR2-1.5B-exp.Q8_0.gguf` | `mradermacher/Nanonets-OCR2-1.5B-exp-GGUF` | no (comunidad) | Apache-2.0 | 712.893.984 | `b9d22d8a084fd127…` |

GLM-OCR se bajó de `ggml-org` (los autores de llama.cpp), no del repositorio de Zhipu, que no publica GGUF. Qota-OCR y
el GGUF de Nanonets son de la comunidad: solo archivos de datos. No se ejecutó código de ningún modelo ni se cargó
ningún pickle. `qota-ocr\dict.txt` lo escribí yo a partir de su `preprocessor_config.json`. Puedes borrar cualquiera de
estas carpetas; nada de lo instalado depende de ellas.

## Dinero gastado

| Qué | Imágenes | Tokens entrada / salida | USD |
|---|---:|---|---:|
| Lector externo, Xenoblade | 16 | 7.520 / 385 | 0,019 |
| Lector externo, Elden Ring | 320 | 100.480 / 11.930 | 0,320 |
| Lector externo, Ori | 140 | 79.240 / 5.664 | 0,215 |
| Lector externo, Sekiro | 124 | 46.376 / 2.748 | 0,120 |
| Lector externo, Stellar Blade | 125 | 52.750 / 3.686 | 0,142 |
| **Total** | 725 | 286.366 / 24.413 | **≈ 0,82** |

Claude Sonnet 5.5 a 2 / 10 USD por millón de tokens (precio de lista de Anthropic, comprobado el día de la corrida).
Los tokens son los que informó el servicio. Ningún pedido falló ni se repitió. No se usó ningún otro proveedor.

## Defectos encontrados y corregidos

1. **La división dev / held-out ponía todas las escenas de un vídeo en el mismo lado.** `split_of` tomaba un bit del
   hash FNV que no depende de los últimos caracteres de la clave, y las escenas de un juego solo difieren ahí (`sb03`,
   `sb04`; también los nombres por reloj de «guardar lo leído»). En mi set, la mitad de juicio era Stellar Blade entero
   y nada más. Ahora el hash se mezcla antes de tomar el bit, hay una prueba, y `eval-report` calcula la mitad a partir
   del nombre de la imagen, así que un informe guardado antes se parte igual. **Vi las tablas con la división vieja
   antes de corregirla**; las dejé en `…\run-3\reports\tables-before-split-fix.md`. No había nada afinado que pudiera
   contaminarse, y con ninguna de las dos divisiones cambia un valor por defecto.
2. **`maru eval-ocr` abría el historial y las demás bases de datos en modo normal.** Ahora arranca en solo lectura.
3. **La fusión dejaba el vigilante en vivo sin una de las dos cosas** (no esperar al lector, o ampliar). Resuelto con
   `recognize_now_scaled`, con pruebas.
4. **Números de prueba manual duplicados** tras la fusión (M-14, M-15, M-16).
5. **Los valores de demostración de Settings** no traían los dos ajustes nuevos (el campo de ampliación salía vacío).
6. **Errores míos**, corregidos durante la corrida: las 38 imágenes de rótulos cortados (arriba); medí una vez con ellas
   dentro y repetí la medición entera sin ellas; durante la primera medición calculé hashes de archivos grandes, que
   pudo ensuciar algún tiempo (la tabla de oro es de una segunda medición).
7. **Un nombre tuyo en mi salida de consola:** al listar los nombres de tus conexiones guardadas apareció el de la de
   Azure. No está en ningún archivo ni commit; solo en el registro de esta sesión.

Observado y no corregido:

* Los archivos ONNX oficiales de PaddlePaddle leen exactamente igual que los instalados (0 subtítulos distintos) y
  tardan más en esta PC (124 ms frente a 98). No investigué por qué.
* El lector externo a veces comenta en vez de callar, y el cuadro de la página de etiquetado empieza entonces con ese
  comentario (once imágenes de Ori).
* El vídeo de Ori es de baja resolución reescalada; sus letras son más borrosas que en el juego real.

## Qué está verificado y cómo

| Qué | Cómo | Resultado |
|---|---|---|
| «Definition of done» | en esta PC, sobre el árbol final | verde: clippy (workspace y app); 803 pruebas del workspace (9 opcionales omitidas), 45 de la app, 189 de UI, 112 de `tools/` y las de la extensión |
| Las cajas de subtítulos | fotogramas reales de cada vídeo, a resolución completa, con una rejilla | cuatro cajas correctas, la de Ori errónea, la de Xenoblade medida |
| El buscador de subtítulos | contra las 30 líneas verificadas | encontró 29 |
| La regla de etiquetas provisionales | aplicada a las 89 imágenes de oro | 85 etiquetadas, 0 incorrectas |
| Cada candidato | `maru eval-ocr` con los modelos reales, 498 imágenes con etiqueta | 21 motores medidos; salidas guardadas |
| Descargas | sha256 de cada archivo grande contra el que publica el Hub | todos coinciden |
| Qué modelos usa Maru hoy | sha256 de los archivos instalados contra los del Hub | detector = `detection/v5/det.onnx` de `monkt/paddleocr-onnx` (PP-OCRv5 server); lector = GGUF oficial de PaddleOCR-VL-1.5 |
| Los dos ajustes en Settings | Edge sin interfaz, datos de demostración | aparecen en Live subtitles; **nadie hizo clic** |

## Sin verificar

* **Las 408 etiquetas provisionales.** Nadie las ha mirado. La regla no falló en las 30 líneas de oro, que son un solo
  juego.
* **Todo lo que dicen las tablas provisionales**, por lo mismo y por el sesgo descrito arriba.
* **Las 30 líneas de oro** las tomé como verdaderas; a ojo coinciden con la hoja de muestra que miré (una docena),
  pero no las comprobé una por una (salvo el fotograma sin subtítulo, que sí miré).
* **Los tiempos con la PC en reposo**: había un juego abierto (M-54).
* **Ninguna opción nueva de `[ocr]` se probó en la app**: solo por `maru eval-ocr` y pruebas unitarias (M-52). La app
  no se arrancó en esta corrida.
* **Subtítulos en vivo de verdad.** Todo se midió sobre imágenes recortadas de vídeos de YouTube (comprimidos) y una
  grabación tuya, no sobre un juego en marcha: sin el vigilante, sin fundidos, sin el filtro de marcas sueltas.
* **El escaneo de pantalla y las capturas** con otro detector o sin lector: no medidos. La medición anterior de la
  página (capturas de pantalla, letra estilizada) decía lo contrario para ese uso: ahí el lector ayudaba mucho.
* **Los dos ajustes de Settings**: vistos sin interfaz; nadie hizo clic (M-53).
* **El código exclusivo de Windows** nuevo (la medición de memoria usa el monitor de recursos de `maru-platform`) solo
  se ejecutó en esta PC.
* **El último push**: su CI estaba en curso al escribir esto.

## Lo que solo tú puedes comprobar

1. **Etiquetar** (M-51; una hora larga). Abre `tools\label_review.html` en Edge o Chrome, «Open the folder…», elige
   `%APPDATA%\maru\Maru\data\evalset\raw`. Primero salen las imágenes donde los lectores discrepan (las 47 sin
   etiqueta están ahí). Corrige y Enter; Alt+U para lo que no sea un subtítulo (el mapa de Ori con «필터: 전체»).
   Las provisionales se confirman con un Enter cada una.
2. **El comando único** (arriba), con los juegos cerrados. Mira la tabla de la mitad de juicio: ¿sigue el lector 1.6
   mejor que el 1.5?, ¿sigue PP-OCR solo por delante del híbrido?
3. **Si el 1.6 se confirma**, pruébalo en la app antes de pedirme que lo deje por defecto (M-52): `reader_dir` bajo
   `[ocr]`, reinicia, mira un par de capturas y una escena en vivo.
4. **Si PP-OCR solo se confirma para subtítulos**, enciende Settings → Live subtitles → «Read live subtitles with the
   quick reader only» y juega una escena de Xenoblade y otra de un juego de letra limpia. En Xenoblade puede ir peor.
5. **Tiempos en reposo** (M-54): cierra `ssbu.exe` y corre
   `python tools\compare_ocr.py "%APPDATA%\maru\Maru\data\evalset\raw" --only "PP-OCR alone (also" "1.6"`.
6. **Los dos ajustes de Settings** (M-53).
7. **Decide qué borrar**: los modelos candidatos (6 GB en `models\ocr-candidates` y `models\vlm\{paddleocr-vl-1.6,
   glm-ocr,surya-2.5,nanonets-ocr2-1.5b}`) y `…\data\run-3\` (corridas guardadas, la copia de `maru` que usé y una
   carpeta `home` con dos enlaces de directorio a `models` y `bin`: **quítalos con `cmd /c rmdir`**, no con un borrado
   recursivo, que borraría los modelos de verdad).

## Git

* Rama `feature/run-3-ocr`, subida a `origin`. No toqué `main`, no reescribí historia, no fusioné ningún pull request.
* La fusión de `origin/main` es un commit de fusión (`31e7385`), sin rebase.
* El pull request es **borrador** y apunta a `feature/run-2`, como pedía el encargo. Su diff incluye los cuatro commits
  de `main` que la corrida 2 no tenía.
* `.claude/`, `package.json` y `package-lock.json` siguen sin seguimiento en tu carpeta: no son de esta corrida.

## Propuestas para la corrida 4

1. **Decidir con tus etiquetas** entre «lector 1.6» y «sin lector en vivo», y quizá las dos: 1.6 para capturas y
   escaneo, PP-OCR solo para subtítulos.
2. **Quitar las marcas sueltas de PP-OCR en la lectura misma**, no solo en vivo: son casi todos sus errores («M», «0 0»,
   «C» al borde de la línea). Con eso PP-OCR solo quedaría claramente por delante en letra limpia.
3. **Que el híbrido no reemplace una línea que PP-OCR leyó con mucha confianza** por otra del lector que difiere en una
   sola sílaba: es exactamente el fallo que mide esta corrida («늑대여» → «느낌에»).
4. **Elegir el modo por juego**: Xenoblade pide lector; los otros cuatro no. El perfil por programa ya existe.
5. **Medir el escaneo de pantalla** con los detectores v6 antes de proponerlos: son entre un 35 % y un 60 % más rápidos
   y aquí leen igual, pero un menú no es un subtítulo.
6. **Un set con tus propias partidas** («Save the subtitles it reads»): los vídeos de YouTube están comprimidos y dos
   de ellos traían rótulos del autor que hubo que apartar.
7. **Que el lector externo no pueda comentar**: pedirle una respuesta con formato fijo, o descartar las que no tengan
   hangul, antes de usarlo para más etiquetas.

## Dónde paré

Las tareas 0 a 6 están hechas. Lo que queda es tuyo: etiquetar y correr el comando. El estado final del CI y del pull
request está en la descripción del propio pull request.
