# Maru — Run 2: un modelo por tarea, Settings por flujo, tarjetas y voces

Informe de la corrida desatendida del 26 de noviembre de 2026 (hora de Lima). Rama: `feature/run-2`, creada desde
`5ffa7e2` (la punta de `stabilize/run-1`). Todo lo que dice «medido» o «verificado» lo observó el agente en esta PC; lo
que no, está en «Sin verificar». La corrida se detuvo dos veces por el límite de uso y se retomó donde estaba; no se
repitió trabajo.

## En corto

* **Lee esto primero: tus subtítulos en vivo siguen apuntando a Gemini.** Tu `config.toml` ya enviaba los subtítulos en
  vivo y las capturas a Gemini (con Azure mientras hay subtítulos y el modelo pequeño de respaldo). La migración conserva
  lo que el archivo hacía, así que **no lo cambié**, aunque lo decidido es «subtítulos en local o NAS» y la clave de
  Gemini sigue sin cuota. Es un clic: pasos en «Lo que solo tú puedes comprobar», punto 1.
* **Un modelo por tarea** (`[tasks]`): subtítulos, capturas, significados de tarjeta, traducción de la frase y las dos
  voces eligen cada uno su conexión. Los archivos anteriores se migran solos al primer arranque, con copia de respaldo.
  Los significados de tarjeta usan el modelo de esta PC, con el diccionario como último recurso.
* **Settings rehecho por flujo**: Resumen, Subtítulos, Capturas, Tarjetas, Voz, Conexiones, Atajos, Avanzado, con
  buscador y un estado con botón «Test» por tarea.
* **Make cards**: inglés y español editables, lista de acepciones, qué modelo respondió, «Retry with…», dos mazos, la
  aplicación como etiqueta `maru::<app>` y aviso de cola con tus números por día. Probado contra Anki real en «User 2».
* **Diccionario**: 3.280 acepciones cuyo «inglés» era solo la romanización ya no pasan por traducción; las palabras con
  más de 8 acepciones se preguntan en dos pasos; la forma corregida por el modelo existe pero queda **apagada**.
* **Anthropic nativo** (`/v1/messages`) funciona contra el servicio real. **No es más rápido** que el endpoint compatible
  y la caché de prompts no entra en juego: los prompts de Maru son más cortos que el mínimo que el servicio cachea.
* **`maru bench` ya no escribe en tu historial.**
* **Voces**: el modelo de 0,6B **no** es más rápido que el de 1,7B y ahorra cerca de 1 GB; la voz por tarea está
  construida, pero Supertonic 3 **no** quedó como voz de lectura por defecto (requiere una descarga y un proceso que Maru
  no arranca: decisión tuya).
* **YouTube**: receptor local y extensión MV3 escritos y con pruebas; la extensión **nunca se cargó en un navegador**.
* **Gasto en APIs: unos USD 0,24** de un tope de 3.
* **Tres páginas esperan tu juicio** (no puntué ninguna): traducciones de frases, formas de diccionario de Kiwi y voces.

## Estado inicial (tarea 0)

| | |
|---|---|
| Rama y HEAD | `feature/run-2` creada desde `5ffa7e2`; árbol limpio salvo `.claude/`, `package.json` y `package-lock.json` sin seguimiento (no los toqué ni los subí) |
| Pruebas al empezar | 690 del workspace (9 opcionales omitidas), 43 de la app, 170 de UI; clippy limpio |
| Herramientas | rustc 1.99.0, node 24.18.0, Python 3.12, gh, ffmpeg |
| Anki | 26.9, AnkiConnect API 6. No estaba abierto: lo abrí yo en el perfil «User 2» (ver «Defectos», punto 9) |
| Tu configuración | subtítulos y capturas en Gemini; Azure mientras hay subtítulos; mazo `Coreano::2 Media`; sin `[tasks]` |

## Qué cambió

