# corral.R: R side of Corral.
#
# Add to ~/.Rprofile:
#   if (interactive()) try(source("~/workspace/personal/corral-term/r/corral.R"))
#
# What it does in an interactive terminal session (not in RStudio or Positron):
#   * Registers this session for Mirador and, after every top-level command,
#     writes a light description of the global environment (needs jsonlite).
#   * View(x) writes x to a temporary parquet file and opens it with `corral view`
#     (a popup inside herdr; a tab in Mirador outside herdr).
#   * Plots go to httpgd. Inside herdr, ctrl+b f (or corral_plots()) shows them in
#     a popup; outside herdr the first plot opens Mirador. Plot history lives in
#     this R process, so closing a viewer loses nothing.

local({
  if (!interactive()) return(invisible())
  if (nzchar(Sys.getenv("RSTUDIO")) || nzchar(Sys.getenv("POSITRON"))) return(invisible())

  env <- new.env()

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
  has_json <- requireNamespace("jsonlite", quietly = TRUE)

  env$corral_bin <- function() {
    bin <- getOption("corral.bin", Sys.getenv("CORRAL_BIN"))
    if (!nzchar(bin)) bin <- Sys.which("corral")
    if (!nzchar(bin)) {
      stop("`corral` is not on PATH; set options(corral.bin = '/path/to/corral-term/bin/corral')",
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
    if (is.factor(x)) return(sprintf("fct [%d] %d niveles", length(x), nlevels(x)))
    abbr <- c(double = "num", integer = "int", character = "chr", logical = "lgl",
              complex = "cplx", raw = "raw")[[typeof(x)]]
    if (length(x) == 1L) {
      return(if (is.character(x)) paste0('"', x, '"') else format(x))
    }
    sprintf("%s [%d]", abbr, length(x))
  }

  describe <- function(name, x) {
    kind <- if (is.data.frame(x) || is.matrix(x)) "data"
      else if (is.function(x)) "function"
      else if (is.atomic(x) || is.factor(x)) "value"
      else "other"
    n <- tryCatch(if (is.data.frame(x)) nrow(x) else length(x), error = function(e) NA_integer_)
    preview <- tryCatch(switch(kind,
      data = paste(dim(x), collapse = " × "),
      `function` = paste0("function(", paste(names(formals(x)), collapse = ", "), ")"),
      value = value_preview(x),
      other = if (is.list(x) && !is.object(x)) sprintf("list [%d]", length(x)) else paste(class(x), collapse = "/")
    ), error = function(e) class(x)[1])
    # object.size() walks every element; skip it for very large objects.
    size <- if (is.environment(x) || isTRUE(n > 1e6)) NA_real_
      else tryCatch(as.numeric(utils::object.size(x)), error = function(e) NA_real_)
    list(name = name, kind = kind, class = class(x)[1],
         preview = substr(trimws(paste(preview, collapse = " ")), 1, 80), size = size)
  }

  write_meta <- function() {
    dir.create(session_dir, recursive = TRUE, showWarnings = FALSE)
    write_json_atomic(list(
      id = session_id, pid = Sys.getpid(),
      pane_id = if (nzchar(pane_id)) pane_id else NULL,
      workspace_id = if (nzchar(Sys.getenv("HERDR_WORKSPACE_ID"))) Sys.getenv("HERDR_WORKSPACE_ID") else NULL,
      r_version = paste(R.version$major, R.version$minor, sep = "."),
      cwd = getwd(), started = started
    ), file.path(session_dir, "meta.json"))
  }

  started <- as.numeric(Sys.time())
  last_cwd <- ""

  snapshot <- function() {
    if (!identical(getwd(), last_cwd)) {
      last_cwd <<- getwd()
      write_meta()
    }
    genv <- globalenv()
    names <- sort(ls(genv))
    vars <- lapply(names, function(n) describe(n, get(n, envir = genv)))
    write_json_atomic(list(updated = as.numeric(Sys.time()), vars = vars),
                      file.path(session_dir, "vars.json"))
  }

  if (has_json && !isFALSE(getOption("corral.vars"))) {
    try(snapshot(), silent = TRUE)
    addTaskCallback(function(...) { try(snapshot(), silent = TRUE); TRUE }, name = "corral")
    reg.finalizer(env, function(e) unlink(session_dir, recursive = TRUE), onexit = TRUE)
  }

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
      cat("# corral eval: la petición venció antes de empezar; no se ejecutó\n")
      status$status <- "expired"
      return(invisible(done()))
    }
    cat("# corral eval\n")
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
    writers <- list(
      arrow = function(f) arrow::write_parquet(x, f),
      nanoparquet = function(f) nanoparquet::write_parquet(x, f)
    )
    for (pkg in names(writers)) {
      if (!requireNamespace(pkg, quietly = TRUE)) next
      f <- tempfile("corral-", fileext = ".parquet")
      ok <- tryCatch({ writers[[pkg]](f); TRUE }, error = function(e) FALSE)
      if (ok) return(f)
    }
    f <- tempfile("corral-", fileext = ".csv")
    utils::write.csv(x, f, row.names = FALSE)
    f
  }

  env$corral_view <- function(x, title = NULL) {
    if (is.null(title)) title <- paste(deparse(substitute(x)), collapse = " ")
    f <- write_table(x)
    # Mirador and herdr popups are separate from this terminal, so R stays free.
    # Elsewhere the viewer takes over this terminal until you quit it.
    system2(env$corral_bin(),
            c("view", shQuote(f), "--title", shQuote(title), "--session", session_id),
            wait = !in_herdr())
    invisible(f)
  }

  env$View <- function(x, title) {
    if (missing(title)) title <- paste(deparse(substitute(x)), collapse = " ")
    env$corral_view(x, title)
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
    httpgd::hgd(silent = TRUE)
    url <- httpgd::hgd_url()
    dir.create(session_dir, recursive = TRUE, showWarnings = FALSE)
    writeLines(url, file.path(session_dir, "plots-url"))
    writeLines(url, file.path(state_dir, "plots-url"))
    open_viewer(url)
    invisible(url)
  }

  env$corral_plots <- function() {
    if (!requireNamespace("httpgd", quietly = TRUE)) stop("install httpgd first", call. = FALSE)
    if (!"httpgd" %in% names(grDevices::dev.list())) return(start_httpgd())
    url <- httpgd::hgd_url()
    open_viewer(url, force = TRUE)
    invisible(url)
  }

  # grDevices only sets `device` when it is still unset, so this survives startup.
  if (requireNamespace("httpgd", quietly = TRUE) && !isFALSE(getOption("corral.httpgd"))) {
    options(device = start_httpgd)
  }

  # Attach right above utils so our View() masks utils::View(). During
  # .Rprofile only base is attached, so wait for utils when needed.
  attach_corral <- function(...) {
    if ("corral" %in% search()) detach("corral", character.only = TRUE)
    attach(env, name = "corral", warn.conflicts = FALSE)
  }
  if ("package:utils" %in% search()) attach_corral()
  else setHook(packageEvent("utils", "attach"), attach_corral)

  invisible()
})
