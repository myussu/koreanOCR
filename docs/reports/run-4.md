# Maru — Run 4: adoptar lo de OCR, medir lo que faltaba, dialectos de Kiwi

Informe de la corrida desatendida del 26 al 27 de noviembre de 2026 (hora de Lima). Rama: `feature/run-4`, creada desde
`2ce09fc` (la punta de `feature/run-3-ocr`). Todo lo que dice «medido» lo midió el agente en esta PC; lo que no, está en
«Sin verificar». Nadie respondió preguntas: donde había ambigüedad tomé la lectura más conservadora y lo digo.

## En corto

* **Subtítulos en vivo: PP-OCR solo con el detector PP-OCRv6 small, sin modelo lector.** Es el nuevo valor por defecto,
  como decidiste. El lector no ocupa memoria de vídeo mientras solo corren los subtítulos: medido en la app real, ningún
  `llama-server` con subtítulos en marcha; aparece con la primera captura (2.254 MB).
* **Capturas: híbrido con PaddleOCR-VL-1.6** (antes 1.5). En 73 pantallas completas no es peor que 1.5 (15 líneas mejor,
  12 peor, p = 0,70), y en tus subtítulos es mejor (38 a 12).
* **Cada tarea elige cómo lee** (`[reading]`): detector, reconocedor, lector sí/no y qué lector, para subtítulos y para
  capturas; un `[[profile]]` puede cambiarlo para un juego (p. ej. Xenoblade con lector en vivo). Tu `config.toml` se migra
  solo al primer arranque, con copia; lo que tú hubieras elegido se queda. **No toqué tu archivo**: lo hará la app.
* **El escaneo de pantalla nunca usó el lector**: lee con PP-OCR solo a 2048 px. El encargo suponía el híbrido. No lo cambié.
* **Adoptado por medición (tarea 2c):** en el híbrido, una línea que PP-OCR leyó con confianza ≥ 0,90 conserva su texto si el
  lector cambia una sola sílaba. Mitad de juicio: 16 mejor, 4 peor (p = 0,012), mismo tiempo; en pantallas, 9 a 2.
* **No adoptado (tarea 2b):** quitar marcas sueltas («M», «0», «FIRST») en la lectura. Nunca empeoró nada (0 de 468), pero con el
  detector v6 small de vivo la mitad de juicio queda en 5 a 0 (p = 0,062): no llega. Queda como opción apagada.
* **Qota-OCR sobre v6 small (tarea 2a):** mitad de juicio 15 a 3 (p = 0,008), pero en dev 7 a 6. Es de la comunidad: apagado.
* **Kiwi:** la instalada es 0.23.1 y sigue siendo la más nueva que pasa la prueba de Maru aquí: 0.23.2, 0.24.0, 0.24.1 y 0.25.0
  se caen (también con kiwi-rs más nuevo). Los dialectos funcionan: con Gyeongsang cambian 10 formas de 239 en las 50 líneas
  de la corrida 2 (0 en texto estándar); 주꾸마 → 주다; 잡그래이 no cambia. La corrección de erratas **hace caer a Kiwi**: queda
  detrás de una prueba en otro proceso, así que no puede tumbar a Maru, y hoy queda apagada.
* **El set de pruebas ya se abre desde el navegador:** por defecto `Documents\maru-evalset\raw`; la página de etiquetado
  muestra el nombre del archivo, marca las provisionales, filtra «sin etiqueta alguna» y tiene «No es un subtítulo» (Alt+N).
* **Clics de verdad:** una prueba automática arranca la app real, recorre los 8 apartados de Settings, cambia y guarda un
  ajuste en cada uno (comprobado en `config.toml`) y abre Make cards. Sin tocar el escritorio. Lo que no clica está dicho.
* **Gasto en APIs: USD 0,54** de 3 (Claude Sonnet como lector externo de las pantallas; las imágenes salieron de esta PC).

## Estado inicial (tarea 0)

| | |
|---|---|
| Rama | `feature/run-4` desde `2ce09fc`; árbol limpio salvo `.claude/`, `package.json`, `package-lock.json` sin seguimiento (no los toqué) |
| Pruebas de `feature/run-3-ocr` | verde: clippy (workspace y app); 803 pruebas del workspace (9 opcionales omitidas), 45 de la app, 189 de UI, 112 de `tools/` |
| Herramientas | rustc 1.99.0, node 24.18.0, Python 3.12.10, ffmpeg, Edge 158 |
| Carga de la PC | ningún juego abierto; Brave, Notion y Anki abiertos todo el tiempo; Steam abierto (sin juego) en la tarea 3 |