| Commit | Qué |
|---|---|
| `972101b` | Tarea 1: tabla `[tasks]`, migración con respaldo, rutas por tarea, respaldos (elegido → esta PC → diccionario), estado y «Test» por tarea |
| `0995efd` | Tarea 2: Settings por flujo, conexiones en un solo lugar, buscador |
| `c7163e7` | Tarea 7: página de revisión a ciegas de traducciones de frases |
| `52053c9` | Tarea 3: Make cards con dos mazos, etiqueta de aplicación, cola, reintento con otra conexión, español en altas del panel |
| `7835f6c` | Tarea 4: romanizaciones, dos pasos para más de 8 acepciones, forma corregida (apagada), página de revisión de Kiwi, herramientas de voces |
| `7e18432` | Tarea 6: proveedor nativo de Anthropic y `maru bench` de solo lectura |
| `131b4a1`, `150c3cf` | Tarea 8: receptor de subtítulos como texto y extensión de YouTube |
| `d2239d9` | Tarea 5: una voz para leer y otra para tarjetas; puerto de subtítulos de YouTube movido |
| último commit | Tarea 9: este informe, `MANUAL_TESTS.md` (M-42 a M-47) y el arreglo de las notas de Estado |

### Un modelo por tarea (tarea 1)

* `[tasks]` en `config.toml`: `live`, `capture` (con `while_live`), `card_meanings`, `card_sentence`, `voice_reading`,
  `voice_cards`. Cada una es `local` o el nombre de una conexión guardada.
* **Migración**: al primer arranque la app escribe la tabla a partir de lo que el archivo ya hacía y deja
  `config.toml.before-tasks-<fecha>-<hora>.bak` al lado. Ninguna clave antigua se renombra; los comentarios se conservan.
  El único cambio de comportamiento: los significados de tarjeta preguntan al modelo de esta PC.
  `maru config migrate --dry-run` muestra la tabla sin escribir; `maru config tasks` muestra la vigente.
* **Respaldos para tarjetas**: conexión elegida → modelo de esta PC → primera acepción del diccionario, y la hoja dice
  cuál respondió y por qué. Con subtítulos en vivo en marcha **no se carga** el modelo de esta PC para una tarjeta; la
  hoja ofrece preguntar cuando paren.
* **Errores en lenguaje llano**: «Gemini has no quota left.», «NAS cannot be reached.», nunca el cuerpo crudo del servidor.

### Settings por flujo (tarea 2)

Capturas (Chromium sin interfaz, datos de demostración): [resumen](run-2/settings-1-overview.png),
[subtítulos](run-2/settings-2-live.png), [capturas](run-2/settings-3-captures.png), [tarjetas](run-2/settings-4-cards.png),
[voz](run-2/settings-5-voice.png), [conexiones](run-2/settings-6-connections.png),
[editor de conexión](run-2/settings-6b-connection-editor.png), [atajos](run-2/settings-7-shortcuts.png),
[avanzado](run-2/settings-8-advanced.png), [búsqueda](run-2/settings-9-search.png).

Cada ajuste dice qué funciones lo usan; hay tres tipos de control (elección, interruptor, texto) y ninguna lista
desplegable para elegir; los nombres internos desaparecieron de la página. Los enlaces profundos
`hub.html?settings=<sección>&q=<búsqueda>#settings` abren una sección.

### Make cards (tarea 3)

Captura: [hoja](run-2/make-cards.png) (datos de demostración: la frase del encabezado y las palabras no son de la misma
línea).

* Inglés y español editables por palabra; lista de acepciones del diccionario; la hoja dice qué modelo eligió.
* «Retry with …» por conexión; conserva lo que escribiste a mano, tus marcas y los mazos.
* Dos mazos: `learning.anki.deck` (palabras) y `learning.anki.sentence_deck` (frases). **No puse en tu archivo**
  `Coreano::2 Media::1 Palabras` y `Coreano::2 Media::2 Oraciones`: pasos en el punto 2 de abajo.
* La aplicación es la etiqueta `maru::<app>`, ya no un submazo (el submazo por aplicación vuelve con
  «Suggest a deck per program»).
* Aviso de cola: cuántas tarjetas nuevas esperan en cada mazo y en cuántos días saldrían las que añades, con tus números
  (3 palabras y 1 frase por día por defecto). Los límites propios de Anki no se leen: AnkiConnect no ve los ajustes por mazo.
