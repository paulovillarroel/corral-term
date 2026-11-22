# Corral

Corral junta en la terminal lo que hoy obliga a abrir Positron: escribir un script de R o de Python y ejecutarlo por partes, ver los gráficos, revisar las variables y mirar una tabla, todo al lado de los agentes. Está pensado para WSL con [herdr](https://herdr.dev), aunque el comando `corral` funciona en cualquier terminal.

Todo ocurre dentro de herdr. Los visores aparecen encima de la terminal cuando los pides y se cierran con `q` o `Esc`, así que nada ocupa espacio mientras trabajas. Si quieres ver algo en grande, **Mirador** muestra lo mismo en el navegador.

Estado: versión 0.1.0, en uso diario en WSL con Warp. La consola de R es [Ark](https://github.com/posit-dev/ark), el kernel de R de Positron (o R normal, si lo prefieres), y la de Python es IPython en el entorno uv del proyecto.

## Piezas

| Carpeta | Qué es |
|---|---|
| `bin/corral` | El comando, con la lógica. Python 3 sin dependencias. |
| `rs/` | `corral` en Rust, sin dependencias: hace lo que se ejecuta a cada rato (Ctrl+R, F5, `ctrl+b t`, `ctrl+b y`, abrir una consola) y le pasa el resto a `bin/corral`. |
| `bin/corral-run` | Lo que ejecutan los atajos de herdr: el `corral` de Rust si está compilado, si no el de Python. |
| `lib/corral_tui.py` | Los visores de gráficos y variables que corren dentro de herdr. |
| `lib/corral_theme.py` | Toma el tema de herdr y lo aplica al editor, los visores, VisiData y Mirador. |
| `herdr/` | Plugin de herdr: atajos, layout y popups. Solo llama a `bin/corral`. |
| `r/corral.R` | Lado de R: registra la sesión, `View()` y gráficos con httpgd. |
| `py/` | Lado de Python: la sesión de IPython publica sus variables, sirve las figuras de matplotlib con la misma interfaz que httpgd y agrega `View()`. `ark_lsp.py` conecta el editor con el servidor de lenguaje de Ark. |
| `mirador/` | Mirador, la vista en el navegador: un servidor local (`server.py`) y una página (`index.html`). |
| `claude/skills/corral-r/` | Skill para Claude Code: le enseña a trabajar en tus consolas de R y Python con `corral eval`. |
| `fresh/` | Configuración y plugin del editor [Fresh](https://getfresh.dev) para escribir scripts de R y Python. |

Todo lo que tiene estado vive en un panel de herdr (las sesiones de R y Python, los agentes). Los visores se pueden cerrar sin perder nada, porque leen de esas sesiones cada vez que se abren.

## Instalación

1. Compila el `corral` de Rust (necesita `cargo`; no descarga bibliotecas) y ponlo en el PATH:

   ```bash
   cd ~/workspace/personal/corral-term/rs && cargo build --release
   ln -sfn ~/workspace/personal/corral-term/rs/target/release/corral ~/.local/bin/corral
   ```

   Sin Rust, enlaza la versión Python, que hace lo mismo un poco más lento: `ln -sfn ~/workspace/personal/corral-term/bin/corral ~/.local/bin/corral`.

2. Enlaza el plugin en herdr (queda donde está, no se copia). Si cambia `herdr-plugin.toml`, repite con `herdr plugin unlink corral` antes:

   ```bash
   herdr plugin link ~/workspace/personal/corral-term/herdr
   ```

3. Agrega esta línea a `~/.Rprofile`:

   ```r
   if (interactive()) try(source("~/workspace/personal/corral-term/r/corral.R"))
   ```

4. Copia los atajos de [`herdr/keys.example.toml`](herdr/keys.example.toml) a `~/.config/herdr/config.toml` y recarga:

   ```bash
   herdr server reload-config
   ```

5. Enlaza la skill para que Claude Code use tu consola de R:

   ```bash
   ln -s ~/workspace/personal/corral-term/claude/skills/corral-r ~/.claude/skills/corral-r
   ```

6. El editor de scripts: [Fresh](https://getfresh.dev), con `languageserver` para el autocompletado y los diagnósticos, y [Air](https://posit-dev.github.io/air/) para el formato.

   ```bash
   # Fresh: binario oficial de https://github.com/sinelaw/fresh/releases (x86_64-unknown-linux-musl) en ~/.local/bin
   uv tool install air-formatter
   Rscript -e 'install.packages("languageserver")'
   cp ~/workspace/personal/corral-term/fresh/config.json ~/.config/fresh/config.json
   ln -s ~/workspace/personal/corral-term/fresh/plugins/corral.ts ~/.config/fresh/plugins/corral.ts
   ```

7. VisiData, para explorar tablas con orden, filtros y resúmenes:

   ```bash
   uv tool install visidata --with pyarrow
   ```

8. Para Python, [uv](https://docs.astral.sh/uv/). Corral abre la consola con `uv run --with ipython` en el proyecto y el autocompletado con `uv run --with python-lsp-server`; la primera vez uv descarga IPython y python-lsp-server a su caché, sin instalarlos en el proyecto. Los diagnósticos y el formato son de Ruff, con `uvx ruff`.

9. Para la consola de R con Ark (recomendado), el kernel de Ark y el cliente de consola de Jupyter. Sin ellos, Corral usa R normal:

   ```bash
   corral ark --instalar
   uv tool install jupyter-console
   ```

   `corral ark --instalar` descarga Ark 0.1.252 (unos 7 MB) desde las versiones publicadas de `posit-dev/ark` a `~/.local/share/corral/ark/`, y Corral escribe su propia definición de kernel en `~/.local/share/corral/jupyter/`, sin tocar tu instalación de Jupyter.

10. Revisa que todo esté en orden:

   ```bash
   corral doctor
   ```

En R necesitas `httpgd` (gráficos), `jsonlite` (variables), `arrow` o `nanoparquet` (tablas), `languageserver` (autocompletado) y `rlang` (para no dibujar dos veces el mismo objeto gráfico). En el sistema, DuckDB y VisiData para las tablas, y `chafa` solo si usas una terminal que dibuje imágenes.

### Desinstalar

```bash
herdr plugin unlink corral
rm ~/.local/bin/corral ~/.claude/skills/corral-r ~/.config/fresh/plugins/corral.ts
```

Después borra la línea de Corral de `~/.Rprofile` y el bloque `# Corral` de `~/.config/herdr/config.toml`.

### Salir de Corral

`ctrl+b shift+c` cierra el script, la consola y las variables (también el que esté oculto por `ctrl+b t`), y deja la pestaña con Claude, como cualquier pestaña de herdr. Cerrar la consola termina esa sesión de R. El borrador sin guardar queda unos días en la carpeta `.corral/` del proyecto por si lo necesitas. Mirador sigue abierto hasta que cierres su ventana; `corral mirador --stop` detiene su servidor.

## Cuándo actúa Corral

Corral no cambia nada mientras no lo pidas:

- **En R** solo se activa en la consola que abre `ctrl+b a` (que arranca R con `CORRAL=1`), o en un R donde tú lo actives: `ctrl+b m` en ese panel, o `corral_activar()` en la consola. Cualquier otro R funciona como siempre, con su `View()` y su dispositivo gráfico. Para activarlo en todos los R, agrega `options(corral.always = TRUE)` a tu `.Rprofile` antes de la línea de Corral.
- **El layout** (script, consola y variables) solo aparece con `ctrl+b a`.
- **Los atajos `ctrl+b`** solo hacen algo cuando los usas.
- **Claude** usa tu consola solo si el workspace tiene una consola de Corral activa. Si no, trabaja como siempre.

## Uso

Los atajos empiezan con `ctrl+b`: apriétalo, suéltalo y después la otra tecla.

| Quieres | Haces |
|---|---|
| Abrir el espacio de trabajo | `ctrl+b a`. Crea una pestaña con Claude Code a la izquierda (52 % del ancho) y, a la derecha, el script, la consola de R y las variables (el script y la consola se reparten el 73 % del alto, 57/43, y las variables el 27 % restante). El editor abre con el explorador de carpetas a la vista. Con `corral layout --shell` agrega una shell junto a las variables; `--ratio` cambia el ancho de Claude. |
| Expandir el script o la consola | `ctrl+b t` expande el panel donde estás (script o consola) a toda la zona que comparten, sobre las variables. Otra vez `ctrl+b t` y vuelven los dos, con la misma proporción. herdr no achica un panel a menos del 10 %, así que el otro pasa a una pestaña propia, "consola (oculta)" o "script (oculto)", donde sigue corriendo: Ctrl+R y Claude siguen ejecutando en la consola aunque no la veas. También `corral toggle script\|consola\|ambos`. En un layout que ya tenías: `corral script`. |
| Ver tus objetos y gráficos mientras trabajas | El panel de variables se actualiza después de cada comando, tuyo o de Claude, y sigue a la consola aunque reinicies R. Lista tus objetos, incluidos los gráficos guardados en variables (`ggplot`, `lattice`, `patchwork`). Enter sobre un data frame abre la tabla; sobre un objeto gráfico lo dibuja y lo abre en Mirador; sobre otro objeto corre `str()`. El historial de dibujos no aparece aquí: está en Mirador (`ctrl+b f`). En un layout que ya tenías: `corral vars --split`. |
| Correr código en R | Selecciona texto con el mouse en cualquier panel (por ejemplo, código que escribió Claude); herdr lo copia solo. Después `ctrl+b enter`. Si no hay selección, se envía lo que tengas en el portapapeles. |
| Ver los gráficos | El primer gráfico de cada sesión abre Mirador solo, como ventana normal a la derecha de la pantalla y al frente. Después se actualiza sin robarte el foco. `ctrl+b f` lo trae al frente en el último gráfico. En una terminal que dibuje imágenes (ver abajo), `ctrl+b f` muestra el gráfico dentro de herdr. |
| Ver las variables en grande | `ctrl+b d` las abre encima de la terminal. |
| Moverte en el panel de variables | Pestañas Datos, Gráficos, Valores, Funciones y Otros: ←/→, `Tab` o las teclas 1 a 5, o un clic. `/` busca. Un objeto gráfico que ya está dibujado se abre sin volver a dibujarlo. |
| Ver una tabla | `View(df)` en R. Se abre encima de la terminal en VisiData; `q` la cierra. |
| Usar otra consola de R | En ese panel, `ctrl+b m`: la marca como consola y, si hay un R abierto, activa Corral en él. |
| Abrir Mirador | `ctrl+b shift+m`, o `corral mirador`. |
| Abrir `quarto preview` | `ctrl+click` sobre la URL `http://localhost:...` en herdr. |

### Atajos de herdr

| Atajo | Acción |
|---|---|
| `ctrl+b a` | Abrir el espacio de trabajo de Corral |
| `ctrl+b shift+c` | Cerrar Corral: cierra script, consola y variables, y deja a Claude como panel normal |
| `ctrl+b t` | Expandir el script o la consola (el panel donde estás); otra vez, ambos |
| `ctrl+b y` | La consola pasa de R a Python, o de Python a R (Python se abre la primera vez) |
| `ctrl+b enter` | Enviar la selección o el portapapeles a la consola que se ve (R o Python) |
| `ctrl+b f` | Ver el último gráfico en Mirador |
| `ctrl+b d` | Ver las variables encima de la terminal |
| `ctrl+b m` | Usar este panel como consola (R o IPython, según lo que corre) y activar Corral en él |
| `ctrl+b shift+m` | Abrir Mirador |

### Comandos

| Comando | Qué hace |
|---|---|
| `corral layout` | Abre el espacio de trabajo (`--shell` agrega una shell, `--editor ''` sin script, `--agent ''` sin Claude). |
| `corral close` | Cierra los paneles de Corral de la pestaña (script, consola, variables) y deja el resto, Claude incluido. La sesión de R se cierra con su panel. |
| `corral script [archivo]` | Agrega el editor sobre la consola en un layout existente. `--leer` muestra el borrador actual; `--guardar ruta.R` lo guarda (con `--forzar` reemplaza). |
| `corral toggle [script\|consola\|ambos]` | Expande el script o la consola, o vuelve a mostrar ambos; sin argumento, expande el panel donde estás o vuelve a ambos. |
| `corral switch [r\|python]` | La consola del layout pasa a R o a Python; sin argumento, alterna. La otra sigue corriendo en la pestaña "corral (oculto)". |
| `corral console` | Marca el panel actual como consola de R o de Python (según lo que corre en él) y activa Corral si hay un R o un IPython abierto. |
| `corral send 'código'` | Envía código a la consola que se ve, o a la de `--lang r\|python`. También lee stdin, la selección o el portapapeles; `--source` lo ejecuta como archivo (`source()` en R, `%run -i` en Python). |
| `corral eval 'código'` | Ejecuta en la consola y devuelve la salida (pensado para agentes); R por defecto, `--lang python` para Python. `--file`, `--timeout`, `--check`; `--reemplazar` hace que sus gráficos reemplacen a los de la petición anterior. |
| `corral vars` | Variables de la consola que se ve, encima de la terminal; `--split` abre el panel fijo, `--json` las imprime (`--lang` elige R o Python). |
| `corral plots` | Último gráfico (en Mirador en Windows). |
| `corral view archivo` | Abre una tabla (parquet, csv, json) en VisiData. |
| `corral mirador` | Abre Mirador; `--stop` lo detiene, `--url` muestra la dirección. |
| `corral open [url]` | Abre una URL local en el navegador. |
| `corral theme` | Aplica el tema de herdr al editor, los visores, VisiData y Mirador (`--mostrar` solo enseña la paleta). |
| `corral config [clave] [valor]` | Lee o cambia la configuración: `images`, `mirador_window`, `mirador_size`, `python_console`, `r_console` (`ark`, `r` o un comando propio). |
| `corral ark` | Muestra si la consola de R usa Ark; `--instalar [versión]` descarga Ark (por defecto 0.1.252, la versión probada). |
| `corral doctor` | Revisa dependencias, plugin, Mirador y qué terminal detecta. |

### El script

El panel de script es [Fresh](https://getfresh.dev), un editor de terminal sin modos, con los atajos de VS Code (y por lo tanto casi los de Positron), menús y mouse. Para R tiene:

- **Autocompletado** en un menú que aparece solo mientras escribes; ayuda al pasar sobre una función y la firma al abrir el paréntesis; ir a la definición. Con Ark lo da el mismo R de la consola, así que conoce tu sesión: `autos$` ofrece las columnas de tu `autos`. Con R normal lo da el paquete `languageserver`, que solo ve el texto de los archivos.
- **Diagnósticos** como en Positron, solo problemas reales. Con Ark son los de Ark (errores de sintaxis, sobre todo). Con R normal son las reglas *correctness* de lintr (un objeto que no existe, `x == NA`, un argumento repetido), o el `.lintr` del proyecto si tiene uno. En los dos casos el estilo no se marca, porque Air lo arregla al guardar. Fresh arranca el servidor con `corral _lsp`, que elige según la consola.
- **Indentación mientras escribes**, igual que la deja Air: un nivel dentro de un paréntesis abierto, un nivel para las líneas que siguen a `|>`, `%>%`, `+` u otro operador, y de vuelta al margen cuando la instrucción termina.
- **Formato con Air** cada vez que guardas (`Ctrl+S`).

Atajos de Corral. Desde Warp, a través de herdr, solo llegan bien `Ctrl+letra` y las teclas de función: `Ctrl+Enter` llega como `Enter` y los atajos con `Alt` se cortan. Por eso cada acción tiene un atajo que funciona ahí y, entre paréntesis, el de Positron para terminales que sí lo envían.

| Atajo | Qué hace |
|---|---|
| `Ctrl+R` (`Ctrl+Enter`) | Ejecuta la selección o, si no hay, la instrucción completa bajo el cursor (incluidas cadenas de `\|>` o `+` de varias líneas) y avanza a la siguiente. |
| `F5` (`Ctrl+Shift+Enter`) | Ejecuta el archivo completo con `source(echo = TRUE)`. |
| `Ctrl+L` (`Alt+-`) | Inserta ` <- `. |
| `Ctrl+U` (`Ctrl+Shift+M`) | Inserta ` \|> `. |
| `Ctrl+/` | Comenta o descomenta. |
| `Ctrl+Space` | Autocompletado. También aparece solo mientras escribes. |
| `Ctrl+S` | Guarda y formatea con Air. En el borrador temporal, pregunta dónde guardarlo. |

**El borrador es temporal.** Cada `ctrl+b a` abre un `borrador.R` nuevo y vacío dentro del proyecto, en `.corral/<workspace>-<fecha>/`. Así Air, lintr y el servidor de lenguaje usan la configuración del proyecto (`air.toml`, `.lintr`, `DESCRIPTION`, `renv.lock`). Corral crea `.corral/.gitignore` con `*`, de modo que git no ve esos archivos y no hace falta tocar tu `.gitignore`. El explorador de archivos no muestra `.corral/` porque empieza con punto. Los borradores de más de dos días se borran al abrir uno nuevo en ese proyecto. Si abres el layout en tu carpeta personal o en una carpeta sin permiso de escritura, el borrador va a `$XDG_RUNTIME_DIR/corral/borradores/`, que WSL vacía al reiniciar.

Se pierde al cerrar el layout, salvo que lo guardes:

- `Ctrl+S` en el borrador pide un nombre y lo guarda en el proyecto (la carpeta donde abriste el layout). Puedes escribir `analisis/modelo` o una ruta completa; si falta la extensión, agrega `.R`, y si el archivo existe, pregunta antes de reemplazarlo. Desde ahí el editor sigue con el archivo nuevo. En cualquier otro archivo, `Ctrl+S` guarda normal.
- *Archivo → Guardar como* parte en la carpeta del borrador, que no se puede cambiar. Si guardas ahí con otro nombre, Corral mueve el archivo al proyecto con ese nombre (si ya existe uno igual, lo deja y te avisa).
- O pídeselo a Claude: usa `corral script --guardar ruta.R`, que guarda también lo que no has guardado. `corral script --leer` muestra el contenido actual.

El borrador es un archivo real, y no un "sin nombre", porque el autocompletado solo funciona en archivos guardados. Los archivos nuevos sin nombre se abren como R, pero sin autocompletado hasta que los guardes.

Fresh mantiene un servicio de fondo con la configuración y los plugins cargados. Si cambias `config.json` o el plugin, cierra el editor y reinícialo con `fresh --cmd daemon kill orchestrator` antes de volver a abrirlo.

La configuración está en [`fresh/config.json`](fresh/config.json) y el plugin en [`fresh/plugins/corral.ts`](fresh/plugins/corral.ts). Fresh viene con las búsquedas de actualizaciones y la telemetría desactivadas (`check_for_updates: false`).

### Rendimiento: corral en Rust

Los motores ya están en Rust: herdr, Fresh, Ark, Air, Ruff y uv. Lo propio de Corral es sobre todo Python, salvo lo que corre a cada tecla: `corral send` (Ctrl+R, F5, `ctrl+b enter`), `toggle` (`ctrl+b t`), `switch` (`ctrl+b y`) y el arranque de las consolas. Eso está en `rs/`, un binario de Rust de unos 450 KB sin dependencias que habla con herdr por su socket. Cuando le toca algo poco frecuente (abrir la consola de Python por primera vez, leer el portapapeles) o cualquier otro comando, se lo pasa a `bin/corral`.

| `corral send` | Rust | Python |
|---|---|---|
| Tiempo | menos de 10 ms | unos 50 ms |
| Memoria | 2,5 MB | 26 MB |

`toggle` y `switch` bajan de 50–60 ms a 10–30 ms. Si cambias algo en `rs/`, vuelve a compilar con `cargo build --release`; `corral --version` dice si estás usando el binario de Rust.

### La consola de R: Ark

Cuando Ark está instalado, la consola de R es Ark, el kernel de R de Positron, con el cliente `jupyter-console`. Se ve así:

- El prompt es `In [1]:` y los resultados salen como `Out[1]:`, como en IPython. El código de Ctrl+R llega como un bloque pegado y se ve completo en el prompt.
- `?mean` o `help(mean)` abren la ayuda en una ventana del navegador, como el panel de ayuda de Positron.
- `Ctrl+C` interrumpe lo que está corriendo.
- Variables, gráficos en Mirador, `View()`, `corral eval` y el resto de Corral funcionan igual que con R normal. Ark cambia algunas opciones de R para Positron (dispositivo gráfico, navegador, editor); `corral.R` las protege con `ark.protected_options` para que sigan siendo las de Corral.

La ganancia está en el editor y en los recursos. El autocompletado sale del mismo R de la consola, así que no hace falta un segundo R para el servidor de lenguaje. Medido con los mismos paquetes cargados (dplyr y ggplot2) y un gráfico: Ark ocupa unos 330 MB con el servidor de lenguaje incluido, más unos 100 MB del cliente de consola y el puente, frente a unos 250 MB de R normal más unos 450 MB de `languageserver`. En reposo, Ark usa 0,1 % de CPU y `languageserver` 3 %.

Para volver a R normal, `corral config r_console r`; para volver a Ark, `corral config r_console ark`. El cambio rige para las consolas que abras después. `corral ark` muestra qué está instalado.

### Python

R es la consola por defecto. En un proyecto con R y Python no hay que elegir al empezar: la consola cambia según lo que ejecutas.

- Ctrl+R o F5 en un archivo `.py` muestran la consola de Python en el lugar de la de R. La primera vez la abren: IPython con `uv run --with ipython` en la carpeta del proyecto, así que usa el entorno del proyecto (su `.venv` y sus paquetes) sin agregarle IPython. Si la carpeta no es un proyecto de uv, uv usa el `.venv` que encuentre o un Python propio.
- Ctrl+R o F5 en un archivo `.R` vuelven a R. `ctrl+b y` cambia a mano.
- La consola que no se ve sigue corriendo en la pestaña "corral (oculto)", con sus variables. Si entras a esa pestaña puedes usarla igual.
- El panel de variables, Mirador y `View()` siguen a la consola que se ve. El encabezado del panel dice "R 4.6.1" o "Python 3.14.3".

En Python, Ctrl+R envía la instrucción completa bajo el cursor: una función con todo su cuerpo (aunque tenga líneas en blanco), un `if` con su `else`, una llamada repartida en varias líneas o una función con decoradores. El código llega a IPython como un bloque pegado, igual que en Positron, y se ve en `In [n]:`. Una selección se envía sin su sangría común. F5 ejecuta el archivo con `%run -i`, que deja las variables en la sesión. Ctrl+L y Ctrl+U (`<-`, `|>`) no hacen nada en un `.py`.

Las figuras de matplotlib (y de seaborn o plotnine, que dibujan con matplotlib) van a Mirador al terminar cada ejecución, como en Jupyter; `plt.show()` las envía en ese momento. Una figura guardada en una variable aparece en la pestaña Gráficos del panel de variables, y Enter la muestra. Enter sobre un DataFrame de pandas o polars lo abre en VisiData, y `View(df)` en la consola hace lo mismo.

En el editor, el autocompletado y la ayuda vienen de python-lsp-server, que corre en el entorno del proyecto y por eso conoce tus paquetes. Los diagnósticos son de Ruff con sus reglas por defecto (errores reales: nombres sin definir, imports sin usar, errores de sintaxis), y Ruff formatea al guardar, igual que Air en R. Si el proyecto tiene configuración de Ruff en `pyproject.toml` o `ruff.toml`, se usa esa.

Para usar otro comando en la consola de Python (por ejemplo, con más paquetes), `corral config python_console 'CORRAL=1 uv run --with ipython --with polars ipython --no-banner -i ~/workspace/personal/corral-term/py/startup.py'`.

### Claude en tus consolas de R y Python

Con la skill `corral-r`, Claude Code ejecuta su código en tu consola en vez de abrir un R o un Python aparte. Para Python agrega `--lang python`; si la consola de Python no existe, la abre. Ves cada comando aparecer en `corral:r` con una línea `# corral eval`, y los objetos, gráficos y tablas quedan en tu sesión. Claude recibe la salida y, si hizo un gráfico, una copia en PNG que puede mirar.

Lo mismo sirve desde cualquier terminal o script:

```bash
corral eval 'summary(mtcars)'
corral eval --file analisis.R
corral eval --lang python 'df.describe()'
corral vars --json
```

Si la consola está ocupada, `corral eval` espera hasta 20 segundos (`--wait-idle`). Si tienes una línea a medio escribir, no ejecuta nada y avisa. Varios agentes pueden usar la misma consola: sus peticiones se ejecutan una a la vez.

Claude mira cada gráfico que dibuja (Corral lo guarda como PNG) y a veces lo corrige: mueve etiquetas, cambia una escala. Para que no te queden versiones casi iguales en Mirador, la corrección va con `corral eval --reemplazar` y reemplaza al borrador. Si le pides un gráfico distinto, o quieres comparar versiones, se suma al historial como siempre.

### Gráficos en la terminal

En Windows, los gráficos no se pueden ver nítidos dentro de herdr. herdr solo deja pasar imágenes con el protocolo de Kitty, y las terminales nativas de Windows (Warp, WezTerm, Windows Terminal) se conectan con WSL a través de ConPTY, que descarta esas secuencias. Lo probamos: WezTerm en Windows no dibujó ni Kitty ni Sixel. Por eso en Windows los gráficos van a Mirador.

En Linux o macOS, con una terminal que dibuje Kitty (kitty, WezTerm, Ghostty), `ctrl+b f` muestra los gráficos dentro de herdr. Corral detecta en qué terminal está abierta cada ventana de herdr (`corral doctor` lo muestra); para forzarlo, `corral config images kitty`. [`wezterm/wezterm.lua`](wezterm/wezterm.lua) queda como referencia de configuración para WezTerm con WSL.

## Colores

herdr manda: Corral lee el tema de `~/.config/herdr/config.toml` (`[theme] name` y los colores de `[theme.custom]`) y lo aplica a todo lo demás:

- **Fresh** usa el tema `corral` (`~/.config/fresh/themes/corral.json`), con el fondo de la terminal para que el editor se vea igual que los otros paneles.
- **El panel de variables y los visores** usan el acento y los grises del tema.
- **VisiData** toma los mismos colores, aproximados a la paleta de 256 colores de la terminal.
- **Mirador** recibe la paleta como variables CSS.

Los cambios se siguen en vivo: si cambias el tema de herdr (o sus colores en `config.toml`), en unos segundos el editor, el panel de variables y Mirador se recolorean solos, sin reiniciar nada. VisiData toma los colores nuevos la próxima vez que abres una tabla. `corral theme` fuerza la actualización. herdr no publica los valores de sus colores, así que Corral usa los colores publicados de cada tema (vesper, catppuccin, dracula, tokyo-night, nord, gruvbox, one-dark, solarized, kanagawa, rose-pine) más tus ajustes de `[theme.custom]`.

## Mirador

El visor de gráficos en el navegador: los gráficos al tamaño de la ventana con historial, ← y →, y descarga en SVG o PNG; además, las tablas con orden, filtro y un resumen por columna. La barra lateral con los objetos de R viene cerrada; se abre con el ícono de la esquina o con la tecla `v`, y Mirador recuerda tu elección. En ventanas angostas siempre se oculta.

Corral abre cada ventana nueva de Mirador sin maximizar, a la derecha de la pantalla, y la trae al frente. Para cambiarlo:

```bash
corral config mirador_size 0.4x0.7     # fracción del ancho y alto de la pantalla
corral config mirador_window libre     # no tocar el tamaño ni la posición
```

Mirador escucha solo en `127.0.0.1` y pide un token en cada solicitud. El token está en `~/.local/state/corral/mirador.json`.

## Configuración

| Variable | Para qué | Por defecto |
|---|---|---|
| `CORRAL_IMAGES` | Cómo dibujar gráficos en la terminal: `auto`, `kitty` o `symbols`. También con `corral config images …` | `auto` |
| `CORRAL_AGENT` | Comando del panel izquierdo en `layout` (`''` deja una shell) | `claude` |
| `CORRAL_R` | Comando de la consola en `layout` | `CORRAL=1 R --quiet --no-save` |
| `CORRAL_EDITOR` | Editor del panel de script (`''` sin script) | `fresh` |
| `CORRAL_VIEW` | Dónde abre `View()`: `auto` (popup dentro de herdr, Mirador fuera), `mirador` o `popup` | `auto` |
| `CORRAL_BROWSER` | Navegador para Mirador: `firefox`, `edge`, `chrome`, `print` (solo muestra la URL) o un comando con `{url}` | Firefox; si no está, Edge o Chrome |
| `CORRAL_MIRADOR_PORT` | Puerto de Mirador | `7317` |
| `CORRAL_TABLE_VIEWER` | Visor de tablas; `{file}` se reemplaza por la ruta | `vd`, si no DuckDB + `less` |
| `CORRAL_MAX_ROWS` | Filas que muestra el visor de DuckDB | `1000` |
| `CORRAL_CLIPBOARD` | Comando para leer el portapapeles | `wl-paste`, `xclip` o PowerShell |

Opciones de R:

| Opción | Efecto |
|---|---|
| `options(corral.viewer = "mirador")` | El primer gráfico abre Mirador también dentro de herdr. `"httpgd"` abre el visor propio de httpgd. |
| `options(corral.open_plots = FALSE)` | El primer gráfico no avisa ni abre nada. |
| `options(corral.httpgd = FALSE)` | Deja el dispositivo gráfico por defecto de R. |
| `options(corral.vars = FALSE)` | No publica las variables. |
| `options(corral.always = TRUE)` | Activa Corral en todos los R, no solo en la consola del layout. Va antes de la línea de Corral en `.Rprofile`. |

## Problemas comunes

| Síntoma | Qué hacer |
|---|---|
| Fresh muestra un aviso (⚠) sobre el portapapeles de Wayland | WSLg no tiene el protocolo de portapapeles que busca Fresh; Fresh usa X11 y copiar y pegar funciona igual. Corral abre Fresh sin `WAYLAND_DISPLAY` para que no lo intente; si lo abres a mano, `env -u WAYLAND_DISPLAY fresh`. |
| El panel de variables dice "Esperando una sesión de R" o queda vacío | Ese R no tiene Corral activo. En el panel de la consola, `ctrl+b m`, o `corral_activar()` en R. |
| Los atajos del script no hacen nada, o el editor no toma el tema nuevo | Fresh sigue con una versión anterior cargada: `Ctrl+Q`, `fresh --cmd daemon kill orchestrator` y vuelve a abrir el layout con `ctrl+b a`. |
| No aparece el autocompletado | El archivo tiene que estar guardado como `.R` y el estado de Fresh debe decir `LSP (on)`. Si la carpeta no es de confianza, márcala desde el estado (`Confiable`). |
| Mirador no se abre | `corral doctor` muestra si está corriendo y qué navegador usa. `corral mirador --url` da la dirección para abrirla a mano. |
| `ctrl+b enter` dice "Nada que enviar" | Selecciona el código con el mouse (herdr lo copia) o cópialo con `Ctrl+C` antes. |
| Claude abre su propio R en vez de usar tu consola | `corral eval --check` debe responder con la consola. Si falla, activa Corral en ese R con `ctrl+b m`. |

## Límites conocidos

- En Windows los gráficos no se ven nítidos dentro de herdr; van a Mirador (ver arriba).
- Desde Warp, `Ctrl+Enter`, `Ctrl+Shift+…` y `Alt+…` no llegan al editor; por eso los atajos principales son `Ctrl+R`, `F5`, `Ctrl+L` y `Ctrl+U`.
- Sin VisiData, las tablas se muestran con DuckDB y `less`: hasta 1000 filas, sin orden ni filtro interactivos.
- Las tablas de `View()` se guardan en el directorio temporal de R (o de Python) y desaparecen al cerrar la consola.
- La consola de Python necesita IPython; `ctrl+b m` sobre un `python` sin IPython marca el panel, pero no activa Corral.
- Las figuras de Python se guardan al dibujarse (PNG y SVG) y Mirador las escala; no se vuelven a dibujar al cambiar el tamaño de la ventana, como sí hace httpgd en R.
- Para objetos con más de un millón de elementos no se muestra el tamaño en memoria, porque calcularlo en cada comando sería lento.
- Si el servidor de herdr se detiene (reinicio de Windows, `wsl --shutdown`), las sesiones de R y Python se pierden. herdr restaura los paneles, pero no su memoria.
- Ark todavía es una versión 0.1 y Corral habla con su servidor de lenguaje por un canal interno de Positron (`positron.lsp`), que puede cambiar entre versiones; por eso `corral ark --instalar` instala la versión probada. Si una versión nueva falla, `corral config r_console r` vuelve a R normal.
- Con Ark no probé todavía depurar con `browser()` ni funciones que piden datos por teclado (`readline()`, `menu()`).
- Si reinicias la consola de Ark, el autocompletado del editor se reconecta cuando Fresh vuelve a arrancar el servidor de lenguaje (al cerrar y abrir el archivo, o al reiniciar Fresh).

## Próximos pasos

- Variables, explorador de datos y ayuda desde los canales de Ark, sin pasar por archivos.
- Hook `[[startup]]` que relance la consola de R cuando herdr restaure la sesión.