**Tus números, reproducidos** con `--tables` sobre una copia de `Documents\maru-evalset\reports`. `compare_ocr.py --labels all`
no imprimía la vista del set entero (lo arreglé en la tarea 4), así que la saqué con `maru eval-report` sobre las mismas
corridas. Error de letras, mediana de ms y mejor/peor coinciden exactamente en las seis filas. «Líneas exactas» sale entre
0,4 y 0,6 puntos más alto: tú contaste un subtítulo como exacto solo si **todas** sus imágenes lo eran (con esa cuenta salen
tus 83,5 / 88,5 / 88,7 / 92,5 / 92,7 / 92,7); la herramienta promedia la proporción de imágenes exactas por subtítulo. Las
dos son correctas; las tablas de abajo usan la de la herramienta. La mitad de juicio confirma: frente a hoy, PP-OCR solo
30/14 (p = 0,023), híbrido 1.6 20/6 (0,009), Qota 36/6, v6 medium 34/6, v6 small 34/9 (todas p < 0,001 salvo las dos primeras).

## Qué cambió

| Commit | Qué |
|---|---|
| `6a5c5ec` | Tarea 1: `[reading]` por tarea, motores por tarea, carga del lector bajo demanda, migración, instalaciones con hash, Settings |
| `14ba23e` | Tarea 2: marcas sueltas (`maru_ocr::stray`, apagado) y «línea segura» en el híbrido (`keep_sure_lines`) |
| `448a3ab` | Tarea 3: set de pantallas, `tools/scan_lines.py`, `eval-ocr --scan`; capturas con PaddleOCR-VL-1.6; `keep_sure_lines = 0.90` |
| `eef1acc` | Tarea 4: set en Documents, página de etiquetado, `compare_ocr.py`, lector externo con formato fijo |
| `904ffca` | Tarea 5: dialectos y corrección de erratas por programa en Kiwi, prueba aparte, página de cambios |
| `d492eac` | Tarea 6: `tools/e2e_app.mjs`, prueba de punta a punta de la app real |
| último commit | Tarea 7: este informe, `ARCHITECTURE.md`, `README.md`, `MANUAL_TESTS.md` (M-55 a M-60) |

### Cómo lee cada tarea (tarea 1)

* **`[reading.live]` y `[reading.capture]`** (`maru-core/src/reading.rs`): `detector` (`standard`, `pp-ocrv6-small`,
  `pp-ocrv6-medium`, `pp-ocrv5-mobile` o una ruta), `recogniser` (`standard`, `qota-ocr` o una ruta), `reader` (sí/no) y
  `reader_model` (`paddleocr-vl-1.5`, `paddleocr-vl-1.6` o una carpeta). Vacío = lo que diga `[ocr]`. Las capturas incluyen la
  selección leída de píxeles, la región y hover; el escaneo usa el detector y el reconocedor de capturas, sin lector.
* **Por programa:** `[profile.reading.live]` / `[profile.reading.capture]` cambian cualquier campo para ese `.exe`. El motor
  de vivo de ese juego se arma al encender los subtítulos en él y su PP-OCR carga con la primera lectura.
* **Valores por defecto:** vivo = v6 small, sin lector; capturas = estándar + PaddleOCR-VL-1.6. Si un archivo con nombre no
  está instalado, se usa el estándar (o 1.5) y Maru lo dice.
* **Migración** (`derive`, `migrate`, `migrate_file`): se conserva lo que el archivo nombraba (`det_model`, `rec_model`/
  `dict_path`, `reader_dir`, `vision_reader = false`); lo que estaba en su valor por defecto toma el nuevo. Copia
  `config.toml.before-reading-<fecha>.bak`, una sola vez (después de `[tasks]`). En tu archivo (`vision_reader = true` y
  `cpu_ocr = false` escritos, ambos su valor por defecto) el ensayo da: vivo v6 small sin lector; capturas estándar + 1.6.
  `cpu_ocr` sigue leyéndose y mantiene el PP-OCR de vivo en el procesador. El modo lite apaga el lector en todo.
* **Un solo lector a la vez:** si vivo y capturas nombran lectores distintos, ambos usan el de capturas (y Maru lo dice).
* **Carga bajo demanda:** con PP-OCR solo, los subtítulos nunca piden el lector. La primera captura que lo necesita espera a
  que cargue y lo dice en palabras llanas («Loading the reading model for captures. This first capture waits for it (a few
  seconds); the next ones are quick.»). Medido: `llama-server` apareció unos 4 s después de la primera captura.