* Las altas desde el panel con varias acepciones ya llevan español.

### Diccionario y Kiwi (tarea 4)

* **Romanizaciones** (`maru dict romanized`, sobre una copia): de 67.653 acepciones en 48.353 entradas, 3.280 (en 2.448
  entradas) tenían como «inglés» solo la palabra coreana en letras latinas. Ahora: 521 toman una glosa de la definición,
  406 se conservan como préstamos reales, 44 conservan sus otras palabras y 2.309 quedan sin inglés para que lo escribas
  o lo ponga el modelo.
* **Más de 8 acepciones**: 257 de 44.436 grupos de palabras (치다 tiene 49). En tus 50 líneas, el modelo local se
  equivocaba en el 27,6 % de esas palabras frente al 12,3 % en las de 2 a 8, y en 6 de sus 8 errores la acepción correcta
  no estaba entre las ocho mostradas. Ahora se hace un segundo pedido corto (55 pedidos para 50 líneas; 6 palabras
  cambiaron de acepción). **No está juzgado** si las nuevas respuestas son mejores.
* **Forma corregida**: el modelo puede proponer la forma de diccionario cuando Kiwi falla (dialecto, OCR roto). Los
  modelos locales y el NAS no propusieron ninguna; Claude Haiku 7; Claude Sonnet 25 (11 en dialecto, 14 en OCR roto,
  ninguna en texto estándar), con aciertos (꾸다→주다, 잡그래이→잡다) y varias que no son formas de diccionario
  (나한테, 조금만, 것인데). Además, añadir la instrucción cambió respuestas del modelo local sin ninguna corrección a
  cambio. Por eso queda **apagada** (`learning.anki.correct_forms = false`) y con su propio prompt, para que el de la
  corrida 1 no cambie. La propuesta nunca se aplica sola.

### Proveedores y herramientas (tarea 6)

* Conexión de tipo **Anthropic (Claude)**: `POST /v1/messages`, prompt de sistema marcado para caché, eventos nativos,
  mismo apagado del razonamiento (`disabled` y, si el modelo lo rechaza, `between_tools`). La conexión que ya tenías
  (dirección de Anthropic guardada como servidor compatible) **sigue usando el endpoint compatible**, sin cambios.
  `--connection "Claude+native=modelo"` la pregunta por la ruta nueva en una sola corrida.
* `maru bench` arranca el motor en solo lectura: no abre ni escribe `history.db`, las áreas de lectura ni las etiquetas.
* `vendor/ort-sys` quedó como estaba.

### Voces (tarea 5)

* `[tasks.voice_reading]` y `[tasks.voice_cards]` aceptan, además del locutor, otro servidor de voz (`endpoint`,
  `model`). Sin nada escrito, una sola voz para todo, como antes.
* Si la lectura va a otro servidor y las tarjetas usan la voz de esta PC, esa voz se carga con el primer sonido de
  tarjeta y **se descarga al terminar** Make cards (también tras el alta del panel y «rellenar sonidos»). Si ambas
  tareas comparten voz, no se descarga nada.
* Settings → Voice tiene los campos y un botón de prueba por voz.
* **No hice**: dejar Supertonic 3 como voz de lectura por defecto. Maru no arranca ese servidor (es un paquete de
  Python) y el paquete no está instalado; instalarlo es una descarga nueva. No cambié el tamaño de la voz de tarjetas.

### YouTube (tarea 8)

`[captions]` (apagado por defecto): un receptor en `127.0.0.1:18098` que solo acepta a la extensión y con un código;
las líneas entran al historial como `live · youtube` y pueden marcarse con estrella y minarse. La extensión MV3 está en
`extension/youtube-captions`. Diseño, instalación y límites: [YOUTUBE_CAPTIONS.md](../YOUTUBE_CAPTIONS.md). La traducción
no se dibuja sobre el video.

### Tu PC y tus datos

* **`config.toml`: no lo modifiqué.** Solo lo leyó `maru config migrate --dry-run`. La app lo migrará en su primer
  arranque, con respaldo.
* Tus bases de datos y `..\koreanDiary` solo se leyeron o se copiaron; las mediciones corrieron sobre copias en una
  carpeta temporal.
