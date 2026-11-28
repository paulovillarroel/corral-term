# corral.R: R side of Corral.
#
# install.sh adds this line to ~/.Rprofile (the path is where Corral lives):
#   if (interactive()) try(source("~/.local/share/corral-term/r/corral.R"))  # Corral
# Corral's own consoles load it through r/profile.R instead, so it also loads in a
# project whose .Rprofile R reads in place of yours (renv's).
#
# Corral only acts in R sessions that ask for it: the console that `corral layout`
# (ctrl+b a) starts, which sets CORRAL=1, or any session where you run
# corral_activate() (ctrl+b m does it for you). Every other R behaves as usual.
# options(corral.always = TRUE) in .Rprofile turns it on everywhere.
#
# Messages follow Corral's language: CORRAL_LANG (en or es), else `language` in
# ~/.config/corral/config.json (what `corral config language` writes), else English.
#
# What it does in an active session (never in RStudio or Positron):
#   * Registers this session for Mirador and, after every top-level command,
#     writes a light description of the global environment (needs jsonlite).
#   * View(x) writes x to a temporary parquet file and opens it with `corral view`
#     (View(x, mirador = TRUE): in Mirador, in the browser)
#     (a popup inside herdr; a tab in Mirador outside herdr).
#   * Plots go to httpgd. Inside herdr, ctrl+b f (or corral_plots()) shows them in
#     a popup; outside herdr the first plot opens Mirador. Plot history lives in
#     this R process, so closing a viewer loses nothing.