* **Settings:** en Live subtitles «Find subtitle lines with» y «Check live letters with the reading model» (más lector y
  reconocedor en avanzado); en Captures «Use the reading model for sharper letters», «Reading model for captures» (y detector y
  reconocedor en avanzado). El paso «Read the screen» del resumen tiene una fila por tarea: qué lee y dónde corre
  ([captura de la app real](run-4/real-app-read-the-screen.png)). El interruptor «quick reader only» salió de Settings.
* **Instalación** (`maru setup ocr`, `maru setup ocr-vl`): el detector v6 small y PaddleOCR-VL-1.6, de los repositorios de
  PaddlePaddle, comprobados contra su tamaño y SHA-256 publicados; si la copia de la corrida 3 tiene el hash correcto se copia
  (nunca se mueve). `ocr-vl` instala 1.5 y 1.6 (3,6 GB): 1.5 queda de respaldo. **No los corrí sobre tu carpeta**: las pruebas
  usan carpetas temporales, y el ensayo comprobó que tus copias de la corrida 3 están donde la configuración las busca.

### Marcas sueltas y líneas seguras (tarea 2)

* **Qué son las marcas sueltas.** Con un volcado de cada línea (texto, caja, confianza) sobre las 537 imágenes: **todas** las
  marcas que PP-OCR leyó del fondo son líneas aparte, sin hangul (12 con v6 small, 22 con el estándar); **todas** las letras
  latinas o cifras reales («B32 프로토콜에…», «07531 포드 랜딩 완료.») están dentro de una línea coreana. La regla
  (`maru_ocr::stray`): en una lectura con coreano, fuera las líneas sin hangul, salvo una en la misma fila que una línea coreana,
  de su altura y pegada a ella (un número que el detector separó, como podría pasar con «에이도스 7»). Solo para subtítulos: un
  menú tiene líneas de números de verdad.
* **Líneas seguras** (`HybridOcr::keeping_sure_lines`, `[ocr] keep_sure_lines`): si PP-OCR leyó la línea con confianza ≥ umbral
  y el lector solo cambia una sílaba, se queda la de PP-OCR. El umbral se eligió en dev (0,80 / 0,85 / 0,90 / 0,95 / 0,98 →
  11/6, 11/6, 11/3, 5/2, 2/0) y solo después se miró la mitad de juicio.
* `maru eval-ocr --drop-stray-lines` y `--keep-sure` miden las dos cosas.

### Pantallas completas (tarea 3)

* **El set** (fuera del repositorio, `Documents\maru-run-4\scanset`): 115 fotogramas 1080p de tus vídeos (solo lectura):
  86 repartidos a lo largo de cada vídeo, 14 en los momentos de las imágenes «sin subtítulo» de la corrida 3 (una de cada tres
  de Ori, que se repiten) y 15 de pantallas tipo menú (las 7 que marcaste ilegibles y 8 con palabras como 필터 o 장비 en los
  borradores). La selección no miró ninguna salida de OCR. 42 solo tenían la marca de agua del canal: apartadas en
  `scanset\set-aside\no-korean`. **Quedan 73 pantallas** (Elden Ring 18, Ori 13, Sekiro 18, Stellar Blade 16, Xenoblade 8):
  menos de las ~100 del encargo, porque los vídeos son sobre todo escenas sin texto; casi todas son escenas con subtítulo, HUD o
  avisos; menús de verdad hay pocos.
* **Etiquetas provisionales por línea** (`tools/scan_lines.py label`): una línea entra cuando el lector externo (Claude
  Sonnet 5.5, por tu conexión «Claude», con respuesta entre `<<<` y `>>>`) la escribió y uno de los lectores locales de hoy
  (el escaneo o el híbrido 1.5) leyó las mismas letras en la pantalla. Líneas de menos de dos sílabas fuera. 70 pantallas, 189
  líneas. **Son parciales** (no todo el texto de la pantalla) y **favorecen a los de hoy**: ahí un candidato solo puede empatar
  o perder. Los fotogramas de Xenoblade 3, 6 y 8 tienen encima una ventana de Windows (el Administrador de tareas) que salió en
  tu grabación: no tiene coreano y no afecta a las etiquetas.