* Carpeta nueva `%APPDATA%\maru\Maru\data\run-2\`: las tres páginas de revisión y, en `voices\models\`, el modelo
  `qwen-talker-0.6b-customvoice-Q8_0.gguf` (968.588.544 bytes, Apache-2.0), que puedes borrar si no lo quieres.
* **Anki quedó abierto en «User 2»** (lo abrí yo). Nunca se llamó a `loadProfile` ni a `sync`; solo se escribió bajo
  `MaruTest::…` y no quedó nada. Tu perfil «User 2» tenía tres tarjetas tuyas de Maru y los mazos `Coreano`,
  `Coreano::2 Media`, `Coreano::2 Media::Eden` y `Default`: solo se contaron.

## Qué está verificado y cómo

| Qué | Cómo | Resultado |
|---|---|---|
| «Definition of done» | en esta PC, sobre el árbol final | verde: clippy (workspace y app); 767 pruebas del workspace (9 opcionales omitidas), 45 de la app, 178 de UI y 23 de la extensión, 86 de `tools/` |
| CI | GitHub Actions en la rama | verde en cada push hasta `150c3cf`; el estado de los dos últimos está en el pull request |
| Dos mazos, etiqueta y cola contra Anki real (M-43) | `maru-srs` y `maru-engine` `--test real_anki`, perfil «User 2», mazos `MaruTest::…` | pasó (4,6 s y 16,4 s): palabras en `…::1 Palabras`, frase en `…::2 Oraciones`, etiqueta `maru::eldenring`, sin submazo del juego; la cola leyó 2 y 1, y 1 tras suspender una |
| Migración sobre tu archivo real | `maru config migrate --dry-run` | imprimió la tabla esperada; no escribió |
| Anthropic nativo contra el servicio real (M-44) | `maru connection test`, `maru bench --live` | respondió 40 de 40 con Haiku y con Sonnet |
| Receptor de subtítulos (M-45) | ejemplo `captions_listen` y `curl` en un puerto libre | línea aceptada, repetida descartada, código malo 401, origen de página web 403 |
| Páginas de Settings, hoja Make cards, pestaña Estado | Chromium sin interfaz con datos de demostración | cargan; capturas arriba. **Nadie hizo clic en nada** |
| `maru bench` no deja rastro | prueba del motor: tras una línea en vivo en solo lectura no existe ningún `.db` | pasó |

## Números

**Traducción de frases (tarea 7), 50 líneas, mediana de la línea completa en ms:** local 233, NAS 324, Claude Haiku 631,
Claude Sonnet 812, Azure 1.039. Los cinco respondieron 50 de 50. Gemini quedó fuera: respondió HTTP 429 al primer pedido
y no se le volvió a preguntar. La calidad **no está puntuada**: es la página de revisión.

**Anthropic: endpoint compatible frente a nativo (tarea 6), 20 líneas × 2 pasadas, un pedido a la vez, ms:**

| Modelo | Ruta | Primera palabra p50 | p95 | Línea p50 | p95 | Fallos | Entrada leída de caché |
|---|---|---:|---:|---:|---:|---:|---:|
| Claude Haiku 5.5 | compatible | 544 | 665 | 644 | 846 | 0 de 40 | 0 |
| Claude Haiku 5.5 | nativa | 506 | 649 | 615 | 828 | 0 de 40 | 0 |
| Claude Sonnet 5.5 | compatible | 762 | 1.779 | 1.296 | 1.893 | 0 de 40 | 0 |
| Claude Sonnet 5.5 | nativa | 790 | 1.577 | 1.249 | 1.723 | 0 de 40 | 0 |

Las diferencias están dentro del ruido de 40 pedidos. La caché no leyó nada porque cada pedido lleva unos 430 tokens y
el mínimo cacheable de estos modelos es 512: la propuesta de la corrida 1 («con el nativo Sonnet podría bajar su
latencia») **no se cumplió**. El proveedor nativo queda como opción, no como valor por defecto.

**Voces de tarjeta (tarea 5), 50 líneas, `tts-server` instalado (Vulkan), RTX 5070 Ti:**

| Modelo | Lista en | Memoria de GPU | Tiempo para generar | Audio generado | Retomas | Observación |
|---|---:|---:|---:|---:|---:|---|
| Qwen3-TTS 1,7B CustomVoice Q8 (la actual) | 1,7 s | 3,8–3,9 GB | 11,0 s | 68,2 s | 3 de 50 | |
| Qwen3-TTS 0,6B CustomVoice Q8 | 1,1–1,7 s | 2,9–3,2 GB | 16,7 s | 99,4 s | 10 de 50 (2 quedaron largas) | clips 1,5 veces más largos, con más silencio (54 % frente a 26 %) |

El modelo pequeño no es más rápido y falla más: no hay motivo medido para cambiar. Supertonic 3 no se volvió a medir
(paquete no instalado); de la corrida 1: 0,9 s en estar listo, 519 MB, sin GPU. Los «10 s» de arranque de la corrida 1
eran sobre todo las comprobaciones de puerto de Maru, no el modelo. **Nadie escuchó nada.**

**Kiwi en tus 50 líneas** (palabras de contenido por grupo): estándar 65, dialecto 52, OCR roto 104. La página de
revisión tiene 239 filas. Una estimación del agente, que **no es tu juicio**: lemas mal en 1 de 67 (estándar), 13 de 53
(dialecto) y 30 de 108 (OCR roto).

## Dónde están las tres páginas de revisión

Todas en `%APPDATA%\maru\Maru\data\run-2\` (fuera del repositorio: contienen tus líneas).

1. `sentence-translation-review.html`: 193 traducciones distintas de 50 líneas, sin decir de qué modelo es cada una.
   Marca, «Save marks», y luego
   `python tools/build_sentence_review.py score <marks.json> "%APPDATA%\maru\Maru\data\run-2\sentence-review\key.json"`.
   Una línea de Haiku y una de Sonnet van marcadas como cortadas.
2. `kiwi-review\index.html`: ¿es correcta la forma de diccionario de cada palabra en su línea?
3. `voices\index.html`: las mismas frases con la voz de 1,7B, la de 0,6B y Supertonic 3.

## Dinero gastado

| Qué | Tokens entrada / salida | USD |
|---|---|---:|
| Tarea 7: traducciones de frases (Azure, Claude Haiku, Claude Sonnet) | no los anoté por separado | 0,043 |
| Tarea 4: formas corregidas, Claude Haiku | | 0,005 |
| Tarea 4: formas corregidas, Claude Sonnet | | 0,100 |
| Tarea 6: Claude Haiku, compatible y nativa | 34.359 / 1.343 | 0,004 |
| Tarea 6: Claude Sonnet, compatible y nativa | 34.427 / 1.356 | 0,082 |
| Pruebas de conexión sueltas | unos 1.300 / 50 | 0,001 |
| Gemini | un pedido, rechazado (429) | 0 |
| **Total** | | **≈ 0,24** |

Precios por millón de tokens: Claude Sonnet 5.5 2/10, Claude Haiku 5.5 0,10/0,50; Azure `gpt-4.1-mini` 0,40/1,60
(**supuesto**, como en la corrida 1). Los tokens son los que informó cada servidor. Ningún pedido se reintentó en bucle.

## Defectos encontrados y corregidos

1. **`maru bench --live` escribía en tu historial**: sin `--connection` pasaba por el motor, que guarda cada línea en
   vivo. Ahora el motor arranca en solo lectura para `bench`.
2. **3.280 acepciones con la romanización como «inglés»** habrían llegado a las tarjetas como si fueran traducción.
3. **Palabras con más de 8 acepciones**: la acepción correcta a menudo no estaba entre las ocho que se mostraban al modelo.
4. **Altas desde el panel sin español** cuando la palabra tenía varias acepciones.
5. **El panel mostraba el cuerpo crudo del error del servidor** (por ejemplo un JSON de cuota). Ahora una frase llana.
6. **El puerto por defecto de los subtítulos de YouTube era 18096**, el mismo que usa el modelo local de subtítulos
   cuando las capturas van a un servidor remoto (y el de la herramienta de Supertonic). Ahora 18098.
7. **Las notas de la pestaña Estado mostraban `[object HTMLParagraphElement]`** en vez del texto; estaba así desde el
   primer commit de la app.
8. **La prueba del motor contra Anki real suponía un perfil vacío** y falló al encontrar tus tres tarjetas en «User 2».
   Ahora cuenta primero lo que hay.
9. **Un error mío**: para las capturas de pantalla levanté un servidor web local en el puerto 8765, que es el de
   AnkiConnect. Lo noté cuando la comprobación de perfil devolvió HTML; paré mi servidor (tras comprobar que el proceso
   era el mío). Anki no estaba abierto, así que no interferí con él, y lo abrí después en «User 2». No se escribió nada en
   Anki mientras duró el error.

10. **Un nombre tuyo en el repositorio público, desde la corrida 1**: una prueba de la línea de comandos
    (`crates/maru-cli/src/connections.rs`, commit `e70c50d`) usaba como ejemplo el nombre de tu conexión de Azure tal
    como la tienes guardada. Lo cambié por un nombre neutro en el árbol actual, pero **sigue en el historial ya
    publicado** de `stabilize/run-1` y de esta rama: quitarlo exige reescribir historia, que no me corresponde. No es
    una clave ni una dirección (la dirección de la prueba es `example.openai.azure.com`), solo el nombre. Decide tú si
    importa (punto 12 de abajo).

## Sin verificar

* **La app no se arrancó** con tu configuración real: ni la migración real (solo el ensayo), ni Settings, ni Make cards.
* **Nadie hizo clic** en las páginas nuevas: se vieron en Chromium sin interfaz con datos de demostración.
* **Ninguna tarjeta fue vista ni escuchada** por una persona; tu perfil real de Anki y AnkiWeb no se tocaron.
* **Si los significados y las traducciones son correctos**: ninguna de las tres páginas está marcada. En particular, no
  sé si el segundo pedido para palabras largas ni la regla de romanizaciones mejoran las tarjetas.
* **Voces**: nadie escuchó. Que la voz de tarjetas se descargue tras Make cards solo está probado como decisión, no con
  el `tts-server` real. Maru nunca habló por Supertonic desde la app.
* **YouTube**: la extensión nunca se cargó en un navegador ni vio YouTube. Sus datos de prueba se escribieron de memoria
  del formato, no capturados del sitio. Es probable que YouTube rechace pedidos de subtítulos que no hizo su reproductor.
* **El tipo de conexión «Anthropic (Claude)» en el editor** de Settings: la ruta nativa se probó por línea de comandos.
* **Gemini**: sigue sin cuota.
* **El código exclusivo de Windows** solo se compiló y pasó sus pruebas aquí; el comportamiento con ventanas y foco no.
* **El último push**: su CI estaba en curso al escribir esto.

## Lo que solo tú puedes comprobar

1. **Sacar los subtítulos de Gemini** (2 min). Arranca Maru (hará la migración y dejará el `.bak`). Settings → Live
   subtitles → «Translate live lines with» → *This PC* o *NAS*. Settings → Captures → «Translate captures with» → lo que
   prefieras. Save y reinicia. Si algo no te gusta: copia `config.toml.before-tasks-….bak` sobre `config.toml`.
2. **Tus dos mazos**: Settings → Making cards → «Deck for word cards» = `Coreano::2 Media::1 Palabras` y «Deck for
   sentence cards» = `Coreano::2 Media::2 Oraciones`. Save.
3. **Una tarjeta de verdad** (M-43): con Anki en tu perfil normal, Historial → una línea con estrella → Make cards.
   Mira el aviso de cola, cambia una acepción, pulsa «Retry with …», añade dos palabras y la frase. En Anki: cada una en
   su mazo, etiqueta `maru::<juego>`, sin submazo del juego.
4. **Marcar las traducciones de frases** (página 1 de arriba) y correr el comando `score`.
5. **Marcar la página de Kiwi** y decidir si vale la pena encender `correct_forms` con Claude Sonnet (M-47).
6. **Escuchar las voces** (página 3) y decidir: ¿0,6B o 1,7B para tarjetas?, ¿Supertonic 3 para leer?
7. **Si quieres Supertonic para leer** (M-46): `pip install supertonic`, `python tools/supertonic_server.py --port 18097`,
   Settings → Voice → «While reading, use another voice server» = `http://127.0.0.1:18097`, «Speaker while reading» =
   `F1`, Advanced → «Model name sent to the reading server» = `supertonic-3`. Save, reinicia, prueba los dos botones y mira
   en el Administrador de tareas que la voz de Qwen sale de la GPU después de hacer tarjetas.
