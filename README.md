# Corral

Corral junta en la terminal lo que hoy obliga a abrir Positron: correr un trozo de código en la consola de R, ver los gráficos, revisar las variables y mirar una tabla, todo al lado de los agentes. Está pensado para WSL con [herdr](https://herdr.dev), aunque el comando `corral` funciona en cualquier terminal.

Todo ocurre dentro de herdr. Los visores aparecen encima de la terminal cuando los pides y se cierran con `q` o `Esc`, así que nada ocupa espacio mientras trabajas. Si quieres ver algo en grande, **Mirador** muestra lo mismo en el navegador.

Estado: versión 0.1.0. La consola es R normal; la conexión con el kernel Ark viene después.

## Piezas

| Carpeta | Qué es |
|---|---|
| `bin/corral` | El comando, con la lógica. Python 3 sin dependencias. |
| `lib/corral_tui.py` | Los visores de gráficos y variables que corren dentro de herdr. |
| `herdr/` | Plugin de herdr: atajos, layout y popups. Solo llama a `bin/corral`. |
| `r/corral.R` | Lado de R: registra la sesión, `View()` y gráficos con httpgd. |
| `mirador/` | Mirador, la vista en el navegador: un servidor local (`server.py`) y una página (`index.html`). |
| `claude/skills/corral-r/` | Skill para Claude Code: le enseña a trabajar en tu consola de R con `corral eval`. |

Todo lo que tiene estado vive en un panel de herdr (la sesión de R, los agentes). Los visores se pueden cerrar sin perder nada, porque leen de esas sesiones cada vez que se abren.

## Instalación

1. Pon `corral` en el PATH:

   ```bash
   ln -s ~/workspace/personal/corral-term/bin/corral ~/.local/bin/corral
   ```

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

6. Revisa que todo esté en orden:

   ```bash
   corral doctor
   ```

En R necesitas `httpgd` (gráficos), `jsonlite` (variables) y `arrow` o `nanoparquet` (tablas). En el sistema, `chafa` para dibujar gráficos en la terminal y DuckDB para las tablas.

## Uso

Los atajos empiezan con `ctrl+b`: apriétalo, suéltalo y después la otra tecla.

| Quieres | Haces |
|---|---|
| Abrir el espacio de trabajo | `ctrl+b a`. Crea una pestaña con Claude Code a la izquierda, la consola de R arriba a la derecha y el panel de variables abajo. Con `corral layout --shell` agrega una shell junto a las variables. |
| Ver tus objetos y gráficos mientras trabajas | El panel de variables se actualiza después de cada comando, tuyo o de Claude, y sigue a la consola aunque reinicies R. Lista tus objetos y los últimos 12 gráficos con su título. Enter sobre un data frame abre la tabla; sobre un gráfico lo muestra nítido (en herdr si tu terminal dibuja imágenes, si no en Mirador); sobre otro objeto corre `str()`. En un layout que ya tenías: `corral vars --split`. |
| Correr código en R | Selecciona texto con el mouse en cualquier panel (por ejemplo, código que escribió Claude); herdr lo copia solo. Después `ctrl+b enter`. Si no hay selección, se envía lo que tengas en el portapapeles. |
| Ver los gráficos | El primer gráfico de cada sesión abre Mirador solo, como ventana normal a la derecha de la pantalla y al frente. Después se actualiza sin robarte el foco. `ctrl+b f` lo trae al frente en el último gráfico. En una terminal que dibuje imágenes (ver abajo), `ctrl+b f` muestra el gráfico dentro de herdr. |
| Ver las variables en grande | `ctrl+b d` las abre encima de la terminal. `/` busca. |
| Ver una tabla | `View(df)` en R. Se abre encima de la terminal en VisiData; `q` la cierra. |
| Usar otra consola de R | En ese panel, `ctrl+b m`. |
| Abrir Mirador | `ctrl+b shift+m`, o `corral mirador`. |
| Abrir `quarto preview` | `ctrl+click` sobre la URL `http://localhost:...` en herdr. |

Desde la terminal: `corral send 'código'`, `... | corral send`, `corral plots`, `corral vars`, `corral view archivo.parquet`.

### Claude en tu consola de R

Con la skill `corral-r`, Claude Code ejecuta su código en tu consola en vez de abrir un R aparte. Ves cada comando aparecer en `corral:r` con una línea `# corral eval`, y los objetos, gráficos y tablas quedan en tu sesión. Claude recibe la salida y, si hizo un gráfico, una copia en PNG que puede mirar.

Lo mismo sirve desde cualquier terminal o script:

```bash
corral eval 'summary(mtcars)'
corral eval --file analisis.R
corral vars --json
```

Si la consola está ocupada, `corral eval` espera hasta 20 segundos (`--wait-idle`). Si tienes una línea a medio escribir, no ejecuta nada y avisa. Varios agentes pueden usar la misma consola: sus peticiones se ejecutan una a la vez.

### Gráficos en la terminal

En Windows, los gráficos no se pueden ver nítidos dentro de herdr: las terminales nativas (Warp, WezTerm, Windows Terminal) se conectan con WSL a través de ConPTY, que descarta las secuencias de imágenes, y herdr solo deja pasar el protocolo de Kitty. Por eso en Windows los gráficos van a Mirador. Si algún día usas una terminal que sí los dibuje:

- Aprieta `o` en el visor y se abren en el navegador.
- O abre herdr en una terminal que soporte el protocolo de imágenes de Kitty, como WezTerm. herdr solo deja pasar ese protocolo; ni Windows Terminal ni Warp lo dibujan. Corral revisa en qué terminal está abierta cada ventana de herdr y usa imágenes nítidas cuando puede (`corral doctor` muestra qué detectó). Para forzarlo: `corral config images kitty`. En [`wezterm/wezterm.lua`](wezterm/wezterm.lua) hay una configuración de WezTerm que abre WSL directamente; cópiala a `C:\Users\<usuario>\.wezterm.lua`.

## Mirador

El visor de gráficos en el navegador: los gráficos al tamaño de la ventana con historial, ← y →, y descarga en SVG o PNG; además, los objetos de la sesión de R y las tablas con orden, filtro y un resumen por columna. Cuando la ventana es angosta, oculta la barra lateral y deja todo el espacio al gráfico.

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
| `CORRAL_R` | Comando de la consola en `layout` | `R --quiet --no-save` |
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

## Límites conocidos

- En Warp y Windows Terminal los gráficos dentro de herdr son de baja resolución (ver arriba).
- Sin VisiData, las tablas se muestran con DuckDB y `less`: hasta 1000 filas, sin orden ni filtro interactivos.
- Las tablas de `View()` se guardan en el directorio temporal de R y desaparecen al cerrar R.
- Para objetos con más de un millón de elementos no se muestra el tamaño en memoria, porque calcularlo en cada comando sería lento.
- Si el servidor de herdr se detiene (reinicio de Windows, `wsl --shutdown`), la sesión de R se pierde. herdr restaura los paneles, pero no la memoria de R.

## Próximos pasos

- Prototipo con Ark: variables y explorador de datos desde el kernel, sin pasar por archivos.
- Hook `[[startup]]` que relance la consola de R cuando herdr restaure la sesión.