* **Cómo se puntúa** (`scan_lines.py score`): cada línea etiquetada se busca en lo que leyó el motor, donde esté (el tramo de
  letras más parecido); «encontrada» si su error es ≤ 1/3, «exacta» si es 0. La comparación es línea a línea con prueba de signos.

### El set de pruebas (tarea 4)

* Por defecto en `Documents\maru-evalset\raw` (`Paths::default_evalset_dir`; con `MARU_HOME` se queda dentro). `save_reads`
  escribe ahí; `[continuous] save_reads_dir` (Settings, avanzado) elige otra carpeta.
* `label_review.html`: nombre del archivo en la cabecera; aviso en las que tienen etiqueta provisional; «only pictures with no
  label at all»; «Not a subtitle» (Alt+N, `name.not-subtitle`), aparte de «Unreadable»: ninguna de las dos se puntúa ni recibe
  borradores ni etiqueta provisional; Shift+Enter hace un salto de línea (una pantalla se etiqueta una línea por texto); una
  lectura externa sin hangul no se muestra ni se ofrece como texto inicial ([captura](run-4/label-review-demo.png)).
* Lector externo con formato fijo: la instrucción pide el texto entre `<<<` y `>>>`; sin ese marco, o sin hangul dentro, no hay
  lectura (nunca un borrador). En la corrida 3 comentaba en inglés en once imágenes de Ori.
* `compare_ocr.py`: con `--labels all`, primero el set entero, luego lo que etiquetó una persona (`eval-report --tag person`: toda
  imagen sin la etiqueta provisional, tenga o no `gold`; **antes tus etiquetas sin `gold` no aparecían en ninguna tabla**),
  luego las provisionales. La etiqueta de una persona gana sobre la provisional (ya era así al cargar; ahora hay una prueba).
  Los dos «hoy» (vivo y capturas) son los primeros candidatos; el set por defecto es el de Documents.

### Kiwi (tarea 5)

* **Versión:** la instalada y en uso es 0.23.1 (`bin\kiwi\kiwi-rs\0.23.1`, con `probe.ok`). Bajé 0.23.2, 0.24.0, 0.24.1 y 0.25.0
  a una carpeta temporal: **las cuatro hacen caer la prueba de Maru** (acceso de memoria inválido, 0xC0000005), con kiwi-rs
  2026.8.26 y con 2026.10.17. Así que «la más nueva que pasa la prueba» sigue siendo 0.23.1 y no actualicé ni Kiwi ni la
  biblioteca. Lo que haría falta: una versión de kiwi-rs que siga la interfaz C de Kiwi 0.24/0.25 (el fallo parece de interfaz:
  cambia la estructura de opciones de análisis), o que el autor de kiwi-rs lo publique.
* **Las opciones existen en kiwi-rs**: dialectos al construir (`with_enabled_dialects`) y en cada análisis
  (`with_allowed_dialects`); erratas solo al construir (`build_with_typo`): el transformador por análisis siempre va vacío.
* **En Maru:** `[[profile]] dialects = ["gyeongsang", …]` y `typo_correction`, los dos apagados por defecto. Capturas, líneas en
  vivo, Make cards y el escaneo analizan las líneas de ese programa con esas opciones; sin perfil, exactamente como antes.
* **La corrección de erratas hace caer a Kiwi 0.23.1** en la primera línea que analiza con ella (probé a mantener vivo el
  conjunto de erratas y las dos versiones de kiwi-rs). Por eso, si un perfil la pide, Maru la prueba antes en otro proceso
  (`--maru-probe-kiwi-typo`, como ya hace con cada Kiwi) y, si falla, la deja apagada con un aviso. Comprobado: la prueba normal
  pasa y la de erratas cae en su propio proceso.

### Clics (tarea 6)

`node tools\e2e_app.mjs --app app\src-tauri\target\release\maru-app.exe --out <carpeta vacía> --history <copia de history.db>`:
arranca la app real con una carpeta temporal (Anki apagado, servidor de traducción en una dirección muerta: no carga ningún
modelo ni escribe en Anki), maneja sus propias ventanas por el puerto de herramientas de WebView2 (clics del DOM dentro de las
páginas de Maru: ningún clic ni tecla llega al escritorio ni a otro programa), recorre los 8 apartados de Settings, cambia y
guarda un ajuste en cada uno de los 7 que los tienen, comprueba `config.toml` y abre Make cards en una línea con estrella (o en
vivo si no hay). Hay una prueba de UI que vigila que siga cubriendo todos los apartados.