local({
  if (!interactive()) return(invisible())
  if (nzchar(Sys.getenv("RSTUDIO")) || nzchar(Sys.getenv("POSITRON"))) return(invisible())
  # Loaded: profile.R doesn't load it a second time.
  options(corral.loaded = TRUE)

  # This file, found through the `corral` command: up from it to Corral's folder
  # (the command is rs/target/release/corral there).
  this_file <- local({
    bin <- Sys.which("corral")
    if (!nzchar(bin)) return(NA_character_)
    d <- dirname(normalizePath(bin))
    while (!file.exists(file.path(d, "r", "corral.R")) && dirname(d) != d) d <- dirname(d)
    file.path(d, "r", "corral.R")
  })

  # Corral's language, looked up again every couple of seconds so a switch made
  # with `corral config language` shows up without restarting R.
  lang_cache <- list(at = -Inf, value = "en")
  corral_lang <- function() {
    now <- as.numeric(Sys.time())
    if (now - lang_cache$at < 2) return(lang_cache$value)
    normalize <- function(x) {
      x <- tolower(trimws(x))
      if (length(x) != 1L || is.na(x)) NA_character_
      else if (x %in% c("en", "english", "ingl\u00e9s", "ingles")) "en"
      else if (x %in% c("es", "spanish", "espa\u00f1ol", "espanol")) "es"
      else NA_character_
    }
    value <- normalize(Sys.getenv("CORRAL_LANG"))
    if (is.na(value)) {
      config <- Sys.getenv("XDG_CONFIG_HOME")
      if (!nzchar(config)) config <- file.path(path.expand("~"), ".config")
      f <- file.path(config, "corral", "config.json")
      text <- if (file.exists(f)) {
        tryCatch(paste(readLines(f, warn = FALSE, encoding = "UTF-8"), collapse = "\n"), error = function(e) "")
      } else ""
      m <- regmatches(text, regexec('"language"[[:space:]]*:[[:space:]]*"([^"]*)"', text))[[1]]
      value <- if (length(m) == 2L) normalize(m[2]) else NA_character_
    }
    if (is.na(value)) value <- "en"
    lang_cache <<- list(at = now, value = value)
    value
  }
  # The text in the user's language.
  tr <- function(en, es) if (identical(corral_lang(), "es")) es else en
  trf <- function(en, es, ...) sprintf(tr(en, es), ...)

  # Attach right above utils so our functions mask utils ones (View). During
  # .Rprofile only base is attached, so wait for utils when needed.
  attach_env <- function(e) {
    attach_now <- function(...) {
      if ("corral" %in% search()) detach("corral", character.only = TRUE)
      attach(e, name = "corral", warn.conflicts = FALSE)
    }
    if ("package:utils" %in% search()) attach_now()
    else setHook(packageEvent("utils", "attach"), attach_now)
  }

  enabled <- nzchar(Sys.getenv("CORRAL")) || isTRUE(getOption("corral.always"))
  if (!enabled) {
    stub <- new.env()
    stub$corral_activate <- function() {
      if (is.na(this_file) || !file.exists(this_file)) {
        stop(tr("corral.R not found (is `corral` on the PATH?)",
                "no encuentro corral.R (\u00bf`corral` est\u00e1 en el PATH?)"), call. = FALSE)
      }
      Sys.setenv(CORRAL = "1")
      source(this_file)
      message(tr("Corral is now active in this R session.", "Corral activado en esta sesi\u00f3n de R."))
      invisible(TRUE)
    }
    attach_env(stub)
    return(invisible())
  }

  env <- new.env()
  env$corral_activate <- function() {
    message(tr("Corral is already active in this session.", "Corral ya est\u00e1 activo en esta sesi\u00f3n."))
    invisible(TRUE)
  }

  state_dir <- file.path(
    Sys.getenv("XDG_STATE_HOME", file.path(path.expand("~"), ".local", "state")),
    "corral"
  )
  pane_id <- Sys.getenv("HERDR_PANE_ID")
  session_id <- paste0(
    if (nzchar(pane_id)) gsub("[^A-Za-z0-9._-]", "-", pane_id) else "pid",
    "-", Sys.getpid()
  )
  session_dir <- file.path(state_dir, "sessions", session_id)

  # The packages Corral uses (jsonlite, httpgd, nanoparquet, duckdb, rlang) are in
  # your R library. A project with its own library (renv) leaves yours out of
  # .libPaths() and points R_LIBS_USER at its own; profile.R kept yours in
  # CORRAL_R_LIBS_USER. has() loads a package from the project's libraries first, so
  # the project's version wins, and else from yours, without adding it to
  # .libPaths(): the project's library stays as it is. In a session without its own
  # library, user_libs is empty and has() is requireNamespace(). A package that isn't
  # in your library either is remembered, so looking for it costs no more than before.
  user_libs <- local({
    l <- Sys.getenv("CORRAL_R_LIBS_USER", Sys.getenv("R_LIBS_USER"))
    l <- path.expand(strsplit(l, .Platform$path.sep, fixed = TRUE)[[1]])
    l <- l[nzchar(l) & dir.exists(l)]
    l[!normalizePath(l, mustWork = FALSE) %in% normalizePath(.libPaths(), mustWork = FALSE)]
  })
  not_in_user_libs <- character()
  has <- function(pkg) {
    if (requireNamespace(pkg, quietly = TRUE)) return(TRUE)
    if (!length(user_libs) || pkg %in% not_in_user_libs) return(FALSE)
    ok <- !inherits(try(loadNamespace(pkg, lib.loc = c(.libPaths(), user_libs)), silent = TRUE), "try-error")
    if (!ok) not_in_user_libs <<- c(not_in_user_libs, pkg)
    ok
  }
  has_json <- has("jsonlite")

  env$corral_bin <- function() {
    bin <- getOption("corral.bin", Sys.getenv("CORRAL_BIN"))
    if (!nzchar(bin)) bin <- Sys.which("corral")
    if (!nzchar(bin)) {
      stop(tr("`corral` is not on the PATH; set options(corral.bin = '/path/to/corral-term/rs/target/release/corral')",
              "`corral` no est\u00e1 en el PATH; define options(corral.bin = '/ruta/a/corral-term/rs/target/release/corral')"),
           call. = FALSE)
    }
    unname(bin)
  }

  in_herdr <- function() identical(Sys.getenv("HERDR_ENV"), "1")

  write_json_atomic <- function(x, path) {
    tmp <- paste0(path, ".tmp")
    jsonlite::write_json(x, tmp, auto_unbox = TRUE, na = "null", null = "null", digits = NA)
    file.rename(tmp, path)
  }

  # ---- session registry and variables (read by Mirador)

  value_preview <- function(x) {
    if (is.factor(x)) return(sprintf(tr("fct [%d] %d levels", "fct [%d] %d niveles"), length(x), nlevels(x)))
    abbr <- c(double = "num", integer = "int", character = "chr", logical = "lgl",
              complex = "cplx", raw = "raw")[[typeof(x)]]
    if (length(x) == 1L) {
      return(if (is.character(x)) paste0('"', x, '"') else format(x))
    }
    sprintf("%s [%d]", abbr, length(x))
  }

  # A single value that may be a secret shows no preview, in the variables pane or to an
  # agent (corral vars --json): its name says so (password, token, secret, API key...), or
  # its text has the shape of one (a known key's prefix, a JWT, a private key, a URL with a
  # password). The same patterns as SECRET_NAME and SECRET_TEXT in py/corral_py.py.
  secret_name <- "(?i)(pass(word|wd)?|pwd|secret|token|api[._-]?key|access[._-]?key|private[._-]?key|credential|bearer)"
  secret_text <- "^(sk-|sk_live_|rk_live_|ghp_|gho_|ghs_|github_pat_|glpat-|xox[abprs]-|(AKIA|ASIA)[0-9A-Z]{16}|AIza[0-9A-Za-z_-]{30,}|hf_|eyJ[A-Za-z0-9_-]+\\.[A-Za-z0-9_-]+\\.)|-----BEGIN|://[^/?#@:\\s]*:[^/?#@\\s]+@"
  may_be_secret <- function(name, x) {
    length(x) == 1L && (grepl(secret_name, name, perl = TRUE) ||
      (is.character(x) && !is.na(x) && grepl(secret_text, x, perl = TRUE)))
  }

  plot_classes <- c("ggplot", "gg", "trellis", "recordedplot", "grob", "gtable", "patchwork")

  plot_preview <- function(x) {
    what <- if (inherits(x, "patchwork")) "patchwork"
      else if (inherits(x, c("ggplot", "gg"))) "ggplot"
      else if (inherits(x, "trellis")) "lattice"
      else if (inherits(x, "recordedplot")) tr("base plot", "gr\u00e1fico base")
      else class(x)[1]
    title <- tryCatch(x$labels$title, error = function(e) NULL)
    if (is.character(title) && length(title) == 1L && nzchar(title)) paste(what, "\u00b7", title) else what
  }

  describe <- function(name, x) {
    kind <- if (inherits(x, plot_classes)) "plotobj"
      else if (is.data.frame(x) || is.matrix(x)) "data"
      else if (is.function(x)) "function"
      else if (is.atomic(x) || is.factor(x)) "value"
      else "other"
    n <- tryCatch(if (is.data.frame(x)) nrow(x) else length(x), error = function(e) NA_integer_)
    preview <- tryCatch(switch(kind,
      data = paste(dim(x), collapse = " × "),
      `function` = paste0("function(", paste(names(formals(x)), collapse = ", "), ")"),
      plotobj = plot_preview(x),
      value = if (may_be_secret(name, x)) tr("hidden: may be a secret", "oculto: puede ser una clave") else value_preview(x),
      other = if (is.list(x) && !is.object(x)) sprintf("list [%d]", length(x)) else paste(class(x), collapse = "/")
    ), error = function(e) class(x)[1])
    # object.size() walks every element; skip it for very large objects.
    # Plot objects carry their data and environments: sizing them on every command is slow.
    size <- if (is.environment(x) || kind == "plotobj" || isTRUE(n > 1e6)) NA_real_
      else tryCatch(as.numeric(utils::object.size(x)), error = function(e) NA_real_)
    # A fingerprint lets Corral reuse a drawing instead of printing the same plot again.
    hash <- if (kind == "plotobj" && has("rlang")) {
      tryCatch(rlang::hash(x), error = function(e) NULL)
    }
    list(name = name, kind = kind, class = class(x)[1],
         preview = substr(trimws(paste(preview, collapse = " ")), 1, 80), size = size, hash = hash)
  }

  # SQL needs duckdb and DBI. Looked up, not loaded (loading duckdb takes a while), and
  # once: `corral context` tells agents whether SQL works in this session.
  sql_ok <- NULL
  sql_available <- function() {
    if (is.null(sql_ok)) {
      found <- vapply(c("duckdb", "DBI"), function(p) system.file(package = p, lib.loc = c(.libPaths(), user_libs)), "")
      sql_ok <<- all(nzchar(found))
    }
    sql_ok
  }

  write_meta <- function() {
    dir.create(session_dir, recursive = TRUE, showWarnings = FALSE)
    write_json_atomic(list(
      id = session_id, pid = Sys.getpid(),
      pane_id = if (nzchar(pane_id)) pane_id else NULL,
      workspace_id = if (nzchar(Sys.getenv("HERDR_WORKSPACE_ID"))) Sys.getenv("HERDR_WORKSPACE_ID") else NULL,
      r_version = paste(R.version$major, R.version$minor, sep = "."),
      # Under Ark: the kernel's connection file, so `corral _lsp r` can reach its language server.
      ark_connection = if (nzchar(Sys.getenv("CORRAL_ARK_CONNECTION"))) Sys.getenv("CORRAL_ARK_CONNECTION") else NULL,
      cwd = getwd(), started_in = started_in, started = started, sql = sql_available()
    ), file.path(session_dir, "meta.json"))
  }

  started <- as.numeric(Sys.time())
  # Where it started: a project's security profile follows the session even if it moves.
  started_in <- getwd()
  last_cwd <- ""

  # After every command the environment is described again, but only objects
  # that changed: each one keeps a cheap signature (address in memory, length,
  # rows, and the value itself when it is a single number or string, since R
  # can modify an object in place without moving it) together with its
  # description and JSON. If no signature changed, nothing is done at all.
  # The address comes from base R (`.Internal(address())`, as fast as rlang's
  # and with no package to load at startup); rlang only where that is missing.
  address_of <- tryCatch({
    .Internal(address(globalenv()))
    function(x) .Internal(address(x))
  }, error = function(e) if (has("rlang")) rlang::obj_address)
  described <- new.env(hash = TRUE, parent = emptyenv())
  last_signature <- NULL

  # A list, compared with identical(): addresses stay pointers, no text needed.
  signature_of <- function(x) {
    list(address_of(x), length(x),
         if (is.data.frame(x)) .row_names_info(x, 2L) else 0L,
         if (is.atomic(x) && length(x) == 1L) format(x) else "")
  }

  snapshot <- function() {
    if (!identical(getwd(), last_cwd)) {
      last_cwd <<- getwd()
      write_meta()
    }
    genv <- globalenv()
    names <- sort(ls(genv))
    if (is.null(address_of)) {
      vars <- lapply(names, function(n) describe(n, get(n, envir = genv)))
      write_json_atomic(list(updated = as.numeric(Sys.time()), vars = vars), file.path(session_dir, "vars.json"))
      return(checkpoint(NULL))
    }
    sigs <- lapply(names, function(n) signature_of(get(n, envir = genv)))
    names(sigs) <- names
    # The variables pane first; then the checkpoint, which may fork.
    if (identical(sigs, last_signature)) return(checkpoint(sigs))
    parts <- character(length(names))
    for (i in seq_along(names)) {
      hit <- described[[names[i]]]
      if (is.null(hit) || !identical(hit$sig, sigs[[i]])) {
        d <- describe(names[i], get(names[i], envir = genv))
        json <- jsonlite::toJSON(d, auto_unbox = TRUE, na = "null", null = "null", digits = NA)
        hit <- list(sig = sigs[[i]], json = json)
        assign(names[i], hit, envir = described)
      }
      parts[i] <- hit$json
    }
    gone <- setdiff(ls(described), names)
    if (length(gone)) rm(list = gone, envir = described)
    path <- file.path(session_dir, "vars.json")
    tmp <- paste0(path, ".tmp")
    writeLines(paste0('{"updated":', format(as.numeric(Sys.time()), digits = 15), ',"vars":[',
                      paste(parts, collapse = ","), "]}"), tmp, useBytes = TRUE)
    file.rename(tmp, path)
    last_signature <<- sigs
    checkpoint(sigs)
  }

  # The httpgd device (it shows up as "unigd" in dev.list() since httpgd 2).
  # NA when there is none (not NULL: `is.na(NULL) || …` would go on and load unigd).
  # The number keeps its name (`d[1]`, not `d[[1]]`): unigd only takes a named one.
  plot_device <- function() {
    d <- grDevices::dev.list()
    d <- d[names(d) %in% c("unigd", "httpgd")]
    if (length(d)) d[1] else NA_integer_
  }

  # A plot object printed by hand (typing its name, Ctrl+R in the script):
  # remember which drawing it made, so opening it from the variables pane
  # shows that drawing instead of drawing it a second time.
  plot_pages <- 0L
  plot_rev <- ""
  remember_plot <- function(value, ok) {
    dev <- plot_device()
    if (is.na(dev) || !has("unigd")) {
      plot_pages <<- 0L
      return(invisible())
    }
    st <- unigd::ugd_state(dev)
    # Mirador waits for this file to change instead of asking every second and a half.
    rev <- paste(st$upid, st$hsize)
    if (!identical(rev, plot_rev)) {
      plot_rev <<- rev
      writeLines(rev, file.path(session_dir, "plots-rev"))
      keep_plots_now() # a new plot, one drawn over, or one removed: the checkpoint keeps them
    }
    pages <- st$hsize
    grew <- pages > plot_pages
    plot_pages <<- pages
    if (!isTRUE(ok) || !grew || !inherits(value, plot_classes) ||
        !has("rlang")) return(invisible())
    f <- file.path(session_dir, "plotobj-map.json")
    drawn <- if (file.exists(f)) tryCatch(jsonlite::read_json(f), error = function(e) list()) else list()
    drawn[[rlang::hash(value)]] <- as.character(unigd::ugd_id(0, which = dev)$id)
    write_json_atomic(drawn, f)
  }

  # ---- checkpoints: objects that outlive a crash
  #
  # After a command that changed an object, a forked copy of this R writes the
  # objects that changed to <state>/checkpoints/r-<session>/, one .rds each,
  # while you keep working: the prompt only waits for the fork (5 to 25 ms, more
  # with more memory in use), and a command that changed nothing costs nothing.
  # Once a minute the copy also hashes every object, for changes made in place.
  # One copy at a time; each one is collected after the next command. When R
  # ends because you closed it (q(), Ctrl+D, corral close) the checkpoint goes.
  # If it ends any other way (a crash, herdr or WSL closed), the next R in this
  # project attaches the packages again and loads the objects back.
  checkpoint_root <- file.path(state_dir, "checkpoints")
  checkpoint_dir <- file.path(checkpoint_root, paste0("r-", session_id))
  r_pid <- Sys.getpid()
  can_hash <- has("rlang")
  ck <- new.env(parent = emptyenv())
  ck$job <- NULL
  ck$sigs <- NULL
  ck$full_at <- as.numeric(Sys.time())

  # Corral's settings (config.json), read again only when the file changes.
  settings <- new.env(parent = emptyenv())
  corral_setting <- function(key) {
    home <- Sys.getenv("XDG_CONFIG_HOME")
    if (!nzchar(home)) home <- file.path(path.expand("~"), ".config")
    f <- file.path(home, "corral", "config.json")
    mtime <- suppressWarnings(file.mtime(f))
    if (!identical(settings$mtime, mtime)) {
      settings$text <- if (is.na(mtime)) "" else
        tryCatch(paste(readLines(f, warn = FALSE, encoding = "UTF-8"), collapse = "\n"), error = function(e) "")
      settings$mtime <- mtime
    }
    m <- regmatches(settings$text, regexec(sprintf('"%s"[[:space:]]*:[[:space:]]*"?([^",}]*)', key), settings$text))[[1]]
    if (length(m) == 2L) trimws(m[2]) else NA_character_
  }
  checkpoints_on <- function() !identical(corral_setting("checkpoints"), "off")
  checkpoint_max_mb <- function() {
    v <- suppressWarnings(as.numeric(corral_setting("checkpoint_max_mb")))
    if (is.na(v) || v <= 0) 1024 else v
  }
  checkpoint_every <- function() {
    v <- suppressWarnings(as.numeric(Sys.getenv("CORRAL_CHECKPOINT_EVERY")))
    if (is.na(v) || v <= 0) 60 else v
  }
  # Running, and not a zombie: Linux says so in /proc. macOS has no /proc, and there
  # kill(pid, 0) only says whether a process of this user has that pid (a zombie counts).
  on_linux <- startsWith(R.version$os, "linux")
  pid_alive <- function(pid) {
    pid <- suppressWarnings(as.integer(pid))
    if (length(pid) != 1L || is.na(pid) || pid <= 0L) return(FALSE)
    if (!on_linux) return(isTRUE(tools::pskill(pid, 0L)))
    stat <- tryCatch(readLines(sprintf("/proc/%d/stat", pid), warn = FALSE), error = function(e) "")
    length(stat) > 0L && nzchar(stat[1]) && !sub("^.*\\) ([A-Za-z]).*$", "\\1", stat[1]) %in% c("Z", "X")
  }

  # In the forked copy: write the objects in `changed` (all of them when `full`)
  # whose hash differs from the one written last, then the index. Nothing is
  # printed: this copy has no console.
  write_checkpoint <- function(changed, full) {
    suppressWarnings(suppressMessages(tryCatch({
      # Turned off since this copy started: no folder at all.
      if (!checkpoints_on()) return(FALSE)
      dir.create(checkpoint_dir, recursive = TRUE, showWarnings = FALSE, mode = "0700")
      closed <- file.path(checkpoint_dir, "closed")
      if (file.exists(closed)) return(FALSE)
      # A sensitive session's objects come back sensitive. corral_sensitive() sets
      # and lifts this mark at once; here it is only ever added, so a race can only
      # leave it on.
      if (file.exists(file.path(session_dir, "sensitive"))) writeLines("1", file.path(checkpoint_dir, "sensitive"))
      index_file <- file.path(checkpoint_dir, "index.json")
      old <- if (file.exists(index_file)) tryCatch(jsonlite::read_json(index_file), error = function(e) list()) else list()
      objects <- if (is.list(old$objects)) old$objects else list()
      skipped <- if (is.list(old$skipped)) old$skipped else list()
      genv <- globalenv()
      here <- ls(genv)
      drop <- function(n) {
        if (!is.null(objects[[n]])) unlink(file.path(checkpoint_dir, objects[[n]]$file))
        objects[[n]] <<- NULL
      }
      for (n in setdiff(names(objects), here)) drop(n)
      skipped <- skipped[intersect(names(skipped), here)]
      numbers <- suppressWarnings(as.integer(gsub("[^0-9]", "", vapply(objects, function(o) o$file, ""))))
      last <- max(c(0L, numbers), na.rm = TRUE)
      max_bytes <- checkpoint_max_mb() * 2^20
      for (n in here) {
        if (!full && !(n %in% changed) && (!is.null(objects[[n]]) || !is.null(skipped[[n]]))) next
        x <- get(n, envir = genv)
        size <- tryCatch(as.numeric(utils::object.size(x)), error = function(e) NA_real_)
        if (!is.na(size) && size > max_bytes) {
          drop(n)
          skipped[[n]] <- "size"
          next
        }
        h <- if (can_hash) tryCatch(rlang::hash(x), error = function(e) NA_character_) else NA_character_
        if (!is.na(h) && identical(objects[[n]]$hash, h)) next
        file <- objects[[n]]$file
        if (is.null(file)) {
          last <- last + 1L
          file <- sprintf("o%05d.rds", last)
        }
        path <- file.path(checkpoint_dir, file)
        # A connection or a model held outside R (an external pointer) can't come back.
        pointer <- FALSE
        ok <- tryCatch({
          saveRDS(x, paste0(path, ".tmp"), compress = FALSE,
                  refhook = function(r) { if (typeof(r) == "externalptr") pointer <<- TRUE; NULL })
          TRUE
        }, error = function(e) FALSE)
        if (!ok || pointer) {
          unlink(paste0(path, ".tmp"))
          drop(n)
          skipped[[n]] <- if (pointer) "pointer" else "error"
          next
        }
        file.rename(paste0(path, ".tmp"), path)
        objects[[n]] <- list(file = file, hash = h)
        skipped[[n]] <- NULL
      }
      if (file.exists(closed) || !checkpoints_on()) return(unlink(checkpoint_dir, recursive = TRUE))
      write_json_atomic(list(lang = "r", pid = r_pid, session = session_id, cwd = getwd(),
                             updated = as.numeric(Sys.time()), packages = .packages(),
                             objects = objects, skipped = skipped), index_file)
      # Checkpoints turned off while it wrote, after its console may have ended and the
      # cleanup run: the copy goes with what it wrote.
      if (!checkpoints_on()) return(unlink(checkpoint_dir, recursive = TRUE))
      TRUE
    }, error = function(e) FALSE)))
  }

  # The plots this R drew, into its checkpoint (<checkpoint>/plots/, one SVG and one PNG
  # per plot, named by its id in httpgd), so a crash doesn't take them. `corral
  # _keep-plots` does it: one per session, started with the first plot, it watches
  # plots-rev and asks httpgd over HTTP. Here R only checks that it still runs (0.1 ms)
  # and starts it again if it doesn't. Not in the forked copy: httpgd's server thread
  # may hold a lock at the fork, and unigd then hangs the copy for good, and every
  # checkpoint after it. Not started for each plot either: system2() forks R, 8 ms
  # with 400-650 MB in use. Rendered by httpgd for another process, R's memory stays put.
  keep_bin <- NULL
  keep_plots_now <- function() {
    if (!checkpoints_on()) return(invisible())
    pid_file <- file.path(session_dir, "keep-plots.pid")
    pid <- if (file.exists(pid_file)) suppressWarnings(as.integer(readLines(pid_file, n = 1L, warn = FALSE))) else NA_integer_
    if (!on_linux) {
      # No /proc to read its arguments from (macOS): the watcher touches its pid file
      # every 5 s, so a live pid with a fresh file is it. One that stopped leaves the file
      # to age, even if another process gets its pid. A second watcher would find the
      # first's lock taken and leave anyway.
      age <- as.numeric(Sys.time()) - as.numeric(file.mtime(pid_file))
      if (pid_alive(pid) && isTRUE(age < 30)) return(invisible())
    } else {
      cmdline <- sprintf("/proc/%d/cmdline", if (length(pid) == 1L && !is.na(pid)) pid else 0L)
      if (file.exists(cmdline)) {
        # Its own watcher: another session's could have the pid now.
        cmd <- tryCatch(readBin(cmdline, "raw", 512L), error = function(e) raw())
        cmd[cmd == as.raw(0L)] <- as.raw(32L) # the arguments, separated by spaces
        if (grepl(paste0(" _keep-plots ", session_id, " "), rawToChar(cmd), fixed = TRUE)) return(invisible())
      }
    }
    if (is.null(keep_bin)) keep_bin <<- tryCatch(env$corral_bin(), error = function(e) "")
    if (nzchar(keep_bin)) {
      system2(keep_bin, c("_keep-plots", session_id, r_pid), wait = FALSE, stdout = FALSE, stderr = FALSE)
    }
    invisible()
  }

  # At startup, from the checkpoint that comes back: its plots (those it had kept from
  # an earlier crash, then its own, in order) move to this session's checkpoint, into
  # kept/ as k0001.svg/.png and so on, the newest 30. Mirador and the terminal's viewer
  # show them before this session's own. How many came along.
  keep_plots <- function(old) {
    stems <- character()
    for (d in file.path(old, c("kept", "plots"))) {
      f <- list.files(d, pattern = "\\.(svg|png)$")
      s <- unique(sub("\\.[a-z]+$", "", f))
      s <- if (basename(d) == "kept") sort(s) else s[order(suppressWarnings(as.integer(s)))]
      stems <- c(stems, file.path(d, s))
    }
    stems <- utils::tail(stems, 30)
    if (!length(stems)) return(0L)
    dest <- file.path(checkpoint_dir, "kept")
    dir.create(dest, recursive = TRUE, showWarnings = FALSE, mode = "0700")
    for (i in seq_along(stems)) {
      for (ext in c("svg", "png")) {
        from <- paste0(stems[i], ".", ext)
        if (file.exists(from)) file.rename(from, file.path(dest, sprintf("k%04d.%s", i, ext)))
      }
    }
    length(stems)
  }

  # Checkpoints turned off while this R runs: its copy goes at the next command (once
  # a copy being written has finished), not when it closes; turned on again, the next
  # copy is a whole one.
  drop_own_checkpoint <- function() {
    if (!dir.exists(checkpoint_dir)) return(invisible())
    if (!is.null(ck$job)) {
      if (is.null(parallel::mccollect(ck$job, wait = FALSE))) return(invisible())
      ck$job <- NULL
    }
    unlink(checkpoint_dir, recursive = TRUE)
    ck$sigs <- NULL
    invisible()
  }

  # After each command: start a copy when something changed (or a minute went
  # by), unless the last one is still writing; then it waits for the next command.
  checkpoint <- function(sigs) {
    if (!checkpoints_on()) return(drop_own_checkpoint())
    if (!is.null(ck$job)) {
      if (is.null(parallel::mccollect(ck$job, wait = FALSE))) return(invisible())
      ck$job <- NULL
    }
    now <- as.numeric(Sys.time())
    full <- can_hash && now - ck$full_at > checkpoint_every()
    changed <- names(sigs)[!vapply(names(sigs), function(n) identical(sigs[[n]], ck$sigs[[n]]), TRUE)]
    gone <- setdiff(names(ck$sigs), names(sigs))
    if (!full && !length(changed) && !length(gone) && !is.null(ck$sigs)) return(invisible())
    if (full) ck$full_at <- now
    ck$sigs <- sigs
    ck$job <- parallel::mcparallel(write_checkpoint(changed, full), silent = TRUE)
    invisible()
  }

  # The console reads the notice as soon as it exists: whole, or not at all.
  write_notice <- function(lines, path) {
    tmp <- paste0(path, ".tmp")
    writeLines(lines, tmp)
    file.rename(tmp, path)
  }

  # At startup: the newest checkpoint of an R in this project that ended
  # without being closed comes back, packages first.
  restore_checkpoint <- function() {
    if (!dir.exists(checkpoint_root)) return(invisible())
    if (!checkpoints_on()) {
      # Off: nothing comes back, and no copy stays once its console has ended, closed
      # or not, R's or Python's. Each folder is named after its console's pid.
      for (d in list.dirs(checkpoint_root, recursive = FALSE)) {
        pid <- suppressWarnings(as.integer(sub("^.*-([0-9]+)$", "\\1", basename(d))))
        if (file.exists(file.path(d, "closed")) || is.na(pid) || !pid_alive(pid)) unlink(d, recursive = TRUE)
      }
      return(invisible())
    }
    best <- NULL
    for (d in list.dirs(checkpoint_root, recursive = FALSE)) {
      if (file.exists(file.path(d, "closed"))) {
        unlink(d, recursive = TRUE)
        next
      }
      idx <- tryCatch(jsonlite::read_json(file.path(d, "index.json")), error = function(e) NULL)
      if (is.null(idx)) {
        # Never got its index (kept plots only, say): gone after a week too.
        age <- as.numeric(Sys.time()) - as.numeric(file.mtime(d))
        if (!is.na(age) && age > 7 * 86400) unlink(d, recursive = TRUE)
        next
      }
      if (is.null(idx$pid) || pid_alive(idx$pid)) next
      if (as.numeric(Sys.time()) - as.numeric(idx$updated) > 7 * 86400) {
        unlink(d, recursive = TRUE) # a week old: nobody is coming back for it
        next
      }
      if (!identical(idx$lang, "r") || !identical(idx$cwd, getwd())) next
      if (is.null(best) || idx$updated > best$idx$updated) best <- list(dir = d, idx = idx)
    }
    if (is.null(best)) return(invisible())
    idx <- best$idx
    # Marked sensitive before loading anything: an agent never reaches these objects
    # in a session that isn't.
    sensitive <- file.exists(file.path(best$dir, "sensitive"))
    if (sensitive) {
      dir.create(session_dir, recursive = TRUE, showWarnings = FALSE)
      writeLines("1", file.path(session_dir, "sensitive"))
    }
    pkgs <- character()
    for (p in rev(unlist(idx$packages))) {
      if (p %in% .packages()) next
      ok <- !inherits(try(suppressPackageStartupMessages(library(p, character.only = TRUE)), silent = TRUE), "try-error")
      if (ok) pkgs <- c(pkgs, p)
    }
    back <- character()
    for (n in names(idx$objects)) {
      ok <- tryCatch({
        assign(n, readRDS(file.path(best$dir, idx$objects[[n]]$file)), envir = globalenv())
        TRUE
      }, error = function(e) FALSE)
      if (ok) back <- c(back, n)
    }
    plots <- tryCatch(keep_plots(best$dir), error = function(e) 0L)
    unlink(best$dir, recursive = TRUE)
    left <- names(idx$skipped)
    why <- vapply(left, function(n) switch(idx$skipped[[n]],
      pointer = tr("a connection", "una conexi\u00f3n"),
      size = trf("over %d MB", "de m\u00e1s de %d MB", checkpoint_max_mb()),
      tr("couldn't be saved", "no se pudo guardar")), "")
    when <- format(as.POSIXct(as.numeric(idx$updated), origin = "1970-01-01"), "%Y-%m-%d %H:%M")
    shown <- function(x) if (length(x) > 8) paste(c(x[1:8], "\u2026"), collapse = ", ") else paste(x, collapse = ", ")
    # What came back, in one line: the objects and the packages by name, and the plots.
    named <- function(n, one, many, x) paste0(n, " ", if (n == 1) one else many, if (length(x)) paste0(" (", shown(x), ")") else "")
    parts <- c(
      if (length(back)) named(length(back), tr("object", "objeto"), tr("objects", "objetos"), back),
      if (length(pkgs)) named(length(pkgs), tr("package", "paquete"), tr("packages", "paquetes"), rev(pkgs)),
      if (plots > 0) named(plots, tr("plot", "gr\u00e1fico"), tr("plots", "gr\u00e1ficos"), NULL))
    what <- if (length(parts) > 1) paste(paste(parts[-length(parts)], collapse = ", "), tr("and", "y"), parts[length(parts)]) else parts
    mirador <- if (plots > 0) tr(", first in Mirador (ctrl+b f)", ", primero en Mirador (ctrl+b f)") else ""
    # Started again by Corral's console after a crash: it says when, and so how long it took.
    crashed <- suppressWarnings(as.numeric(Sys.getenv("CORRAL_CRASHED_AT")))
    lines <- if (!is.na(crashed)) {
      secs <- format(round(as.numeric(Sys.time()) - crashed, 1), nsmall = 1, decimal.mark = tr(".", ","))
      if (length(parts)) {
        trf("[corral] Back in %s s with %s%s.", "[corral] Volvi\u00f3 en %s s con %s%s.", secs, what, mirador)
      } else {
        trf("[corral] Back in %s s, with nothing to bring back.", "[corral] Volvi\u00f3 en %s s, sin nada que recuperar.", secs)
      }
    } else if (length(parts)) {
      trf("[corral] Restored %s from the R that ended without being closed (%s)%s.",
          "[corral] Restaur\u00e9 %s del R que termin\u00f3 sin que lo cerraran (%s)%s.", what, when, mirador)
    }
    if (length(left)) {
      lines <- c(lines, trf("[corral] Not kept: %s.", "[corral] No se conservaron: %s.",
                            paste(sprintf("%s (%s)", left, why), collapse = ", ")))
    }
    if (sensitive) {
      lines <- c(lines, tr("[corral] That session was marked sensitive, and so is this one: agents need your approval and only see the structure of results.",
                           "[corral] Esa sesi\u00f3n estaba marcada como sensible, y esta tambi\u00e9n: los agentes necesitan tu aprobaci\u00f3n y solo ven la estructura de los resultados."))
    }
    # Under Ark, what R prints while starting doesn't reach the console: Corral's
    # console shows this file instead.
    notice <- Sys.getenv("CORRAL_NOTICE")
    if (nzchar(notice)) write_notice(lines, notice) else message(paste(lines, collapse = "\n"))
    invisible()
  }

  if (has_json && !isFALSE(getOption("corral.vars"))) {
    try(restore_checkpoint(), silent = TRUE)
    try(snapshot(), silent = TRUE)
    # CORRAL_TIMING=<file>: how long Corral's work after each command keeps the
    # prompt waiting, in ms, one line per command (for measuring Corral itself).
    timing <- Sys.getenv("CORRAL_TIMING")
    addTaskCallback(function(expr, value, ok, visible) {
      t0 <- Sys.time()
      try(remember_plot(value, ok), silent = TRUE)
      try(snapshot(), silent = TRUE)
      if (nzchar(timing)) cat(sprintf("%.3f\n", 1000 * as.numeric(Sys.time() - t0, units = "secs")), file = timing, append = TRUE)
      TRUE
    }, name = "corral")
    # R closed on purpose: the session and its checkpoint go (a crash skips this).
    reg.finalizer(env, function(e) unlink(c(session_dir, checkpoint_dir), recursive = TRUE), onexit = TRUE)
  }
  # Started again after a crash: Corral's console waits for the notice before it runs
  # anything typed meanwhile, so it is written even when there was nothing to bring back.
  local({
    notice <- Sys.getenv("CORRAL_NOTICE")
    if (nzchar(Sys.getenv("CORRAL_CRASHED_AT")) && nzchar(notice) && !file.exists(notice)) write_notice(character(), notice)
  })

  # ---- corral eval: an agent runs code in this session and reads the result
  #
  # `corral eval` writes the code to <session>/eval/<id>.R and types
  # .corral_eval("<id>") in this console. The code runs in the global
  # environment with echo, so you see it as if typed; the output is also
  # written to <id>.out and the outcome to <id>.json for the agent.

  env$.corral_eval <- function(id) {
    dir <- file.path(session_dir, "eval")
    path <- function(ext) file.path(dir, paste0(id, ext))
    if (!file.exists(path(".R"))) return(invisible())
    status <- list(status = "ok", error = NULL, warnings = list())
    done <- function() {
      status$finished <- as.numeric(Sys.time())
      if (has_json) write_json_atomic(status, path(".json"))
      else writeLines(status$status, path(".json"))
    }
    deadline <- suppressWarnings(as.numeric(readLines(path(".deadline"), warn = FALSE)))
    if (length(deadline) == 1L && !is.na(deadline) && as.numeric(Sys.time()) > deadline) {
      cat(tr("# corral eval: the request expired before it started; it did not run\n",
             "# corral eval: la petici\u00f3n venci\u00f3 antes de empezar; no se ejecut\u00f3\n"))
      status$status <- "expired"
      return(invisible(done()))
    }
    cat("# corral eval\n")
    # Control or invisible characters can hide part of the echoed code: corral eval
    # then writes the code out with them shown, and this prints it first.
    if (file.exists(path(".shown"))) {
      cat(tr("# corral eval: this code has control or invisible characters; written out, it says:\n",
             "# corral eval: este c\u00f3digo tiene caracteres de control o invisibles; escrito tal cual, dice:\n"))
      cat(paste0("#   ", readLines(path(".shown"), warn = FALSE, encoding = "UTF-8"), "\n"), sep = "")
    }
    con <- file(path(".out"), open = "wt", encoding = "UTF-8")
    sink(con, split = TRUE)
    on.exit({ sink(); close(con); done() }, add = TRUE)
    tryCatch(
      withCallingHandlers(
        source(path(".R"), echo = TRUE, print.eval = TRUE, local = globalenv(),
               max.deparse.length = Inf, spaced = FALSE, keep.source = TRUE,
               prompt.echo = getOption("prompt"), continue.echo = getOption("continue"),
               encoding = "UTF-8"),
        message = function(m) {
          cat(conditionMessage(m))
          invokeRestart("muffleMessage")
        },
        warning = function(w) {
          status$warnings[[length(status$warnings) + 1L]] <<- conditionMessage(w)
        }
      ),
      error = function(e) {
        call <- conditionCall(e)
        # Top-level errors carry source()'s own eval() call; R hides it at the prompt, so do we.
        top <- is.null(call) || identical(call, quote(eval(ei, envir)))
        where <- if (top) "" else paste0(" in ", paste(deparse(call), collapse = " "))
        msg <- paste0("Error", where, ": ", conditionMessage(e))
        cat(msg, "\n", sep = "")
        status$status <<- "error"
        status$error <<- msg
      }
    )
    invisible()
  }

  # ---- tables

  write_table <- function(x) {
    if (!is.data.frame(x)) x <- as.data.frame(x)
    # Keep meaningful row names (mtcars) as a column; parquet has no row names.
    if (.row_names_info(x) > 0L) x <- cbind(.rownames = rownames(x), x, stringsAsFactors = FALSE)
    # A duration as a number in its own units: 1.5 hours shows 1.5, not nanoseconds.
    durations <- vapply(x, inherits, logical(1), "difftime")
    if (any(durations)) {
      x <- as.data.frame(x)
      x[durations] <- lapply(x[durations], as.numeric)
    }
    # nanoparquet first: faster than arrow and lighter on memory (bench/view.R).
    # arrow takes what nanoparquet can't write, such as POSIXlt.
    writers <- list(
      nanoparquet = function(f) nanoparquet::write_parquet(x, f),
      arrow = function(f) arrow::write_parquet(x, f)
    )
    for (pkg in names(writers)) {
      if (!has(pkg)) next
      f <- tempfile("corral-", fileext = ".parquet")
      ok <- tryCatch({ writers[[pkg]](f); TRUE }, error = function(e) FALSE)
      if (ok) return(f)
    }
    f <- tempfile("corral-", fileext = ".csv")
    utils::write.csv(x, f, row.names = FALSE)
    f
  }

  # The terminal viewer by default; mirador = TRUE opens the table in Mirador, in the browser.
  env$corral_view <- function(x, title = NULL, mirador = FALSE) {
    if (is.null(title)) title <- paste(deparse(substitute(x)), collapse = " ")
    f <- write_table(x)
    # Mirador and herdr popups are separate from this terminal, so R stays free.
    # Elsewhere the viewer takes over this terminal until you quit it.
    system2(env$corral_bin(),
            c("view", shQuote(f), "--title", shQuote(title), "--session", session_id, "--temp",
              if (isTRUE(mirador)) "--mirador"),
            wait = !in_herdr() && !isTRUE(mirador))
    invisible(f)
  }

  # A sensitive session: every request from an agent waits for your approval, and
  # the agent sees only whether it worked and the shape of the objects, never values.
  # It lasts until this R ends, or comes back with the objects after a crash;
  # corral_sensitive(FALSE) lifts it.
  env$corral_sensitive <- function(on = TRUE) {
    flag <- file.path(session_dir, "sensitive")
    mark <- file.path(checkpoint_dir, "sensitive")
    if (isTRUE(on)) {
      dir.create(session_dir, recursive = TRUE, showWarnings = FALSE)
      writeLines("1", flag)
      if (dir.exists(checkpoint_dir)) writeLines("1", mark)
      message(tr("Sensitive session: agents need your approval for each request and only see the structure of results, not values.",
                 "Sesi\u00f3n sensible: los agentes necesitan tu aprobaci\u00f3n en cada petici\u00f3n y solo ven la estructura de los resultados, no los valores."))
    } else {
      unlink(c(flag, mark))
      message(tr("This session is no longer marked sensitive.", "Esta sesi\u00f3n ya no est\u00e1 marcada como sensible."))
    }
    invisible(isTRUE(on))
  }

  env$View <- function(x, title, mirador = FALSE) {
    if (missing(title)) title <- paste(deparse(substitute(x)), collapse = " ")
    env$corral_view(x, title, mirador = mirador)
  }

  # ---- SQL

  # SQL runs in this session, on DuckDB: your data frames are tables under their
  # own names, and each result is a data frame here (sql_result, or the name a
  # `-- name: x` line gives it). DuckDB loads with the first query, so a session
  # that never runs SQL doesn't pay for it.
  sql_db <- NULL
  sql_ours <- character()

  sql_connection <- function() {
    if (!is.null(sql_db) && DBI::dbIsValid(sql_db)) return(sql_db)
    if (!has("duckdb") || !has("DBI")) {
      # net: text
      stop(tr("SQL needs the duckdb package: install.packages(\"duckdb\")",
              "SQL necesita el paquete duckdb: install.packages(\"duckdb\")"), call. = FALSE)
    }
    # duckdb 2 asks in an interactive R whether to create ~/.duckdb, and the console
    # would wait for the answer (an agent too): use it when it is there, shared with
    # the DuckDB CLI, else a folder for this session. A place you set yourself
    # (the duckdb.home option, DUCKDB_R_HOME) still wins.
    drv <- list()
    if ("shared_home" %in% names(formals(duckdb::duckdb)) &&
        is.null(getOption("duckdb.home")) && !nzchar(Sys.getenv("DUCKDB_R_HOME"))) {
      home <- Sys.getenv("HOME", path.expand("~"))
      drv$shared_home <- dir.exists(file.path(home, ".duckdb"))
    }
    sql_db <<- suppressMessages(DBI::dbConnect(do.call(duckdb::duckdb, drv)))
    sql_ours <<- character()
    sql_compile()
    sql_db
  }

  # R compiles a function the second time it runs, and these take about 40 ms:
  # compile them now, while DuckDB loads anyway, so the next query doesn't wait.
  sql_compile <- function() {
    home <- environment(sql_compile)
    for (f in c("sql_register", "sql_statements", "sql_is_query", "sql_name", "sql_code")) {
      assign(f, compiler::cmpfun(get(f, envir = home)), envir = home)
    }
    fast <- compiler::cmpfun(env$corral_sql)
    env$corral_sql <- fast
    if ("corral" %in% search()) assign("corral_sql", fast, envir = as.environment("corral"))
  }

  # The code of a statement, without its comments (nested ones too, as in DuckDB).
  sql_code <- function(x) gsub("(?<c>/\\*(?:[^/*]|/(?!\\*)|\\*(?!/)|(?&c))*\\*/)|--[^\n]*", "", x, perl = TRUE)

  # The statements in a piece of SQL: split at each ; outside strings, quoted
  # names and comments, found with one regular expression (a loop over the
  # characters would cost R's compiler far more than the SQL itself). It knows
  # what DuckDB's own parser knows: '' inside strings, E'..' with backslashes,
  # $$..$$ and $tag$..$tag$, and comments inside comments. Pieces with only
  # comments are dropped.
  sql_statements <- function(text) {
    m <- gregexpr("(?<c>/\\*(?:[^/*]|/(?!\\*)|\\*(?!/)|(?&c))*\\*/)|--[^\\n]*|\\$([A-Za-z_]\\w*|)\\$[\\s\\S]*?\\$\\2\\$|\\b[eE]'(?:[^'\\\\]|\\\\[\\s\\S])*'|'[^']*'|\"[^\"]*\"|;", text, perl = TRUE)[[1]]
    ends <- m[regmatches(text, list(m))[[1]] == ";"]
    starts <- c(1L, ends + 1L)
    stops <- c(ends, nchar(text))
    out <- trimws(sub(";\\s*$", "", substring(text, starts, stops)))
    out[nzchar(trimws(sql_code(out)))]
  }

  sql_is_query <- function(stmt) {
    grepl("^[\\s(]*(select|with|from|values|table|describe|show|summarize|pivot|unpivot|explain|call|pragma)\\b",
          tolower(sql_code(stmt)), perl = TRUE)
  }

  # `-- name: monthly` in a statement names its result.
  sql_name <- function(stmt) {
    m <- regmatches(stmt, regexec("(?m)^\\s*--\\s*name:\\s*([A-Za-z_][A-Za-z0-9_.]*)", stmt, perl = TRUE))[[1]]
    if (length(m) == 2L) m[[2L]] else NULL
  }

  # The data frames a statement names become views of DuckDB's, without a copy.
  # A DuckDB table of the same name keeps its name.
  sql_register <- function(con, stmt) {
    code <- sql_code(stmt)
    words <- unique(regmatches(code, gregexpr("[A-Za-z_][A-Za-z0-9_]*", code))[[1]])
    frames <- Filter(function(w) is.data.frame(get(w, envir = globalenv())), intersect(words, ls(globalenv())))
    if (!length(frames)) return(invisible())
    own <- DBI::dbGetQuery(con, "SELECT table_name FROM duckdb_tables()")$table_name
    for (w in frames) {
      if (!(w %in% sql_ours) && w %in% own) next
      duckdb::duckdb_register(con, w, get(w, envir = globalenv()), overwrite = TRUE)
      sql_ours <<- union(sql_ours, w)
    }
  }

  # DuckDB's message, without the R binding's own lines.
  sql_error <- function(e) {
    msg <- strsplit(sub("^rapi_[a-z_]+: ", "", conditionMessage(e)), "\n")[[1]]
    msg <- msg[!grepl("^ℹ (Context|Error type):", msg)]
    paste(trimws(msg, "right"), collapse = "\n")
  }

  env$corral_sql <- function(query, name = NULL) {
    con <- sql_connection()
    statements <- sql_statements(paste(query, collapse = "\n"))
    if (!length(statements)) return(invisible(NULL))
    t0 <- proc.time()[["elapsed"]]
    last <- NULL
    last_name <- NULL
    for (s in statements) {
      sql_register(con, s)
      # The error is raised out here, so the console's trace shows corral_sql() only.
      failed <- tryCatch({
        if (sql_is_query(s)) {
          last <- suppressWarnings(DBI::dbGetQuery(con, s))
          last_name <- sql_name(s)
          if (!is.null(last_name)) assign(last_name, last, envir = globalenv())
        } else {
          DBI::dbExecute(con, s)
        }
        NULL
      }, error = function(e) e)
      if (!is.null(failed)) stop(sql_error(failed), call. = FALSE)
    }
    secs <- proc.time()[["elapsed"]] - t0
    took <- if (secs < 1) sprintf("%.0f ms", secs * 1000) else sprintf("%.1f s", secs)
    if (is.null(last)) {
      cat(sprintf("# %s \u00b7 %s\n", tr("done", "listo"), took))
      return(invisible(NULL))
    }
    target <- if (!is.null(name)) name else if (!is.null(last_name)) last_name else "sql_result"
    assign(target, last, envir = globalenv())
    print(utils::head(last, 10L))
    rows <- if (nrow(last) == 1L) tr("row", "fila") else tr("rows", "filas")
    cat(sprintf("# %s %s \u00b7 %s \u2192 %s\n", format(nrow(last), big.mark = ","), rows, took, target))
    invisible(last)
  }

  # ---- plots

  # corral.viewer: "herdr" (popup with ctrl+b f), "mirador" (browser) or "httpgd" (its own page).
  open_viewer <- function(url, force = FALSE) {
    if (!force && !isTRUE(getOption("corral.open_plots", TRUE))) return(invisible())
    viewer <- getOption("corral.viewer", if (in_herdr()) "herdr" else "mirador")
    args <- switch(viewer,
      httpgd = c("open", shQuote(url)),
      mirador = c("mirador", if (force) "--open"),
      if (force) "plots" else "_plot-ready"
    )
    try(system2(env$corral_bin(), args, wait = FALSE, stdout = FALSE, stderr = FALSE), silent = TRUE)
  }

  start_httpgd <- function(...) {
    # Loaded through has(), so in a project with its own library it comes from yours.
    if (!has("httpgd")) stop(tr("install httpgd first", "instala httpgd primero"), call. = FALSE)
    httpgd::hgd(silent = TRUE)
    url <- httpgd::hgd_url()
    dir.create(session_dir, recursive = TRUE, showWarnings = FALSE)
    writeLines(url, file.path(session_dir, "plots-url"))
    writeLines(url, file.path(state_dir, "plots-url"))
    open_viewer(url)
    invisible(url)
  }

  env$corral_plots <- function() {
    if (!has("httpgd")) stop(tr("install httpgd first", "instala httpgd primero"), call. = FALSE)
    if (is.na(plot_device())) return(start_httpgd())
    url <- httpgd::hgd_url()
    open_viewer(url, force = TRUE)
    invisible(url)
  }

  # grDevices only sets `device` when it is still unset, so this survives startup.
  # httpgd is only looked up here, not loaded: it (with unigd, systemfonts,
  # lifecycle and cli) loads with the first plot, so a session without plots
  # starts faster and uses less memory.
  use_httpgd <- function() {
    if (nzchar(system.file(package = "httpgd", lib.loc = c(.libPaths(), user_libs))) && !isFALSE(getOption("corral.httpgd"))) {
      options(device = start_httpgd)
    }
    invisible()
  }
  use_httpgd()
  # Under Ark (Positron's R kernel), keep these: after .Rprofile Ark points them
  # at Positron's panes, which don't exist here, unless they are protected.
  if (nzchar(Sys.getenv("CORRAL_ARK_CONNECTION"))) {
    options(ark.protected_options = union(
      getOption("ark.protected_options"),
      c("device", "browser", "editor", "askpass", "viewer")
    ))
    # Ark shows help as HTML (?mean): open those pages in a browser window.
    options(browser = function(url) {
      try(system2(env$corral_bin(), c("open", shQuote(url)), wait = FALSE), silent = TRUE)
      invisible()
    })
  }

  attach_env(env)

  invisible()
})