8. **Settings a mano** (M-42): recorre las secciones, usa «Test» en cada tarea con un servidor apagado, busca un ajuste.
9. **YouTube** (M-45): carga `extension/youtube-captions` en Chrome (chrome://extensions → modo desarrollador → «Cargar
   descomprimida»), enciende «Take YouTube captions from the browser» en Settings → Live subtitles, copia el código a la
   extensión y prueba un video con subtítulos coreanos del autor. Dime qué mensaje da la extensión.
10. **Anki**: quedó abierto en «User 2»; vuelve a tu perfil cuando quieras.
11. **El precio real de tu despliegue de Azure**, que sigue supuesto en la tabla de costos.
12. **El nombre de tu conexión de Azure en el historial público** (defecto 10): si te importa, hay que reescribir las
    dos ramas antes de fusionarlas (`git filter-repo --replace-text`, y `push --force` de ambas) o fusionar con
    «squash», que deja fuera los commits intermedios en `main` pero no borra las ramas ya subidas. Si no te importa, no
    hay nada que hacer.

## Git

* Rama `feature/run-2`, subida a `origin`. No toqué `main`, no reescribí historia, no fusioné nada.
* El pull request es **borrador** y apunta a `stabilize/run-1` (el de la corrida 1, #1, sigue abierto): así su diff es
  solo la corrida 2. Si fusionas primero el #1, GitHub lo reapunta a `main`.
* El agente de YouTube trabajó en una copia de trabajo aparte (`.claude/worktrees/…`, rama local
  `worktree-agent-…`); sus dos commits se trajeron con `cherry-pick` después de revisarlos. Esa copia y esa rama local
  siguen ahí, sin subir; puedes borrarlas.
* `.claude/`, `package.json` y `package-lock.json` están sin seguimiento en tu carpeta: no son de esta corrida.

## Propuestas para la corrida 3

1. **Decidir con tus marcas**: modelo para la frase de la tarjeta, si el segundo pedido se queda, si `correct_forms` se
   enciende con Sonnet (y entonces validar la forma propuesta contra el diccionario antes de mostrarla: descartaría
   «나한테» o «조금만»).
2. **Que Maru arranque Supertonic** (o empaquetar su ONNX en Rust con el ONNX Runtime que ya usa el OCR) para que la
   lectura en CPU sea el valor por defecto sin pasos manuales.
3. **Caché de prompts de verdad**: solo sirve si el prefijo fijo supera los 512 tokens. Para significados de tarjeta se
   podría mover el contexto del juego y ejemplos al prompt de sistema; medir antes si compensa.
4. **Leer los límites reales de Anki** para el aviso de cola (por ahora usa tus dos números).
5. **YouTube**: tras tu prueba, decidir si se dibuja la traducción sobre el video y si hace falta la ruta de respaldo.
6. **Un `maru config set`** que use el mismo parche que Settings (conserva comentarios, deja respaldo), para no editar
   el archivo a mano.
7. **Probar la app de punta a punta de forma automática** (WebDriver de Tauri) para que «nadie hizo clic» deje de ser
   la frase más repetida de estos informes.

## Dónde paré

Las tareas 0 a 9 están hechas, con tres parciales que el encargo admitía y que quedan dichos arriba: Supertonic no es la
voz de lectura por defecto (tarea 5), la extensión de YouTube no se probó en un navegador (tarea 8) y tu `config.toml`
no lleva todavía los dos mazos ni el cambio de los subtítulos (decisiones conservadoras: pasos 1 y 2). El estado final
del CI y del pull request está en la descripción del propio pull request.