## Números

### Subtítulos (tarea 2): copia del set etiquetado, 468 subtítulos (148 etiquetas de persona, 389 provisionales)

Corridas nuevas, una tras otra en las mismas condiciones. «Exactas» con la definición de la herramienta (ver tarea 0).

| Motor | Error dev | Error held-out | Exactas held-out | p50 ms held-out | Mejor / peor (held-out) | p | Dev | Todo |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| PP-OCR solo, v6 small (base de a y b) | 1,2 % | 1,2 % | 92,1 % | 42 | — | — | — | — |
| + Qota-OCR (a) | 1,4 % | 0,5 % | 96,1 % | 34 | 15 / 3 | 0,008 | 7 / 6 | 22 / 9 |
| + marcas sueltas fuera (b) | 0,9 % | 0,7 % | 94,3 % | 40 | 5 / 0 | 0,062 | 6 / 0 | 11 / 0 |
| PP-OCR solo, detector estándar | 1,6 % | 1,9 % | 88,6 % | 126 | — | — | — | — |
| + marcas sueltas fuera (b) | 0,8 % | 1,0 % | 93,0 % | 127 | 10 / 0 | 0,002 | 10 / 0 | 20 / 0 |
| Híbrido, lector 1.6 (base de c) | 1,4 % | 2,3 % | 87,3 % | 161 | — | — | — | — |
| + líneas seguras 0,90 (c) | 1,1 % | 1,7 % | 92,5 % | 158 | 16 / 4 | 0,012 | 11 / 3 | 27 / 7 |

### Pantallas completas (tarea 3): 73 pantallas, 189 líneas provisionales

**Provisional**: líneas que hoy ya se leen bien por construcción.

| Motor | Encontradas | Exactas | Error de letras | p50 ms por pantalla | Frente a la base: mejor / peor | p |
|---|---:|---:|---:|---:|---:|---:|
| Escaneo hoy: PP-OCR a 2048 px, detector estándar | 100 % | 98,9 % | 0,1 % | 1.606 | — | — |
| Escaneo, detector v6 small | 100 % | 97,9 % | 0,3 % | 266 | 1 / 3 | 0,625 |
| Escaneo, detector v6 medium | 100 % | 97,9 % | 0,3 % | 681 | 1 / 3 | 0,625 |
| Captura hoy: híbrido con 1.5 | 100 % | 87,3 % | 2,0 % | 762 | — | — |
| Captura: híbrido con 1.6 | 100 % | 89,4 % | 1,6 % | 753 | 15 / 12 | 0,701 |
| Captura: 1.6 + líneas seguras 0,90 | 100 % | 93,1 % | 1,1 % | 751 | 19 / 9 (frente a 1.6: 9 / 2, p = 0,065) | 0,087 |
| Captura: 1.5 + líneas seguras 0,90 | 100 % | 92,1 % | 1,2 % | 772 | 10 / 1 | 0,012 |

Por mitades, 1.6 frente a 1.5: dev 6/5, held-out 9/7. El escaneo de hoy supera al híbrido en estas líneas (24 a 2), pero
compararlos aquí no es justo: las líneas salen justamente de lo que leyeron esos dos.

### Memoria de vídeo con subtítulos en vivo (tarea 1), app real

App compilada en `6a5c5ec`, carpeta temporal enlazada a tus modelos, servidor de traducción muerto (solo se mide OCR). El área
de vivo no mostraba coreano, así que el vigilante no leyó líneas.

| Momento | Total de la tarjeta (nvidia-smi) | `llama-server` | Lo de Maru |
|---|---:|---|---|
| Antes de abrir la app | 2.467 MiB | no | — |
| App abierta | 2.525 MiB | no | sus ventanas WebView (~40–60 MB) |
| Subtítulos en vivo, 30 s, sin capturas | 2.482 MiB | **no** | lo mismo |
| Tras la primera captura (hover) | 4.775 MiB | sí, **2.254 MB** dedicados | PaddleOCR-VL cargado (~4 s) |
| Tras «Quit» | — | no queda ninguno | — |

## Decisiones y su evidencia

| Decisión | Evidencia | Regla |
|---|---|---|
| Vivo = PP-OCR solo + v6 small | tus etiquetas (decisión tuya) | tomada por ti |
| Capturas: lector 1.6 | pantallas: 15 / 12 (p = 0,70), no peor; subtítulos: 38 / 12 | «solo si no es peor más allá del azar» |
| `keep_sure_lines = 0.90` por defecto | held-out 16 / 4 (p = 0,012), tiempo igual; pantallas 9 / 2 frente a 1.6 | mejor en held-out y no 25 % más lento |
| Marcas sueltas: **no** por defecto | v6 small held-out 5 / 0 (p = 0,062) | no llega; con 5 casos la prueba de signos no puede llegar |
| Qota-OCR: apagado | held-out 15 / 3, dev 7 / 6; modelo de la comunidad | el encargo lo pide apagado |
| Detector del escaneo: sin cambiar | v6 small 1 / 3, 6× más rápido; etiquetas provisionales | no cambiar con provisionales: **listo para tus etiquetas** |
| Kiwi: sin actualizar | 0.23.2–0.25.0 caen en la prueba | «la más nueva que pase la prueba» = 0.23.1 |
| Erratas en Kiwi: detrás de una prueba | 0.23.1 cae con ellas | no puede tumbar a Maru |

## Descargas

| Qué | De dónde | Oficial | Licencia | Bytes | sha256 (inicio) | Dónde quedó |
|---|---|---|---|---:|---|---|
| Kiwi 0.23.2 biblioteca / modelo | github.com/bab2min/Kiwi | sí | Apache-2.0 | 36.847.207 / 88.069.580 | `9881e56c…` / `15f35787…` | `Documents\maru-run-4\kiwihome` |
| Kiwi 0.24.0 | igual | sí | Apache-2.0 | 38.138.308 / 87.967.233 | `d876d04c…` / `33188ba9…` | igual |
| Kiwi 0.24.1 | igual | sí | Apache-2.0 | 38.261.195 / 87.967.233 | `857576c5…` / `50d163e1…` | igual |
| Kiwi 0.25.0 | igual | sí | Apache-2.0 | 39.222.663 / 87.967.198 | `f7f38f04…` / `2b32096c…` | igual |
| Kiwi 0.23.1 (otra vez, misma que ya tenías) | igual | sí | Apache-2.0 | — | archivos borrados por `setup` tras instalar | igual |
| crate `sha2` 0.10.9 (y `digest`, `generic-array`) | crates.io | sí | MIT/Apache-2.0 | — | `Cargo.lock` | dependencia de `maru-cli`, Rust puro |
| crate `kiwi-rs` 2026.10.17 | crates.io | sí | — | — | — | solo probado y **revertido** |

Ningún modelo nuevo: el detector v6 small y PaddleOCR-VL-1.6 son los que bajó la corrida 3 (hash comprobado entonces, contra
el que publica el Hub; las instalaciones de esta corrida comprueban el mismo hash). No se ejecutó código de ningún modelo.

## Dinero gastado

| Qué | Pedidos | Tokens entrada / salida | USD |
|---|---:|---|---:|
| Lector externo, prueba de formato (2 pantallas) | 2 | 4.300 / 91 | 0,009 |
| Lector externo, las 115 pantallas | 115 | 247.250 / 3.677 | 0,531 |
| **Total** | 117 | 251.550 / 3.768 | **≈ 0,54** |

Claude Sonnet 5.5 a 2 / 10 USD por millón (el precio que usó la corrida 3). Tokens informados por el servicio. Ningún pedido
falló ni se repitió; ningún otro proveedor.

## Defectos encontrados y corregidos

1. **`compare_ocr.py --labels all` dejaba fuera tus etiquetas sin `gold`**: solo imprimía «tag gold» y las provisionales; las
   de persona sin esa etiqueta no salían en ninguna tabla, y faltaba la vista del set entero.
2. **La premisa del escaneo:** nunca usó el lector (el encargo y el resumen de Settings lo daban por híbrido). Lo dice ahora la
   fila «Screen scan» del resumen.
3. **La página de etiquetado juntaba las líneas en una**, lo que impedía etiquetar pantallas por línea.
4. **El texto inicial podía ser una lectura sin hangul** (un comentario del lector externo o un «0 0»).
5. **Un valor vacío de carpeta habría escrito en la carpeta actual** (`save_reads_dir = ""`): ahora vacío = sin valor.
6. **Visto en la captura de la app real:** el aviso «se carga con la primera captura» salía en color de error; ahora va en el
   texto de la fila.
7. **Errores míos, corregidos en el momento:** quise probar la corrección de erratas soltando el conjunto de erratas tras
   construir (podía ser un uso indebido; no lo era: sigue cayendo con él vivo); una prueba de UI usaba textos latinos que la
   nueva regla ya no ofrece como inicio.

Observado y no corregido: con OCR de Windows (sin PP-OCR) el resumen de Settings sigue diciendo «PP-OCR» en la fila del lector
de texto (venía de antes).

## Qué está verificado y cómo

| Qué | Cómo | Resultado |
|---|---|---|
| «Definition of done» | en esta PC, sobre el árbol final | verde: clippy (workspace y app); 846 pruebas del workspace (9 opcionales omitidas), 45 de la app, 195 de UI, 121 de `tools/`. El CI del último push: en la descripción del pull request |
| Migración en la app real | arranque sobre una carpeta temporal | `[tasks]` y `[reading]` escritos, dos copias `.bak` |
| Memoria de vídeo con vivo | app real, contadores de Windows y nvidia-smi | sin lector hasta la primera captura (tabla de arriba) |
| Settings y Make cards | `tools/e2e_app.mjs`, app real | 8 apartados, 7 ajustes guardados y leídos en `config.toml`, Make cards abierta |
| Resumen «Read the screen» | captura de la app real con modelos | tres filas por tarea, como se diseñó |
| Instalación con hash | pruebas con archivos temporales (reutilizar, rechazar un hash malo, no mover) | pasan; no se corrió sobre tu carpeta |
| Prueba de erratas en otro proceso | `maru --maru-probe-kiwi-typo` con tu Kiwi 0.23.1 | cae en su proceso; la prueba normal pasa |
| Página de etiquetado | Edge sin interfaz, `?demo` | se ve; nadie abrió tu carpeta con ella |

## Sin verificar

* **Nada se probó con un juego en marcha**: ni los subtítulos con v6 small, ni capturas de menús con 1.6, ni el aviso de espera
  visto por una persona (M-55). El área de vivo de la medición de memoria no tenía coreano.
* **Las 389 etiquetas provisionales de subtítulos y las 189 líneas de pantallas**: nadie las ha mirado. Las decisiones de 1.6
  para capturas y de `keep_sure_lines` usan provisionales (con tus 148 etiquetas dentro en la tarea 2).
* **Tu `config.toml` real no se migró**: solo el ensayo (`--dry-run`). Lo hará la app al arrancar.
* **`maru setup ocr` / `ocr-vl` contra la red**: el código de descarga con hash solo se probó con archivos locales.
* **Las pantallas son de vídeos de YouTube** (comprimidos), casi sin menús; las capturas reales suelen ser regiones, no pantallas.
* **Los 10 cambios de Kiwi con Gyeongsang**: sin juzgar (M-59). La corrección de erratas: sin medir (Kiwi cae).
* **Lo que la prueba automática no clica** (M-60): conexiones, botones «Test», Añadir/Reintentar/sonidos de Make cards, el
  reinicio, y las ventanas que no son el lanzador ni el hub.
* **El código exclusivo de Windows** nuevo (prueba de Kiwi en otro proceso, contadores de GPU) solo corrió en esta PC.

## Lo que solo tú puedes comprobar

1. **Instalar lo nuevo** (2 min): con Maru cerrado, `maru setup ocr` y `maru setup ocr-vl`. Deben decir que copian o ya
   tienen el detector y PaddleOCR-VL-1.6 «size and SHA-256 checked» (no debería bajar nada).
2. **Arrancar Maru** (M-55): `config.toml` gana `[reading]` y aparece `config.toml.before-reading-<fecha>.bak` (también
   `before-tasks`, que la corrida 2 dejó pendiente). Settings → Overview → «Read the screen»: tres filas.
3. **Jugar una escena con subtítulos** y mirar en el Administrador de tareas que no hay `llama-server` de ~2 GB; luego una
   captura de región de un menú: aviso de espera la primera vez, rápida la segunda (M-56). Si Xenoblade lee peor, en su perfil:
   `[profile.reading.live]` `reader = true`.
4. **Etiquetar las pantallas** (M-57): `tools\label_review.html` → `Documents\maru-run-4\scanset\raw`, todo el texto coreano,
   una línea por texto (Shift+Enter). Después `python C:\Users\suiny\Documents\maru-run-4\scripts\task3_runs.py` (cambia
   `provisional` por `all` en el script) y `python tools\scan_lines.py score x <las .json de maru-run-4\task3>`. Así se decide el
   detector del escaneo (v6 small, 6× más rápido).
5. **Marcar la página de Kiwi** (M-59): `Documents\maru-run-4\kiwi-review\index.html`, 10 filas, «Save marks». Si con Gyeongsang
   acierta, `dialects = ["gyeongsang"]` en el perfil de Sekiro.
6. **La prueba de punta a punta en tu PC** (M-60), con una copia de tu `history.db`.
7. **Opcional:** encender «Drop marks read beside the subtitle» (Settings → Advanced) si quieres probarlo: no empeoró nada en
   468 subtítulos, solo no alcanzó la prueba.

## Carpetas de modelos que ya no hacen falta

No borré nada. Puedes borrar, si quieres:

| Carpeta (bajo `%APPDATA%\maru\Maru\data\`) | Tamaño | Por qué |
|---|---:|---|
| `models\vlm\glm-ocr`, `models\vlm\nanonets-ocr2-1.5b`, `models\vlm\surya-2.5` | 1,4 + 1,7 + 0,8 GB | leen peor que PaddleOCR-VL (corrida 3) |
| `models\ocr-candidates\pp-ocrv5-mobile-det`, `pp-ocrv5-server-det`, `korean-pp-ocrv5-mobile-rec`, `pp-ocrv6-medium-det` | ~170 MB juntos | no se usan; el «medium» lo nombra Settings como opción (si lo borras, Maru avisa y usa el estándar) |
| `models\ocr-candidates\pp-ocrv6-small-det` | 10 MB | **solo después de `maru setup ocr`**, que lo copia a `models\ocr\pp-ocrv6-small` |
| `models\ocr-candidates\qota-ocr` | 27 MB | si no quieres la opción Qota-OCR (apagada) |
| `models\vlm\paddleocr-vl-1.5` | 1,7 GB | solo es respaldo; pero `maru setup ocr-vl` la vuelve a bajar |
| `bin\kiwi\kiwi-rs\0.23.2`, `bin\kiwi\kiwi-rs\0.24.0` | 529 + 535 MB | no pasan la prueba, nunca se cargan |
| `run-3\`, `evalset\` (el set antiguo; el tuyo está en Documents) | 110 + 109 MB | corridas y set ya copiados |

Y en `Documents\maru-run-4\`: `kiwihome` (2,4 GB de Kiwi bajado para probar), `apphome`, `apphome2`, `e2e`, `e2e-2`,
`evalset` (copia del set), `owner-reports`. **Los enlaces de directorio de `apphome` y `apphome2` ya los quité** (con
`rmdir`), así que borrar esas carpetas no toca tus modelos. Guarda `scanset`, `task2`, `task3`, `kiwi-review` y `scripts`
mientras te sirvan.

## Git

* Rama `feature/run-4`, subida a `origin`. No toqué `main`, no reescribí historia, no fusioné nada.
* Pull request en **borrador** contra `feature/run-3-ocr`.
* Las capturas del repositorio no tienen datos tuyos; las de Make cards y la página de Kiwi (con líneas de tus juegos) se
  quedaron en `Documents\maru-run-4`.
* `.claude/`, `package.json`, `package-lock.json` siguen sin seguimiento (no son de esta corrida). `crates/maru-cli/examples/
  ocr_lines.rs` fue un diagnóstico mío y no está en ningún commit.

## Propuestas para la corrida 5

1. **Decidir el detector del escaneo con tus etiquetas de pantallas** (M-57): v6 small lee igual y es 6× más rápido.
2. **Encender las marcas sueltas** si en tus partidas no empeora nada: la prueba de signos no puede llegar con tan pocos casos.
3. **Kiwi nuevo:** pedir o hacer una versión de kiwi-rs para la interfaz de Kiwi 0.24/0.25; o probar la corrección de erratas
   con la biblioteca de Python de Kiwi en otro proceso para medir si vale la pena.
4. **Un set de menús de verdad**, con `save_reads` (o una captura de pantalla por menú) en tus juegos: los vídeos casi no traen.
5. **Medir los subtítulos en vivo con el vigilante real** (`tests/replay.rs` con tus grabaciones) ahora que el motor cambió.
6. **Ampliar la prueba de punta a punta**: el editor de conexiones con un servidor falso local, y la ventana del panel.

## Dónde paré

Las tareas 0 a 7 están hechas. Parciales, dichos arriba: la actualización de Kiwi (ninguna versión nueva pasa la prueba) y la
corrección de erratas (cae; queda detrás de una prueba); el set de pantallas tiene 73 y no ~100. El estado final del CI y del
pull request está en la descripción del propio pull request.
